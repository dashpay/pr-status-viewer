//! Drives the same example-app runner the app uses, outside Tauri, and
//! prints its job and log events to stdout. It starts real builds,
//! simulators and emulators: pick the device with care.
//!
//!     cargo run --example run_smoke -- dashpay/platform#6002 --list
//!     cargo run --example run_smoke -- dashpay/platform#6002 \
//!         --platform ios --source prHead --device <UDID>
//!     cargo run --example run_smoke -- dashpay/platform#6002 \
//!         --platform android --source worktree --source-path <dir> --device avd:<name>
//!
//! Options:
//!   --list                 print the run options (sources, devices) and stop
//!   --sha <sha>            the PR's head commit (default: from `gh pr view`)
//!   --branch <name>        the PR's head branch (default: from `gh pr view`)
//!   --cancel-after <secs>  cancel the job that many seconds after it starts
//!   --cache <dir>          the app cache folder (default: the app's own)
//!
//! Cancel check for one command, through the same host and build
//! environment the runner uses (own process group, TERM then KILL):
//!
//!     cargo run --example run_smoke -- --host-run <dir> <secs> <program> [args...]
//!
//! The PR comes from `gh pr view` or the arguments, not from the sync engine.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pr_status_viewer_lib::model::{
    RunJob, RunLog, RunPlatform, RunRequest, RunSourceKind, StepStatus,
};
use pr_status_viewer_lib::run::host::RealHost;
use pr_status_viewer_lib::run::source::{MachineLookups, RunEvents, SystemClock};
use pr_status_viewer_lib::run::{Deps, PrTarget, RunPaths, Runner, recipes};

const IDENTIFIER: &str = "org.dash.prstatus";

struct Stdout {
    started: Instant,
}

impl RunEvents for Stdout {
    fn job(&self, job: &RunJob) {
        let steps: Vec<String> = job
            .steps
            .iter()
            .map(|s| {
                let mark = match s.status {
                    StepStatus::Done => "done",
                    StepStatus::Running => "RUNNING",
                    StepStatus::Pending => "-",
                    StepStatus::Failed => "FAILED",
                };
                let took = match (s.started_at, s.finished_at) {
                    (Some(a), Some(b)) => format!(" {}s", (b - a).num_seconds()),
                    _ => String::new(),
                };
                format!("{}={mark}{took}", s.key)
            })
            .collect();
        println!(
            "[{:>6.1}s] job {} {:?} sha={} {}{}",
            self.started.elapsed().as_secs_f64(),
            job.job_id,
            job.state,
            job.sha.as_deref().unwrap_or("?"),
            steps.join(" "),
            job.error
                .as_deref()
                .map(|e| format!("  error: {e}"))
                .unwrap_or_default()
        );
    }

    fn log(&self, log: &RunLog) {
        for line in &log.lines {
            println!("  | {line}");
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: run_smoke <owner/name#N> (--list | --platform ios|android --source prHead|worktree \
         [--source-path DIR] --device ID) [--sha SHA] [--branch NAME] [--cancel-after SECS] [--cache DIR]"
    );
    std::process::exit(2);
}

fn gh_head(repo: &str, number: u64) -> Option<(String, String)> {
    let out = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &number.to_string(),
            "-R",
            repo,
            "--json",
            "headRefOid,headRefName",
        ])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some((
        v.get("headRefOid")?.as_str()?.to_owned(),
        v.get("headRefName")?.as_str()?.to_owned(),
    ))
}

/// Runs one command through `RealHost` in the build environment and
/// cancels it after `secs`.
async fn host_run(mut args: impl Iterator<Item = String>) {
    use pr_status_viewer_lib::run::env;
    use pr_status_viewer_lib::run::host::{Cmd, Host, LineSink};
    let (Some(dir), Some(secs), Some(program)) = (
        args.next(),
        args.next().and_then(|s| s.parse::<u64>().ok()),
        args.next(),
    ) else {
        usage()
    };
    let build_env = env::gather(&RealHost).await;
    let cmd = Cmd::new(program)
        .args(args)
        .cwd(dir)
        .env(build_env.vars.clone());
    println!("$ {cmd}");
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        println!("[cancel] after {secs}s");
        let _ = tx.send(true);
    });
    let started = Instant::now();
    let exit = RealHost
        .run(cmd, LineSink::new(|l| println!("  | {l}")), rx)
        .await;
    println!(
        "exit: {exit:?} after {:.1}s",
        started.elapsed().as_secs_f64()
    );
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(id) = args.next() else { usage() };
    if id == "--host-run" {
        host_run(args).await;
        return;
    }
    let Some((repo, number)) = id
        .split_once('#')
        .and_then(|(r, n)| Some((r.to_owned(), n.parse::<u64>().ok()?)))
    else {
        usage()
    };
    let (mut list, mut platform, mut source, mut source_path, mut device) =
        (false, None, None, None, None);
    let (mut sha, mut branch, mut cancel_after, mut cache) = (None, None, None, None);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--list" => list = true,
            "--platform" => {
                platform = Some(match val().as_str() {
                    "ios" => RunPlatform::Ios,
                    "android" => RunPlatform::Android,
                    _ => usage(),
                })
            }
            "--source" => {
                source = Some(match val().as_str() {
                    "prHead" => RunSourceKind::PrHead,
                    "worktree" => RunSourceKind::Worktree,
                    _ => usage(),
                })
            }
            "--source-path" => source_path = Some(val()),
            "--device" => device = Some(val()),
            "--sha" => sha = Some(val()),
            "--branch" => branch = Some(val()),
            "--cancel-after" => cancel_after = val().parse::<u64>().ok(),
            "--cache" => cache = Some(PathBuf::from(val())),
            _ => usage(),
        }
    }

    let (head_sha, head_branch) = match (sha, branch) {
        (Some(s), b) => (s, b),
        (None, b) => match gh_head(&repo, number) {
            Some((s, gb)) => (s, b.or(Some(gb))),
            None => {
                eprintln!("Could not ask `gh` for the PR's head; pass --sha.");
                std::process::exit(1);
            }
        },
    };
    let pr = PrTarget {
        repo,
        number,
        head_sha,
        head_branch,
    };

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("No HOME.");
        std::process::exit(1);
    };
    let (default_cache, config) = if cfg!(target_os = "macos") {
        (
            home.join("Library/Caches").join(IDENTIFIER),
            home.join("Library/Application Support").join(IDENTIFIER),
        )
    } else {
        (
            home.join(".cache").join(IDENTIFIER),
            home.join(".config").join(IDENTIFIER),
        )
    };
    let started = Instant::now();
    let runner = Runner::new(Deps {
        host: RealHost,
        events: Stdout { started },
        clock: SystemClock,
        lookups: MachineLookups::default(),
        paths: RunPaths {
            cache_dir: cache.unwrap_or(default_cache),
            settings_file: config.join("run.json"),
        },
        ios_supported: recipes::platform_supported(RunPlatform::Ios),
    });

    let t = Instant::now();
    let options = match runner.options(&pr).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("run_options failed: {e}");
            std::process::exit(1);
        }
    };
    println!("run_options took {:.2}s", t.elapsed().as_secs_f64());
    if list {
        match serde_json::to_string_pretty(&options) {
            Ok(json) => println!("{json}"),
            Err(e) => eprintln!("{e}"),
        }
        return;
    }

    let (Some(platform), Some(source_kind), Some(device_id)) = (platform, source, device) else {
        usage()
    };
    let request = RunRequest {
        id: pr.id(),
        platform,
        source_kind,
        source_path,
        device_id,
    };
    if let Err(e) = runner.start(&pr, &request).await {
        eprintln!("run_start refused: {e}");
        std::process::exit(1);
    }
    if let Some(secs) = cancel_after {
        let r = runner.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            println!("[cancel] cancelling after {secs}s");
            r.cancel();
        });
    }
    runner.wait().await;
    let job = runner.job();
    match job
        .as_ref()
        .and_then(|j| serde_json::to_string_pretty(j).ok())
    {
        Some(json) => println!("{json}"),
        None => println!("no job"),
    }
    println!("took {:.1}s", started.elapsed().as_secs_f64());
}
