//! Live smoke test of the GitHub client.
//!
//! ```text
//! cargo run --example gh_smoke -- [owner/name] [number ...]
//! ```
//!
//! Resolves a token the way the app does, prints the viewer, then one line
//! per PR the app would show for the repo (default dashpay/platform). Each
//! number given is followed, and also gets a fresh merge check. Read-only:
//! it never merges anything.

use pr_status_viewer_lib::error::AppResult;
use pr_status_viewer_lib::github::{GitHubClient, resolve_token};
use pr_status_viewer_lib::merge;
use pr_status_viewer_lib::model::pr_id;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let repo = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dashpay/platform".to_owned());
    let followed: Vec<u64> = std::env::args()
        .skip(2)
        .filter_map(|a| a.parse().ok())
        .collect();

    let client = GitHubClient::new(resolve_token().await?)?;
    let viewer = client.viewer().await?;
    println!("viewer: {}", viewer.login);

    let canonical = client.canonical_repo(&repo).await?;
    let project = client.fetch_project(&canonical, &followed, &[]).await?;
    println!(
        "{}: {} open, {} listed",
        project.repo,
        project.open_count,
        project.prs.len()
    );
    for pr in &project.prs {
        let bot = match &pr.bot_check {
            Some(b) => format!(
                "{:?}/{} \"{}\"",
                b.status,
                b.conclusion.as_deref().unwrap_or("-"),
                b.title.as_deref().unwrap_or("")
            ),
            None => "no bot check".to_owned(),
        };
        let m = &pr.merge;
        let ready = merge::readiness(m, pr.is_draft);
        println!(
            "#{:<5} ci={:<8} bot={bot}  reviews={} requests={}{}",
            pr.number,
            format!("{:?}", pr.ci),
            pr.reviews.len(),
            pr.review_requests.len(),
            if pr.is_draft { " draft" } else { "" },
        );
        println!(
            "       merge={}/{:?} threads={}/{} unresolved base={} queue={} -> {}",
            m.merge_state,
            m.mergeable,
            m.unresolved_threads,
            m.review_threads,
            m.base_branch,
            m.merge_queue,
            ready.reason.as_deref().unwrap_or("ready"),
        );
    }

    for &number in &followed {
        let check = client.merge_check(&canonical, number).await?;
        let answer = merge::merge_check(pr_id(&canonical, number), &check);
        println!(
            "merge_check #{number}: open={} head={} state={}/{:?} threads={}/{} title={:?} -> {}",
            check.is_open,
            check.head_sha.get(..8).unwrap_or(&check.head_sha),
            check.facts.merge_state,
            check.facts.mergeable,
            check.facts.unresolved_threads,
            check.facts.review_threads,
            answer.commit_title,
            answer.merge.reason.as_deref().unwrap_or("ready"),
        );
    }

    let open = client.list_open_prs(&canonical).await?;
    println!("list_open_prs: {} rows", open.len());
    Ok(())
}
