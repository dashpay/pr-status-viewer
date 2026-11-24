//! What the self-review would do on real pull requests, without doing it.
//!
//! ```text
//! cargo run --example self_review_check -- 'owner/name#N' ['owner/name#M' ...]
//! ```
//!
//! For each PR: the self-review state the snapshot would show (from the same
//! project fetch the app makes, with the PR followed), then the fresh check
//! the confirmation shows, including the exact comment that would be posted.
//! Read-only: it stops at the check and never posts anything.

use pr_status_viewer_lib::error::{AppError, AppResult};
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
    let ids: Vec<String> = std::env::args().skip(1).collect();
    if ids.is_empty() {
        return Err(AppError::InvalidInput(
            "usage: self_review_check 'owner/name#N' ...".into(),
        ));
    }
    let client = GitHubClient::new(resolve_token().await?)?;
    let viewer = client.viewer().await?;
    println!("viewer: {}", viewer.login);
    for id in ids {
        let (repo, number) = id
            .rsplit_once('#')
            .and_then(|(r, n)| Some((r.to_owned(), n.parse::<u64>().ok()?)))
            .ok_or_else(|| AppError::InvalidInput(format!("{id} is not owner/name#N")))?;

        let project = client.fetch_project(&repo, &[number], &[]).await?;
        let shown = project
            .prs
            .iter()
            .find(|pr| pr.number == number)
            .map(|pr| merge::self_review(pr, &repo, Some(&viewer.login), &[]));
        let check = merge::self_review_check(
            pr_id(&repo, number),
            &client.self_review_check(&repo, number).await?,
        );

        println!("\n{}", check.id);
        match shown {
            Some(s) => println!("  snapshot:      {:?}", s.state),
            None => println!("  snapshot:      not listed (closed, or not found)"),
        }
        println!("  gate:          {}", check.gate.as_deref().unwrap_or("-"));
        println!("  head:          {}", check.head_sha);
        println!("  can post:      {}", check.can_post);
        if let Some(reason) = &check.reason {
            println!("  why not:       {reason}");
        }
        if let Some(at) = check.already_posted_at {
            println!("  already, at:   {at}");
        }
        println!("  would post:    {:?}  (not posted)", check.comment);
        println!("  diff:          {}", check.diff_url);
    }
    Ok(())
}
