//! Tries tracked contributors on real data, read-only, with a config in a
//! temp dir (the app's own config is never touched).
//!
//! ```text
//! cargo run --example contributors_smoke -- [login] [owner/name ...]
//! ```
//!
//! First lists the authors of open PRs across the projects (default: the
//! four dashpay projects) straight from the client, with the rate-limit
//! points it took. Then starts a sync engine on a temp config with those
//! projects, prints `known_contributors`, tries a few inputs that must be
//! refused, tracks `login` (default: the first known contributor), waits for
//! the sync it asks for and prints the PRs the snapshot gained. Nothing is
//! written to GitHub.

use std::sync::Arc;
use std::time::Duration;

use pr_status_viewer_lib::config::ConfigStore;
use pr_status_viewer_lib::error::{AppError, AppResult};
use pr_status_viewer_lib::feed::FeedClient;
use pr_status_viewer_lib::github::{GitHubClient, resolve_token};
use pr_status_viewer_lib::model::{Config, Snapshot, SyncState};
use pr_status_viewer_lib::sync::source::{ConfigRules, EventSink, T3Source, TokenConnector};
use pr_status_viewer_lib::sync::{Deps, Engine};
use pr_status_viewer_lib::t3::T3Link;

const DEFAULT_PROJECTS: [&str; 4] = [
    "dashpay/platform",
    "dashpay/dips",
    "dashpay/dash-evo-tool",
    "dashpay/dash",
];

struct Quiet;

impl EventSink for Quiet {
    fn snapshot(&self, _: &Snapshot) {}
    fn sync_state(&self, _: SyncState) {}
}

struct NoT3;

impl T3Source for NoT3 {
    async fn read(&self, _repos: &[String]) -> AppResult<Option<Vec<T3Link>>> {
        Ok(None)
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let login = args.iter().find(|a| !a.contains('/')).cloned();
    let mut projects: Vec<String> = args.iter().filter(|a| a.contains('/')).cloned().collect();
    if projects.is_empty() {
        projects = DEFAULT_PROJECTS.iter().map(|s| (*s).to_owned()).collect();
    }

    // 1. What listing the authors costs.
    let client = GitHubClient::new(resolve_token().await?)?;
    let before = client.usage();
    let lists = list_authors(&client, &projects).await;
    let used = client.usage();
    for (repo, n) in &lists {
        println!("{repo}: {n} open PRs listed");
    }
    println!(
        "authors of every project: {} requests, {} bytes, {} points",
        used.requests - before.requests,
        used.bytes - before.bytes,
        used.cost - before.cost
    );

    // 2. The engine, on a temp config.
    let dir = tempfile::tempdir().map_err(|e| AppError::Other(e.to_string()))?;
    let store = ConfigStore::new(dir.path().join("config.json"));
    store.save(&Config {
        projects: projects.clone(),
        ..Config::default()
    })?;
    let engine = Arc::new(Engine::new(Deps {
        github: TokenConnector,
        feed: FeedClient::new()?,
        t3: NoT3,
        events: Quiet,
        store,
        rules: ConfigRules::REAL,
        merge: pr_status_viewer_lib::merge::build_snapshot,
    }));
    tokio::spawn(Arc::clone(&engine).run());
    let before = wait_for_sync(&engine, None).await;
    let viewer = before
        .viewer
        .as_ref()
        .map(|v| v.login.clone())
        .unwrap_or_default();

    let known = engine.known_contributors().await?;
    println!("\nknown contributors ({}):", known.len());
    for k in &known {
        println!(
            "  {:<24} {:>3} open PRs  avatar {}",
            k.login,
            k.open_prs,
            if k.avatar_url.is_some() { "yes" } else { "no" }
        );
    }

    println!("\nrefusals:");
    for input in [
        viewer.as_str(),
        "dashpay",
        "zz-no-such-user-xq7",
        "thepastaclaw",
        "coderabbitai[bot]",
        "not a login",
    ] {
        match engine.track_contributor(input).await {
            Ok(c) => println!("  {input:<22} -> accepted?! {:?}", c.contributors),
            Err(e) => println!("  {input:<22} -> {e}"),
        }
    }

    let Some(login) = login.or_else(|| known.first().map(|k| k.login.clone())) else {
        println!("nobody to track");
        return Ok(());
    };
    let typed = format!("https://github.com/{}/", login.to_ascii_lowercase());
    let config = engine.track_contributor(&typed).await?;
    println!(
        "\ntracked {typed} -> contributors {:?}",
        config.contributors
    );
    let again = engine
        .track_contributor(&login.to_ascii_uppercase())
        .await?;
    println!("tracked again -> contributors {:?}", again.contributors);

    let after = wait_for_sync(&engine, Some(before.generated_at)).await;
    let count = |s: &Snapshot| -> usize { s.projects.iter().map(|p| p.prs.len()).sum() };
    println!(
        "\nsnapshot PRs: {} before, {} after tracking {login}",
        count(&before),
        count(&after)
    );
    for p in &after.projects {
        let theirs: Vec<String> = p
            .prs
            .iter()
            .filter(|pr| pr.author.eq_ignore_ascii_case(&login))
            .map(|pr| {
                format!(
                    "#{}{}{}",
                    pr.number,
                    if pr.is_mine || pr.is_followed {
                        " (!flags)"
                    } else {
                        ""
                    },
                    pr.needs_me_reason
                        .as_deref()
                        .map(|r| format!(" [needs me: {r}]"))
                        .unwrap_or_default()
                )
            })
            .collect();
        let error = p.error.as_deref().unwrap_or("");
        println!(
            "  {}: {} of theirs {theirs:?} {error}",
            p.repo,
            theirs.len()
        );
    }

    let config = engine.untrack_contributor(&login)?;
    let gone = engine.snapshot().map_or(0, |s| {
        s.projects
            .iter()
            .flat_map(|p| &p.prs)
            .filter(|pr| pr.author.eq_ignore_ascii_case(&login))
            .count()
    });
    println!(
        "\nuntracked -> contributors {:?}; their PRs left in the snapshot: {gone}",
        config.contributors
    );
    Ok(())
}

/// Lists every project's open PR authors one after the other.
async fn list_authors(client: &GitHubClient, projects: &[String]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for repo in projects {
        match client.open_pr_authors(repo).await {
            Ok(a) => out.push((repo.clone(), a.len())),
            Err(e) => println!("{repo}: {e}"),
        }
    }
    out
}

/// Waits for a snapshot newer than `after` from a finished GitHub pass.
async fn wait_for_sync(
    engine: &Arc<Engine<TokenConnector, FeedClient, Quiet, ConfigStore, NoT3>>,
    after: Option<chrono::DateTime<chrono::Utc>>,
) -> Snapshot {
    loop {
        if let Some(s) = engine.snapshot()
            && after.is_none_or(|t| s.github.last_success_at.is_some_and(|at| at > t))
        {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
