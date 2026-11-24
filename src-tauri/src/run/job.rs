//! One build-and-open run, step by step. Everything outside the process
//! goes through `Host`, `RunEvents` and `Clock`, so the tests drive whole
//! jobs on fakes with a paused clock.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::{RunJob, RunLog, RunPlatform, RunSourceKind, RunState, StepStatus};

use super::devices;
use super::env::BuildEnv;
use super::host::{CancelSignal, Captured, Cmd, Exit, Host, LineSink, cancelled};
use super::log::{BATCH_INTERVAL, JobLog};
use super::recipes::{self, AndroidRecipe, IosRecipe, Recipe, RecipeKind, Script};
use super::source::{Clock, RunEvents};

pub const KEY_CHECKOUT: &str = "checkout";
pub const KEY_BUILD_SDK: &str = "buildSdk";
pub const KEY_BUILD_APP: &str = "buildApp";
pub const KEY_DEVICE: &str = "device";
pub const KEY_INSTALL: &str = "install";
pub const KEY_OPEN: &str = "open";

/// How long a started emulator gets to finish booting.
pub const BOOT_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const BOOT_POLL: Duration = Duration::from_secs(2);
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
/// Emulator console ports `emulator -port` accepts: even numbers.
const EMULATOR_PORTS: std::ops::RangeInclusive<u16> = 5554..=5682;

/// Where the code comes from, decided and checked before the job starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedSource {
    /// The app's own checkout of `clone`, at the PR's head.
    PrHead { clone: PathBuf, checkout: PathBuf },
    /// One of the user's worktrees, built as it is.
    Worktree { path: PathBuf },
}

impl PlannedSource {
    pub fn kind(&self) -> RunSourceKind {
        match self {
            Self::PrHead { .. } => RunSourceKind::PrHead,
            Self::Worktree { .. } => RunSourceKind::Worktree,
        }
    }
}

/// The device, as the job addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceChoice {
    Simulator {
        udid: String,
    },
    /// An Android virtual device; `serial` when it is already running.
    Avd {
        name: String,
        serial: Option<String>,
    },
    Physical {
        serial: String,
    },
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub pr_number: u64,
    pub platform: RunPlatform,
    pub source: PlannedSource,
    pub device: DeviceChoice,
    /// Xcode's derived data for this source.
    pub derived_dir: PathBuf,
    /// Where a started emulator's own output goes.
    pub emulator_log: PathBuf,
    /// The commit being built when known up front (a worktree's HEAD).
    pub known_sha: Option<String>,
    pub env: BuildEnv,
}

/// Every step of a job, in order, as (key, label).
pub fn steps_for(
    platform: RunPlatform,
    source: RunSourceKind,
    device: &DeviceChoice,
) -> Vec<(&'static str, &'static str)> {
    let mut steps = Vec::new();
    if source == RunSourceKind::PrHead {
        steps.push((KEY_CHECKOUT, "Get the code"));
    }
    steps.push((KEY_BUILD_SDK, "Build the SDK"));
    match platform {
        RunPlatform::Ios => {
            steps.push((KEY_BUILD_APP, "Build the app"));
            steps.push((KEY_DEVICE, "Start the simulator"));
            steps.push((KEY_INSTALL, "Install the app"));
        }
        RunPlatform::Android => {
            let device_label = match device {
                DeviceChoice::Physical { .. } => "Check the device",
                _ => "Start the emulator",
            };
            steps.push((KEY_DEVICE, device_label));
            // Gradle builds and installs in one go.
            steps.push((KEY_INSTALL, "Build and install the app"));
        }
    }
    steps.push((KEY_OPEN, "Open the app"));
    steps
}

/// Why a job stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Failed(String),
    Cancelled,
}

/// What a job shares with the runner while it runs.
pub struct Shared {
    pub job: Option<RunJob>,
    pub log: JobLog,
    pub log_path: Option<PathBuf>,
}

pub struct Ctx<H, E, C> {
    pub host: Arc<H>,
    pub events: Arc<E>,
    pub clock: Arc<C>,
    pub shared: Arc<Mutex<Shared>>,
    pub cancel: CancelSignal,
    pub job_id: u64,
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|p| p.into_inner())
}

impl<H: Host, E: RunEvents, C: Clock> Ctx<H, E, C> {
    /// Changes the job and tells the UI.
    fn update(&self, f: impl FnOnce(&mut RunJob)) {
        let snapshot = {
            let mut s = lock(&self.shared);
            match s.job.as_mut() {
                Some(job) if job.job_id == self.job_id => {
                    f(job);
                    Some(job.clone())
                }
                _ => None,
            }
        };
        if let Some(job) = snapshot {
            self.events.job(&job);
        }
    }

    pub fn say(&self, line: &str) {
        lock(&self.shared).log.push(line);
    }

    fn sink(&self) -> LineSink {
        let shared = Arc::clone(&self.shared);
        LineSink::new(move |line| lock(&shared).log.push(&line))
    }

    fn label(&self, key: &str) -> String {
        lock(&self.shared)
            .job
            .as_ref()
            .and_then(|j| j.steps.iter().find(|s| s.key == key))
            .map(|s| s.label.clone())
            .unwrap_or_else(|| key.to_owned())
    }

    fn begin(&self, key: &str) -> Result<String, Stop> {
        if *self.cancel.borrow() {
            return Err(Stop::Cancelled);
        }
        let label = self.label(key);
        self.say("");
        self.say(&format!("==> {label}"));
        let now = self.clock.now();
        self.update(|job| {
            if let Some(step) = job.steps.iter_mut().find(|s| s.key == key) {
                step.status = StepStatus::Running;
                step.started_at = Some(now);
            }
        });
        Ok(label)
    }

    fn done(&self, key: &str) {
        let now = self.clock.now();
        self.update(|job| {
            if let Some(step) = job.steps.iter_mut().find(|s| s.key == key) {
                step.status = StepStatus::Done;
                step.finished_at = Some(now);
            }
        });
    }

    fn set_sha(&self, sha: &str) {
        let sha = sha.to_owned();
        self.update(|job| job.sha = Some(sha));
    }

    /// Runs one command of the step `label`, streaming its output.
    async fn run(&self, label: &str, cmd: Cmd) -> Result<(), Stop> {
        self.announce(&cmd);
        match self.host.run(cmd, self.sink(), self.cancel.clone()).await {
            Exit::Code(0) => Ok(()),
            Exit::Code(code) => Err(Stop::Failed(format!("{label} failed (exit code {code})."))),
            Exit::Signal(sig) => Err(Stop::Failed(format!(
                "{label} failed (stopped by signal {sig})."
            ))),
            Exit::Cancelled => Err(Stop::Cancelled),
            Exit::NotStarted(why) => {
                self.say(&why);
                Err(Stop::Failed(why))
            }
        }
    }

    fn announce(&self, cmd: &Cmd) {
        match &cmd.cwd {
            Some(dir) => self.say(&format!("$ {cmd}    (in {})", dir.display())),
            None => self.say(&format!("$ {cmd}")),
        }
    }

    /// A short query; its output is not logged unless `log` is set.
    async fn query(&self, cmd: Cmd, log: bool) -> Result<Option<Captured>, Stop> {
        if log {
            self.announce(&cmd);
        }
        let mut cancel = self.cancel.clone();
        tokio::select! {
            out = self.host.output(cmd, QUERY_TIMEOUT) => {
                let out = out.ok();
                if log && let Some(o) = &out {
                    for line in o.stdout.lines().chain(o.stderr.lines()) {
                        self.say(line);
                    }
                }
                Ok(out)
            }
            () = cancelled(&mut cancel) => Err(Stop::Cancelled),
        }
    }

    async fn sleep(&self, d: Duration) -> Result<(), Stop> {
        let mut cancel = self.cancel.clone();
        tokio::select! {
            () = tokio::time::sleep(d) => Ok(()),
            () = cancelled(&mut cancel) => Err(Stop::Cancelled),
        }
    }

    /// Sends the lines gathered since the last batch to the UI.
    pub fn flush_log(&self) {
        let lines = lock(&self.shared).log.take_pending();
        if !lines.is_empty() {
            self.events.log(&RunLog {
                job_id: self.job_id,
                lines,
            });
        }
    }

    /// Marks the job finished and tells the UI, after the last log lines.
    pub fn finish(&self, result: Result<(), Stop>) {
        let (state, error) = match &result {
            Ok(()) => (RunState::Succeeded, None),
            Err(Stop::Cancelled) => (RunState::Cancelled, None),
            Err(Stop::Failed(msg)) => (RunState::Failed, Some(msg.clone())),
        };
        self.say("");
        self.say(match state {
            RunState::Succeeded => "==> Done.",
            RunState::Cancelled => "==> Stopped.",
            _ => "==> Failed.",
        });
        if let Some(msg) = &error {
            self.say(msg);
        }
        self.flush_log();
        lock(&self.shared).log.close();
        let now = self.clock.now();
        self.update(|job| {
            for step in &mut job.steps {
                if step.status == StepStatus::Running {
                    // A cancelled step did not fail; it just did not finish.
                    step.status = if state == RunState::Failed {
                        StepStatus::Failed
                    } else {
                        StepStatus::Pending
                    };
                    step.finished_at = Some(now);
                }
            }
            job.state = state;
            job.error = error;
            job.finished_at = Some(now);
        });
    }
}

/// Sends batches every [`BATCH_INTERVAL`] until aborted.
pub async fn pump_log<H: Host, E: RunEvents, C: Clock>(ctx: Arc<Ctx<H, E, C>>) {
    let mut tick = tokio::time::interval(BATCH_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        ctx.flush_log();
    }
}

/// Runs every step of the job.
pub async fn execute<H: Host, E: RunEvents, C: Clock>(
    ctx: &Ctx<H, E, C>,
    plan: &Plan,
) -> Result<(), Stop> {
    let root = match &plan.source {
        PlannedSource::PrHead { clone, checkout } => {
            get_code(ctx, plan, clone, checkout).await?;
            checkout.clone()
        }
        PlannedSource::Worktree { path } => {
            ctx.say(&format!(
                "Building the worktree at {} as it is.",
                path.display()
            ));
            path.clone()
        }
    };
    let recipe = recipes::find(&root, plan.platform);
    match (recipe.map(|r| r.kind), &plan.device) {
        (Some(RecipeKind::Ios(r)), DeviceChoice::Simulator { udid }) => {
            run_ios(ctx, plan, &root, &r, udid).await
        }
        (Some(RecipeKind::Android(r)), device) if plan.platform == RunPlatform::Android => {
            run_android(ctx, plan, &root, &r, device).await
        }
        (None, _) => {
            ctx.begin(KEY_BUILD_SDK)?;
            Err(Stop::Failed(no_app(plan.platform, &plan.source)))
        }
        _ => Err(Stop::Failed("That device cannot run this app.".into())),
    }
}

pub fn no_app(platform: RunPlatform, source: &PlannedSource) -> String {
    let what = match source {
        PlannedSource::PrHead { .. } => "This commit",
        PlannedSource::Worktree { .. } => "This worktree",
    };
    format!(
        "{what} has no {} example app.",
        recipes::platform_name(platform)
    )
}

fn git(env: &BuildEnv, dir: &Path) -> Cmd {
    Cmd::new("git")
        .env(env.vars.clone())
        .var("GIT_TERMINAL_PROMPT", "0")
        .arg("-C")
        .arg(dir.to_string_lossy().into_owned())
}

/// Puts the PR's head into the app's own checkout. The only git writes the
/// run feature makes; none of them touch the user's working trees.
async fn get_code<H: Host, E: RunEvents, C: Clock>(
    ctx: &Ctx<H, E, C>,
    plan: &Plan,
    clone: &Path,
    checkout: &Path,
) -> Result<(), Stop> {
    let label = ctx.begin(KEY_CHECKOUT)?;
    let env = &plan.env;

    let usable = checkout.join(".git").is_file()
        && ctx
            .query(
                git(env, checkout).args(["rev-parse", "--show-toplevel"]),
                false,
            )
            .await?
            .filter(Captured::success)
            .is_some_and(|o| {
                super::git::canonical(Path::new(o.stdout.trim())) == super::git::canonical(checkout)
            });
    if !usable {
        let mut add = git(env, clone).args(["worktree", "add", "--detach"]);
        if checkout.exists() {
            ctx.say(&format!(
                "The app's checkout at {} is not usable; making it again.",
                checkout.display()
            ));
            if let Err(e) = std::fs::remove_dir_all(checkout) {
                return Err(Stop::Failed(format!(
                    "Could not remove the old checkout at {}: {e}.",
                    checkout.display()
                )));
            }
            // It may still be registered with the clone.
            add = add.arg("--force");
        }
        if let Some(parent) = checkout.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return Err(Stop::Failed(format!(
                "Could not create {}: {e}.",
                parent.display()
            )));
        }
        let add = add.arg(checkout.to_string_lossy().into_owned());
        ctx.run(&label, add).await?;
    }
    let refspec = format!("pull/{}/head", plan.pr_number);
    let fetch = git(env, checkout)
        // Never start maintenance in the user's repository.
        .args(["-c", "gc.auto=0", "-c", "maintenance.auto=false"])
        .args(["fetch", "--no-tags", "origin", &refspec]);
    ctx.run(&label, fetch).await?;
    // A checkout rewrites git's index even when nothing changes, and some
    // build scripts watch the index: skip it when the tree is already at the
    // fetched commit with no changes to tracked files, so the next build
    // stays incremental.
    let rev = |what: &'static str| {
        let cmd = git(env, checkout).args(["rev-parse", what]);
        async move {
            ctx.query(cmd, false).await.map(|o| {
                o.filter(Captured::success)
                    .map(|o| o.stdout.trim().to_owned())
                    .filter(|s| !s.is_empty())
            })
        }
    };
    let head = if usable { rev("HEAD").await? } else { None };
    let fetched = rev("FETCH_HEAD").await?;
    let unchanged = match (&head, &fetched) {
        (Some(h), Some(f)) if h == f => {
            let status = git(env, checkout).var("GIT_OPTIONAL_LOCKS", "0").args([
                "status",
                "--porcelain",
                "--untracked-files=no",
            ]);
            ctx.query(status, false)
                .await?
                .is_some_and(|o| o.success() && o.stdout.trim().is_empty())
        }
        _ => false,
    };
    if unchanged {
        ctx.say("The checkout is already at that commit.");
    } else {
        ctx.run(
            &label,
            git(env, checkout).args(["checkout", "--detach", "--force", "FETCH_HEAD"]),
        )
        .await?;
    }
    if let Some(out) = ctx
        .query(git(env, checkout).args(["rev-parse", "HEAD"]), false)
        .await?
        .filter(Captured::success)
    {
        let sha = out.stdout.trim().to_owned();
        ctx.say(&format!("At {sha}."));
        ctx.set_sha(&sha);
    }
    if checkout.join(".gitmodules").exists() {
        ctx.run(
            &label,
            git(env, checkout).args(["submodule", "update", "--init", "--recursive"]),
        )
        .await?;
    }
    if recipes::find(checkout, plan.platform).is_none() {
        return Err(Stop::Failed(no_app(plan.platform, &plan.source)));
    }
    ctx.done(KEY_CHECKOUT);
    Ok(())
}

fn script_cmd(env: &BuildEnv, root: &Path, script: &Script, abi: Option<&str>) -> Cmd {
    let mut cmd = Cmd::new(script.program)
        .cwd(root.join(script.dir))
        .env(env.vars.clone());
    for arg in script.args {
        cmd = cmd.arg(match abi {
            Some(abi) => arg.replace("{abi}", abi),
            None => (*arg).to_owned(),
        });
    }
    for (k, v) in script.vars {
        cmd = cmd.var(*k, *v);
    }
    cmd
}

fn xcrun(env: &BuildEnv) -> Cmd {
    Cmd::new("/usr/bin/xcrun").env(env.vars.clone())
}

async fn run_ios<H: Host, E: RunEvents, C: Clock>(
    ctx: &Ctx<H, E, C>,
    plan: &Plan,
    root: &Path,
    recipe: &IosRecipe,
    udid: &str,
) -> Result<(), Stop> {
    let env = &plan.env;

    let label = ctx.begin(KEY_BUILD_SDK)?;
    ctx.run(&label, script_cmd(env, root, &recipe.sdk, None))
        .await?;
    ctx.done(KEY_BUILD_SDK);

    let label = ctx.begin(KEY_BUILD_APP)?;
    let derived = plan.derived_dir.to_string_lossy().into_owned();
    let build = Cmd::new("/usr/bin/xcrun")
        .env(env.vars.clone())
        .cwd(root.join(recipe.xcode_dir))
        .args([
            "xcodebuild",
            "-project",
            recipe.project,
            "-scheme",
            recipe.scheme,
        ])
        .args(["-sdk", "iphonesimulator", "-configuration", "Debug"])
        .arg("-destination")
        .arg(format!("platform=iOS Simulator,id={udid}"))
        .args(["-derivedDataPath", &derived, "build"]);
    ctx.run(&label, build).await?;
    ctx.done(KEY_BUILD_APP);

    let label = ctx.begin(KEY_DEVICE)?;
    let booted = devices::simulator_booted(ctx.host.as_ref(), env, udid).await;
    if booted == Some(true) {
        ctx.say("The simulator is already running.");
    } else {
        ctx.run(&label, xcrun(env).args(["simctl", "boot", udid]))
            .await?;
    }
    // Shows the simulator's window. Not worth failing the job over.
    let open = Cmd::new("/usr/bin/open")
        .env(env.vars.clone())
        .args(["-a", "Simulator"]);
    if let Err(Stop::Cancelled) = ctx.run(&label, open).await {
        return Err(Stop::Cancelled);
    }
    ctx.run(&label, xcrun(env).args(["simctl", "bootstatus", udid]))
        .await?;
    ctx.done(KEY_DEVICE);

    let label = ctx.begin(KEY_INSTALL)?;
    let products = plan
        .derived_dir
        .join("Build/Products/Debug-iphonesimulator");
    let Some(app) = find_app_bundle(&products, recipe.scheme) else {
        return Err(Stop::Failed(format!(
            "The built app was not found in {}.",
            products.display()
        )));
    };
    let app_path = app.to_string_lossy().into_owned();
    ctx.run(
        &label,
        xcrun(env).args(["simctl", "install", udid, &app_path]),
    )
    .await?;
    ctx.done(KEY_INSTALL);

    let label = ctx.begin(KEY_OPEN)?;
    let plist = app.join("Info.plist").to_string_lossy().into_owned();
    let read_id = Cmd::new("/usr/bin/plutil").env(env.vars.clone()).args([
        "-extract",
        "CFBundleIdentifier",
        "raw",
        "-o",
        "-",
        &plist,
    ]);
    let bundle_id = ctx
        .query(read_id, false)
        .await?
        .filter(Captured::success)
        .map(|o| o.stdout.trim().to_owned())
        .filter(|s| !s.is_empty());
    let Some(bundle_id) = bundle_id else {
        return Err(Stop::Failed(
            "Could not read the app's bundle identifier.".into(),
        ));
    };
    ctx.say(&format!("Bundle identifier: {bundle_id}"));
    ctx.run(
        &label,
        xcrun(env).args([
            "simctl",
            "launch",
            "--terminate-running-process",
            udid,
            &bundle_id,
        ]),
    )
    .await?;
    ctx.done(KEY_OPEN);
    Ok(())
}

/// The `.app` in Xcode's products folder: the scheme's if there is one,
/// otherwise the first by name.
pub fn find_app_bundle(products: &Path, scheme: &str) -> Option<PathBuf> {
    let mut apps: Vec<PathBuf> = std::fs::read_dir(products)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "app") && p.join("Info.plist").is_file())
        .collect();
    apps.sort();
    let preferred = products.join(format!("{scheme}.app"));
    if apps.contains(&preferred) {
        return Some(preferred);
    }
    apps.into_iter().next()
}

fn adb(env: &BuildEnv) -> Result<Cmd, Stop> {
    env.adb()
        .map(|p| Cmd::new(p.to_string_lossy().into_owned()).env(env.vars.clone()))
        .ok_or_else(|| Stop::Failed(devices::MSG_NO_ANDROID_SDK.into()))
}

async fn run_android<H: Host, E: RunEvents, C: Clock>(
    ctx: &Ctx<H, E, C>,
    plan: &Plan,
    root: &Path,
    recipe: &AndroidRecipe,
    device: &DeviceChoice,
) -> Result<(), Stop> {
    let env = &plan.env;

    let label = ctx.begin(KEY_BUILD_SDK)?;
    let abi = match device {
        DeviceChoice::Physical { serial }
        | DeviceChoice::Avd {
            serial: Some(serial),
            ..
        } => {
            let q = adb(env)?.args(["-s", serial, "shell", "getprop", "ro.product.cpu.abi"]);
            ctx.query(q, false)
                .await?
                .filter(Captured::success)
                .and_then(|o| devices::parse_abi(&o.stdout))
        }
        DeviceChoice::Avd { name, serial: None } => {
            devices::avd_home(env).and_then(|h| devices::avd_info(&h, name).abi)
        }
        DeviceChoice::Simulator { .. } => None,
    };
    let Some(abi) = abi else {
        return Err(Stop::Failed(
            "Could not tell which CPU the device has.".into(),
        ));
    };
    let Some((_, script_abi)) = recipe.abi_names.iter().find(|(a, _)| *a == abi) else {
        return Err(Stop::Failed(format!(
            "The SDK cannot be built for this device's CPU ({abi})."
        )));
    };
    ctx.say(&format!("The device's CPU is {abi}."));
    ctx.run(&label, script_cmd(env, root, &recipe.sdk, Some(script_abi)))
        .await?;
    ctx.done(KEY_BUILD_SDK);

    ctx.begin(KEY_DEVICE)?;
    let serial = match device {
        DeviceChoice::Physical { serial } => {
            let q = adb(env)?.args(["-s", serial, "get-state"]);
            let state = ctx
                .query(q, true)
                .await?
                .map(|o| o.stdout.trim().to_owned());
            if state.as_deref() != Some("device") {
                return Err(Stop::Failed(
                    "The device is not connected, or it has not allowed USB debugging.".into(),
                ));
            }
            serial.clone()
        }
        DeviceChoice::Avd {
            serial: Some(serial),
            ..
        } => {
            ctx.say("The emulator is already running.");
            wait_for_boot(ctx, env, serial).await?;
            serial.clone()
        }
        DeviceChoice::Avd { name, serial: None } => {
            let serial = start_emulator(ctx, plan, name).await?;
            wait_for_boot(ctx, env, &serial).await?;
            serial
        }
        DeviceChoice::Simulator { .. } => {
            return Err(Stop::Failed("That device cannot run this app.".into()));
        }
    };
    ctx.done(KEY_DEVICE);

    let label = ctx.begin(KEY_INSTALL)?;
    let build_file = root.join(recipe.app_build_file);
    let app_id = std::fs::read_to_string(&build_file)
        .ok()
        .and_then(|t| recipes::application_id(&t));
    let Some(app_id) = app_id else {
        return Err(Stop::Failed(format!(
            "Could not find the app's applicationId in {}.",
            build_file.display()
        )));
    };
    let gradle = Cmd::new("./gradlew")
        .cwd(root.join(recipe.gradle_dir))
        .env(env.vars.clone())
        .var("ANDROID_SERIAL", serial.clone())
        // No daemon: a cancel then stops the whole build, and nothing is
        // left running afterwards.
        .args(["--no-daemon", "--console=plain"])
        .args(recipe.gradle_tasks.iter().copied());
    ctx.run(&label, gradle).await?;
    ctx.done(KEY_INSTALL);

    let label = ctx.begin(KEY_OPEN)?;
    // `monkey` refuses to run on emulators without hardware keys, so the
    // launcher activity is looked up and started directly.
    let resolve = adb(env)?.args([
        "-s",
        &serial,
        "shell",
        "cmd",
        "package",
        "resolve-activity",
        "--brief",
        "-c",
        "android.intent.category.LAUNCHER",
        &app_id,
    ]);
    let component = ctx
        .query(resolve, true)
        .await?
        .filter(Captured::success)
        .and_then(|o| launcher_component(&o.stdout, &app_id));
    let Some(component) = component else {
        return Err(Stop::Failed(format!(
            "{label} failed: {app_id} has no launcher activity on the device."
        )));
    };
    let start = adb(env)?.args(["-s", &serial, "shell", "am", "start", "-n", &component]);
    let out = ctx.query(start, true).await?;
    let ok = out
        .as_ref()
        .is_some_and(|o| o.success() && !o.stdout.contains("Error") && !o.stderr.contains("Error"));
    if !ok {
        return Err(Stop::Failed(format!(
            "{label} failed: the app {app_id} could not be started."
        )));
    }
    ctx.done(KEY_OPEN);
    Ok(())
}

/// The `package/.Activity` line of `cmd package resolve-activity --brief`,
/// when it belongs to `app_id`.
pub fn launcher_component(text: &str, app_id: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .filter(|l| !l.contains(char::is_whitespace))
        .filter(|l| {
            l.split_once('/')
                .is_some_and(|(pkg, act)| pkg == app_id && !act.is_empty())
        })
        .map(str::to_owned)
}

/// Starts the AVD detached, on a free console port, and returns its serial.
async fn start_emulator<H: Host, E: RunEvents, C: Clock>(
    ctx: &Ctx<H, E, C>,
    plan: &Plan,
    name: &str,
) -> Result<String, Stop> {
    let env = &plan.env;
    let Some(emulator) = env.emulator() else {
        return Err(Stop::Failed(
            "The Android emulator is not installed in the Android SDK.".into(),
        ));
    };
    let listed = ctx
        .query(adb(env)?.args(["devices", "-l"]), false)
        .await?
        .map(|o| devices::parse_adb_devices(&o.stdout))
        .unwrap_or_default();
    let used: Vec<&str> = listed.iter().map(|d| d.serial.as_str()).collect();
    let Some(port) = EMULATOR_PORTS
        .step_by(2)
        .find(|p| !used.contains(&format!("emulator-{p}").as_str()))
    else {
        return Err(Stop::Failed("Too many emulators are running.".into()));
    };
    let cmd = Cmd::new(emulator.to_string_lossy().into_owned())
        .env(env.vars.clone())
        .args(["-avd", name, "-port", &port.to_string()]);
    ctx.announce(&cmd);
    ctx.say(&format!(
        "The emulator keeps running after this job; its own output goes to {}.",
        plan.emulator_log.display()
    ));
    if let Err(e) = ctx
        .host
        .spawn_detached(cmd, Some(plan.emulator_log.clone()))
    {
        return Err(Stop::Failed(e.to_string()));
    }
    Ok(format!("emulator-{port}"))
}

async fn wait_for_boot<H: Host, E: RunEvents, C: Clock>(
    ctx: &Ctx<H, E, C>,
    env: &BuildEnv,
    serial: &str,
) -> Result<(), Stop> {
    ctx.say(&format!("Waiting for {serial} to finish starting..."));
    let deadline = tokio::time::Instant::now() + BOOT_TIMEOUT;
    loop {
        let q = adb(env)?.args(["-s", serial, "shell", "getprop", "sys.boot_completed"]);
        let booted = ctx
            .query(q, false)
            .await?
            .is_some_and(|o| o.success() && o.stdout.trim() == "1");
        if booted {
            ctx.say("It is ready.");
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Stop::Failed(format!(
                "The emulator did not finish starting within {} minutes.",
                BOOT_TIMEOUT.as_secs() / 60
            )));
        }
        ctx.sleep(BOOT_POLL).await?;
    }
}

/// The recipe a platform's checks use before the job knows its tree.
pub fn recipe_hint(root: &Path, platform: RunPlatform) -> Option<&'static Recipe> {
    recipes::find(root, platform)
        .or_else(|| recipes::RECIPES.iter().find(|r| r.platform() == platform))
}
