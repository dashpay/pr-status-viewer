//! Building a PR's code and opening a project's example app on a simulator
//! or emulator.
//!
//! - `recipes`: how each project's example apps are built, as data.
//! - `host`: starting processes (each streamed command in its own process
//!   group, so a cancel stops the whole tree).
//! - `env`: the user's real build environment.
//! - `git`: reading the user's clone and worktrees (read-only).
//! - `devices`: simulators, emulators and devices.
//! - `settings`: `run.json`, the clones the user chose.
//! - `log`: the job's log, in memory, on disk and in batches for the UI.
//! - `job`: one run, step by step.
//!
//! Like `sync/`, the [`Runner`] owns all state and talks to the outside
//! world only through traits (`Host`, `RunEvents`, `Clock`, `Lookups`), so
//! its tests run on fakes with a paused clock.

pub mod devices;
pub mod env;
pub mod git;
pub mod host;
pub mod job;
pub mod log;
pub mod recipes;
pub mod settings;
pub mod source;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::error::{AppError, AppResult};
use crate::model::{
    RunDevice, RunDeviceKind, RunJob, RunOptions, RunPlatform, RunPlatformOptions, RunProject,
    RunRequest, RunSource, RunSourceKind, RunState, RunStep, StepStatus, pr_id,
};

use self::env::BuildEnv;
use self::host::Host;
use self::job::{Ctx, DeviceChoice, Plan, PlannedSource, Shared};
use self::log::JobLog;
use self::settings::RunSettings;
use self::source::{Clock, Lookups, RunEvents};

/// How long a resolved environment is reused for the run dialog. Each job
/// resolves its own.
const ENV_TTL: Duration = Duration::from_secs(5 * 60);
/// How long the app waits for a cancelled job to stop when it quits.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

pub const MSG_BUSY: &str = "Another build is running. Stop it or wait for it to finish first.";

/// A pull request as the runner needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrTarget {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub head_sha: String,
    pub head_branch: Option<String>,
}

impl PrTarget {
    pub fn id(&self) -> String {
        pr_id(&self.repo, self.number)
    }
}

/// Where the runner keeps its files.
#[derive(Debug, Clone)]
pub struct RunPaths {
    /// The app's cache folder: checkouts, Xcode's derived data, logs.
    pub cache_dir: PathBuf,
    /// `run.json`.
    pub settings_file: PathBuf,
}

impl RunPaths {
    fn slug(repo: &str) -> String {
        repo.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect()
    }

    /// The app's own checkout of `repo`.
    pub fn checkout(&self, repo: &str) -> PathBuf {
        self.cache_dir.join("checkouts").join(Self::slug(repo))
    }

    pub fn logs(&self) -> PathBuf {
        self.cache_dir.join("logs")
    }

    /// Xcode's derived data for building `source`.
    pub fn derived(&self, repo: &str, source: &PlannedSource) -> PathBuf {
        let base = self.cache_dir.join("xcode");
        match source {
            PlannedSource::PrHead { .. } => base.join(Self::slug(repo)),
            PlannedSource::Worktree { path } => base.join(format!(
                "{}-worktree-{:016x}",
                Self::slug(repo),
                fnv1a(path.to_string_lossy().as_bytes())
            )),
        }
    }
}

/// A stable hash for folder names (std's hasher may change between Rust
/// versions).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

pub struct Deps<H, E, C, L> {
    pub host: H,
    pub events: E,
    pub clock: C,
    pub lookups: L,
    pub paths: RunPaths,
    /// iOS can be built here (macOS only).
    pub ios_supported: bool,
}

struct State {
    next_id: u64,
    shared: Option<Arc<Mutex<Shared>>>,
    cancel: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

struct Inner<H, E, C, L> {
    host: Arc<H>,
    events: Arc<E>,
    clock: Arc<C>,
    lookups: L,
    paths: RunPaths,
    settings: RunSettings,
    ios_supported: bool,
    env_cache: Mutex<Option<(tokio::time::Instant, BuildEnv)>>,
    state: Mutex<State>,
}

/// Runs one job at a time and answers the run dialog's questions.
pub struct Runner<H, E, C, L> {
    inner: Arc<Inner<H, E, C, L>>,
}

impl<H, E, C, L> Clone for Runner<H, E, C, L> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A clone, and whether the user chose it.
type CloneChoice = Option<(PathBuf, bool)>;

impl<H: Host, E: RunEvents, C: Clock, L: Lookups> Runner<H, E, C, L> {
    pub fn new(deps: Deps<H, E, C, L>) -> Self {
        let settings = RunSettings::new(deps.paths.settings_file.clone());
        Self {
            inner: Arc::new(Inner {
                host: Arc::new(deps.host),
                events: Arc::new(deps.events),
                clock: Arc::new(deps.clock),
                lookups: deps.lookups,
                paths: deps.paths,
                settings,
                ios_supported: deps.ios_supported,
                env_cache: Mutex::new(None),
                state: Mutex::new(State {
                    next_id: 1,
                    shared: None,
                    cancel: None,
                    task: None,
                }),
            }),
        }
    }

    pub fn paths(&self) -> &RunPaths {
        &self.inner.paths
    }

    // ---- Clones ----

    /// The user's choice first, then the first T3 Code project that is a
    /// clone of `repo`.
    async fn resolve_clone(&self, repo: &str) -> CloneChoice {
        if let Some(p) = self.inner.settings.clone_for(repo) {
            return Some((p, true));
        }
        for root in self.inner.lookups.t3_workspaces().await {
            if !root.is_dir() {
                continue;
            }
            let Some(url) = git::origin_url(self.inner.host.as_ref(), &root).await else {
                continue;
            };
            if git::origin_matches(&url, repo) {
                return Some((root, false));
            }
        }
        None
    }

    /// One entry per project, in the order given.
    pub async fn targets(&self, projects: &[String]) -> Vec<RunProject> {
        let mut out = Vec::with_capacity(projects.len());
        for repo in projects {
            let clone = self.resolve_clone(repo).await;
            let platforms = clone
                .as_ref()
                .map(|(p, _)| recipes::platforms_in(p, self.inner.ios_supported))
                .unwrap_or_default();
            out.push(RunProject {
                repo: repo.clone(),
                clone_path: clone
                    .as_ref()
                    .map(|(p, _)| p.to_string_lossy().into_owned()),
                clone_is_set: clone.as_ref().is_some_and(|(_, set)| *set),
                platforms,
            });
        }
        out
    }

    /// Remembers (or forgets, with `None`) where the clone of `repo` is.
    pub async fn set_clone(
        &self,
        projects: &[String],
        repo: &str,
        path: Option<&str>,
    ) -> AppResult<Vec<RunProject>> {
        let Some(repo) = projects.iter().find(|p| p.eq_ignore_ascii_case(repo)) else {
            return Err(AppError::InvalidInput(format!(
                "{repo} is not one of your projects."
            )));
        };
        match path.map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => {
                let top = git::validate_clone(self.inner.host.as_ref(), Path::new(p), repo).await?;
                self.inner.settings.set_clone(repo, Some(&top))?;
            }
            None => self.inner.settings.set_clone(repo, None)?,
        }
        Ok(self.targets(projects).await)
    }

    // ---- The run dialog ----

    async fn env_cached(&self) -> BuildEnv {
        if let Some((at, env)) = lock(&self.inner.env_cache).as_ref()
            && at.elapsed() < ENV_TTL
        {
            return env.clone();
        }
        self.env_fresh().await
    }

    async fn env_fresh(&self) -> BuildEnv {
        let env = self.inner.lookups.build_env().await;
        *lock(&self.inner.env_cache) = Some((tokio::time::Instant::now(), env.clone()));
        env
    }

    async fn clone_for_pr(&self, repo: &str) -> AppResult<PathBuf> {
        match self.resolve_clone(repo).await {
            Some((p, _)) if p.is_dir() => Ok(p),
            Some((p, true)) => Err(AppError::InvalidInput(format!(
                "The clone of {repo} at {} is gone. Choose its folder again.",
                p.display()
            ))),
            _ => Err(AppError::InvalidInput(format!(
                "No local clone of {repo} is known. Choose its folder first."
            ))),
        }
    }

    /// The worktrees `run_options` offers for `pr`, with their state.
    async fn worktree_sources(&self, clone: &Path, pr: &PrTarget) -> AppResult<Vec<RunSource>> {
        let host = self.inner.host.as_ref();
        let all = git::list_worktrees(host, clone).await?;
        let checkout = self.inner.paths.checkout(&pr.repo);
        let matching = git::matching_worktrees(
            all,
            pr.head_branch.as_deref(),
            &pr.head_sha,
            &checkout,
            &|p| p.is_dir(),
        );
        let mut sources = Vec::with_capacity(matching.len());
        for m in matching {
            let dirty = git::is_dirty(host, &m.worktree.path).await;
            sources.push(RunSource {
                kind: RunSourceKind::Worktree,
                path: m.worktree.path.to_string_lossy().into_owned(),
                branch: m.worktree.branch.clone(),
                sha: m.worktree.head.clone().unwrap_or_default(),
                at_pr_head: m.at_pr_head,
                dirty,
            });
        }
        Ok(sources)
    }

    async fn probe(
        &self,
        env: &BuildEnv,
        platform: RunPlatform,
        root: &Path,
    ) -> (RunPlatformOptions, HashMap<String, String>) {
        let requires = job::recipe_hint(root, platform)
            .map(|r| r.requires)
            .unwrap_or(&[]);
        let host = self.inner.host.as_ref();
        match platform {
            RunPlatform::Ios if !self.inner.ios_supported => (
                RunPlatformOptions {
                    platform,
                    unavailable: Some(devices::MSG_NOT_MAC.into()),
                    devices: Vec::new(),
                },
                HashMap::new(),
            ),
            RunPlatform::Ios => (
                devices::probe_ios(host, env, requires).await,
                HashMap::new(),
            ),
            RunPlatform::Android => {
                let p = devices::probe_android(host, env, requires).await;
                (p.options, p.serials)
            }
        }
    }

    /// Sources and devices for running `pr`.
    pub async fn options(&self, pr: &PrTarget) -> AppResult<RunOptions> {
        let clone = self.clone_for_pr(&pr.repo).await?;
        let platforms = recipes::platforms_in(&clone, self.inner.ios_supported);
        let env = self.env_cached().await;
        let worktrees = self.worktree_sources(&clone, pr);
        let probes = async {
            let ios = async {
                if platforms.contains(&RunPlatform::Ios) {
                    Some(self.probe(&env, RunPlatform::Ios, &clone).await.0)
                } else {
                    None
                }
            };
            let android = async {
                if platforms.contains(&RunPlatform::Android) {
                    Some(self.probe(&env, RunPlatform::Android, &clone).await.0)
                } else {
                    None
                }
            };
            let (a, b) = tokio::join!(ios, android);
            a.into_iter().chain(b).collect::<Vec<_>>()
        };
        let (worktrees, platform_options) = tokio::join!(worktrees, probes);
        let mut sources = worktrees?;
        sources.push(RunSource {
            kind: RunSourceKind::PrHead,
            path: self
                .inner
                .paths
                .checkout(&pr.repo)
                .to_string_lossy()
                .into_owned(),
            branch: None,
            sha: pr.head_sha.clone(),
            at_pr_head: true,
            dirty: false,
        });
        Ok(RunOptions {
            id: pr.id(),
            clone_path: clone.to_string_lossy().into_owned(),
            sources,
            platforms: platform_options,
        })
    }

    // ---- Jobs ----

    fn running(&self) -> bool {
        let state = lock(&self.inner.state);
        state
            .shared
            .as_ref()
            .and_then(|s| lock(s).job.as_ref().map(|j| j.state == RunState::Running))
            .unwrap_or(false)
    }

    /// Checks the request against the machine, never trusting the UI, and
    /// starts the job. `pr` must come from the current snapshot.
    pub async fn start(&self, pr: &PrTarget, request: &RunRequest) -> AppResult<RunJob> {
        if self.running() {
            return Err(AppError::InvalidInput(MSG_BUSY.into()));
        }
        if request.id != pr.id() {
            return Err(AppError::InvalidInput(
                "The request does not match the pull request.".into(),
            ));
        }
        let platform = request.platform;
        if platform == RunPlatform::Ios && !self.inner.ios_supported {
            return Err(AppError::InvalidInput(devices::MSG_NOT_MAC.into()));
        }
        let clone = self.clone_for_pr(&pr.repo).await?;
        let checkout = self.inner.paths.checkout(&pr.repo);

        // The PR head's commit is only known once it is fetched.
        let mut known_sha = None;
        let source = match request.source_kind {
            RunSourceKind::PrHead => PlannedSource::PrHead {
                clone: clone.clone(),
                checkout,
            },
            RunSourceKind::Worktree => {
                let Some(wanted) = request.source_path.as_deref().filter(|p| !p.is_empty()) else {
                    return Err(AppError::InvalidInput("Choose a worktree to build.".into()));
                };
                let wanted = git::canonical(Path::new(wanted));
                let offered = self.worktree_sources(&clone, pr).await?;
                let Some(found) = offered
                    .into_iter()
                    .find(|s| git::canonical(Path::new(&s.path)) == wanted)
                else {
                    return Err(AppError::InvalidInput(
                        "That worktree is not on this pull request's branch.".into(),
                    ));
                };
                known_sha = Some(found.sha.clone()).filter(|s| !s.is_empty());
                let path = PathBuf::from(found.path);
                let source = PlannedSource::Worktree { path: path.clone() };
                if recipes::find(&path, platform).is_none() {
                    return Err(AppError::InvalidInput(job::no_app(platform, &source)));
                }
                source
            }
        };
        let root = match &source {
            PlannedSource::PrHead { clone, .. } => clone.clone(),
            PlannedSource::Worktree { path } => path.clone(),
        };

        // One environment for the whole job, resolved now.
        let env = self.env_fresh().await;
        let (options, serials) = self.probe(&env, platform, &root).await;
        if let Some(why) = options.unavailable {
            return Err(AppError::InvalidInput(why));
        }
        let Some(device) = options
            .devices
            .iter()
            .find(|d| d.id == request.device_id)
            .cloned()
        else {
            return Err(AppError::InvalidInput(
                "That device is not available any more. Choose another.".into(),
            ));
        };
        let choice = device_choice(platform, &device, &serials)?;

        let plan = Plan {
            pr_number: pr.number,
            platform,
            derived_dir: self.inner.paths.derived(&pr.repo, &source),
            emulator_log: self.inner.paths.logs().join("emulator.txt"),
            source,
            device: choice,
            known_sha,
            env,
        };
        self.launch(pr, &device, plan)
    }

    fn launch(&self, pr: &PrTarget, device: &RunDevice, plan: Plan) -> AppResult<RunJob> {
        let mut state = lock(&self.inner.state);
        let busy = state
            .shared
            .as_ref()
            .and_then(|s| lock(s).job.as_ref().map(|j| j.state == RunState::Running))
            .unwrap_or(false);
        if busy {
            return Err(AppError::InvalidInput(MSG_BUSY.into()));
        }
        let job_id = state.next_id;
        state.next_id += 1;
        let now = self.inner.clock.now();
        let steps = job::steps_for(plan.platform, plan.source.kind(), &plan.device)
            .into_iter()
            .map(|(key, label)| RunStep {
                key: key.to_owned(),
                label: label.to_owned(),
                status: StepStatus::Pending,
                started_at: None,
                finished_at: None,
            })
            .collect();
        let name = log::log_file_name(now, &pr.id(), plan.platform);
        let (log_path, file) = match log::create_log_file(&self.inner.paths.logs(), &name) {
            Ok((path, file)) => (Some(path), Some(file)),
            Err(e) => {
                ::log::warn!("run: could not create a log file: {e}");
                (None, None)
            }
        };
        let job = RunJob {
            job_id,
            pr_id: pr.id(),
            platform: plan.platform,
            device_name: device.name.clone(),
            source_kind: plan.source.kind(),
            sha: plan.known_sha.clone(),
            state: RunState::Running,
            steps,
            started_at: now,
            finished_at: None,
            error: None,
            log_path: log_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        };
        let shared = Arc::new(Mutex::new(Shared {
            job: Some(job.clone()),
            log: JobLog::new(file),
            log_path,
        }));
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let ctx = Arc::new(Ctx {
            host: Arc::clone(&self.inner.host),
            events: Arc::clone(&self.inner.events),
            clock: Arc::clone(&self.inner.clock),
            shared: Arc::clone(&shared),
            cancel: cancel_rx,
            job_id,
        });
        {
            let mut log = lock(&shared);
            let source = match &plan.source {
                PlannedSource::PrHead { checkout, .. } => {
                    format!("the PR head in {}", checkout.display())
                }
                PlannedSource::Worktree { path } => format!("the worktree {}", path.display()),
            };
            log.log.push(&format!(
                "Run {} on {} ({}), from {source}.",
                pr.id(),
                device.name,
                recipes::platform_name(plan.platform)
            ));
        }
        state.shared = Some(shared);
        state.cancel = Some(cancel_tx);
        self.inner.events.job(&job);
        let task = tokio::spawn(async move {
            let pump = tokio::spawn(job::pump_log(Arc::clone(&ctx)));
            let result = job::execute(&ctx, &plan).await;
            pump.abort();
            let _ = pump.await;
            ctx.finish(result);
        });
        state.task = Some(task);
        Ok(job)
    }

    /// Stops the running job, if any.
    pub fn cancel(&self) {
        let state = lock(&self.inner.state);
        if let Some(tx) = &state.cancel {
            let _ = tx.send(true);
        }
    }

    /// The running job or the last one.
    pub fn job(&self) -> Option<RunJob> {
        let state = lock(&self.inner.state);
        state.shared.as_ref().and_then(|s| lock(s).job.clone())
    }

    /// The last lines of that job's log, oldest first.
    pub fn log(&self) -> Vec<String> {
        let state = lock(&self.inner.state);
        state
            .shared
            .as_ref()
            .map(|s| lock(s).log.lines())
            .unwrap_or_default()
    }

    /// That job's complete log file.
    pub fn log_path(&self) -> Option<PathBuf> {
        let state = lock(&self.inner.state);
        state.shared.as_ref().and_then(|s| lock(s).log_path.clone())
    }

    /// Cancels a running job and waits (a while) for it to stop. For when the
    /// app quits.
    pub async fn shutdown(&self) {
        let task = {
            let mut state = lock(&self.inner.state);
            if let Some(tx) = &state.cancel {
                let _ = tx.send(true);
            }
            state.task.take()
        };
        if let Some(task) = task {
            let _ = tokio::time::timeout(SHUTDOWN_WAIT, task).await;
        }
    }

    /// Resolves when the current job (if any) has finished. For the smoke
    /// example and tests.
    pub async fn wait(&self) {
        let task = lock(&self.inner.state).task.take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

fn device_choice(
    platform: RunPlatform,
    device: &RunDevice,
    serials: &HashMap<String, String>,
) -> AppResult<DeviceChoice> {
    let bad = || AppError::InvalidInput("That device cannot run this app.".into());
    match (platform, device.kind) {
        (RunPlatform::Ios, RunDeviceKind::Simulator) => Ok(DeviceChoice::Simulator {
            udid: device.id.clone(),
        }),
        (RunPlatform::Android, RunDeviceKind::Emulator) => {
            let name = device
                .id
                .strip_prefix(devices::AVD_PREFIX)
                .ok_or_else(bad)?
                .to_owned();
            Ok(DeviceChoice::Avd {
                name,
                serial: serials.get(&device.id).cloned().filter(|_| device.running),
            })
        }
        (RunPlatform::Android, RunDeviceKind::Device) => {
            let serial = serials.get(&device.id).cloned().ok_or_else(bad)?;
            Ok(DeviceChoice::Physical { serial })
        }
        _ => Err(bad()),
    }
}
