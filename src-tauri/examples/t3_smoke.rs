//! Lists T3 Code's threads through its `t3` command, the way the app does
//! (read-only: `t3 thread list --json` only), and prints one line per
//! thread-to-PR link, plus how long the read took and what it cost.
//!
//!     cargo run --example t3_smoke                     # dashpay/platform
//!     cargo run --example t3_smoke -- owner/name ...   # these repos
//!     cargo run --example t3_smoke -- --all            # every T3 project
//!
//! `PR_STATUS_T3_CLI` and `PR_STATUS_T3_DB` point it at another command or
//! database.

use std::time::Instant;

use pr_status_viewer_lib::model::T3ThreadState;
use pr_status_viewer_lib::t3::T3Cli;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().any(|a| a == "--all");
    let reader = T3Cli::new();

    match reader.cli_path() {
        Some(p) => println!("t3 command: {}", p.display()),
        None => println!("t3 command: no home directory"),
    }
    let started = Instant::now();
    let projects = match reader.projects().await {
        Ok(Some(projects)) => projects,
        Ok(None) => {
            println!("T3 Code is not installed (no database to list projects from).");
            return;
        }
        Err(e) => {
            println!("T3 Code's projects are unavailable: {e}");
            return;
        }
    };
    println!(
        "projects ({:.0} ms, database and git origins):",
        started.elapsed().as_secs_f64() * 1000.0
    );
    for p in &projects {
        println!(
            "  {}  origin {}",
            p.root.display(),
            p.repo.as_deref().unwrap_or("-")
        );
    }

    let repos: Vec<String> = if all {
        projects.iter().filter_map(|p| p.repo.clone()).collect()
    } else if args.is_empty() {
        vec!["dashpay/platform".to_owned()]
    } else {
        args.iter()
            .filter(|a| !a.starts_with("--"))
            .cloned()
            .collect()
    };
    println!("reading the projects of: {}", repos.join(", "));

    let cpu_before = children_cpu();
    let started = Instant::now();
    let result = reader.read(&repos).await;
    let wall = started.elapsed();
    let cpu = children_cpu().zip(cpu_before).map(|(a, b)| a - b);

    match result {
        Ok(None) => println!("T3 Code is not installed: no t3 command."),
        Ok(Some(links)) => {
            for l in &links {
                let state = match l.state {
                    T3ThreadState::Working => "working",
                    T3ThreadState::Waiting => "waiting",
                    T3ThreadState::Idle => "idle",
                };
                let watching = if l.watching { "watching" } else { "-" };
                let last = l
                    .last_active_at
                    .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                    .unwrap_or_else(|| "never".into());
                let title: String = l.title.chars().take(40).collect();
                println!(
                    "{}#{}  {state}  {watching}  last active {last}  {title}",
                    l.repo, l.number
                );
            }
            println!("{} links", links.len());
        }
        Err(e) => println!("T3 Code unavailable: {e}"),
    }
    print!("read took {:.0} ms wall", wall.as_secs_f64() * 1000.0);
    match cpu {
        Some(cpu) => println!(", {cpu:.2} s CPU in the t3 processes"),
        None => println!(),
    }
}

/// User plus system CPU seconds of every reaped child so far.
#[cfg(unix)]
fn children_cpu() -> Option<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes one `rusage` into the pointer it is given.
    let ok = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) } == 0;
    if !ok {
        return None;
    }
    // SAFETY: initialised by the successful call above.
    let usage = unsafe { usage.assume_init() };
    let secs = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    Some(secs(usage.ru_utime) + secs(usage.ru_stime))
}

#[cfg(not(unix))]
fn children_cpu() -> Option<f64> {
    None
}
