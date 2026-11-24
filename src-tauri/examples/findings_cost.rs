//! Measures what one project sync costs, cold and warm.
//!
//! ```text
//! cargo run --example findings_cost -- [owner/name] [number ...] [@login ...]
//! ```
//!
//! Runs `fetch_project` four times on one client (default dashpay/platform;
//! each number is followed, each `@login` is a tracked contributor): the first pass starts with an empty findings
//! cache, the second right after it with a warm one, the third as if 5
//! minutes had passed (every PR's findings expired; the reviews are reused)
//! and the fourth as if 30 minutes had passed (the reviews are read again
//! too). For each pass it prints the
//! requests sent, the response bytes, and the rate-limit points GitHub
//! reported (`rateLimit { cost }`), then each PR's last-round findings
//! (CodeRabbit's in brackets), the review bot's post count (its round
//! number, in every pass), head, last rounds with their commits and
//! traits. Read-only.

use pr_status_viewer_lib::error::AppResult;
use pr_status_viewer_lib::github::types::{GhFindingReviewer, GhProject};
use pr_status_viewer_lib::github::{GitHubClient, QueryUsage, resolve_token};

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
    let contributors: Vec<String> = std::env::args()
        .skip(2)
        .filter_map(|a| a.strip_prefix('@').map(str::to_owned))
        .collect();

    let client = GitHubClient::new(resolve_token().await?)?;
    let canonical = client.canonical_repo(&repo).await?;

    let mut passes = Vec::new();
    let passes_to_run = [
        ("cold (empty cache)", 0),
        ("warm (second pass)", 0),
        ("expired (5 minutes on)", 5),
        ("reviews expired (30 minutes on)", 30),
    ];
    for (label, minutes) in passes_to_run {
        client.age_findings_cache(minutes);
        let before = client.usage();
        let project = client
            .fetch_project(&canonical, &followed, &contributors)
            .await?;
        let used = diff(client.usage(), before);
        println!(
            "{label}: {} requests, {} bytes, {} points; {} PRs, {} threads, {} findings, {} traits, {} with rounds",
            used.requests,
            used.bytes,
            used.cost,
            project.prs.len(),
            project
                .prs
                .iter()
                .map(|p| p.merge.review_threads)
                .sum::<u32>(),
            project.prs.iter().map(|p| p.findings.len()).sum::<usize>(),
            project.prs.iter().map(|p| p.traits.len()).sum::<usize>(),
            project.prs.iter().filter(|p| p.rounds.is_some()).count(),
        );
        let mut by_author: Vec<(String, usize)> = Vec::new();
        for pr in &project.prs {
            match by_author.iter_mut().find(|(a, _)| *a == pr.author) {
                Some((_, n)) => *n += 1,
                None => by_author.push((pr.author.clone(), 1)),
            }
        }
        let by_author: Vec<String> = by_author.iter().map(|(a, n)| format!("{a} {n}")).collect();
        println!("  PRs by author: {}", by_author.join(", "));
        passes.push(project);
    }

    if let [cold, warm, ..] = passes.as_slice() {
        print_findings(cold, warm, &passes);
    }
    Ok(())
}

fn diff(after: QueryUsage, before: QueryUsage) -> QueryUsage {
    QueryUsage {
        requests: after.requests - before.requests,
        bytes: after.bytes - before.bytes,
        cost: after.cost - before.cost,
    }
}

/// One line per PR: its findings in each pass, flagged when they differ,
/// and its round number in every pass.
fn print_findings(cold: &GhProject, warm: &GhProject, passes: &[GhProject]) {
    for pr in &cold.prs {
        let rounds: Vec<String> = passes
            .iter()
            .map(|p| {
                p.prs
                    .iter()
                    .find(|q| q.number == pr.number)
                    .and_then(|q| q.rounds)
                    .map_or_else(|| "-".to_owned(), |n| n.to_string())
            })
            .collect();
        let again = warm.prs.iter().find(|p| p.number == pr.number);
        let same = again.is_some_and(|p| {
            p.findings == pr.findings && p.last_rounds == pr.last_rounds && p.traits == pr.traits
        });
        let traits: Vec<String> = pr.traits.iter().map(|t| t.summary.clone()).collect();
        let rabbit = pr
            .findings
            .iter()
            .filter(|f| f.reviewer == GhFindingReviewer::CodeRabbit)
            .count();
        let at = |t: Option<chrono::DateTime<chrono::Utc>>, commit: Option<&str>| {
            let when = t.map_or_else(|| "-".to_owned(), |t| t.to_rfc3339());
            match commit {
                Some(c) => format!("{when} on {}", c.get(..8).unwrap_or(c)),
                None if t.is_some() => format!("{when} on ?"),
                None => when,
            }
        };
        let last = &pr.last_rounds;
        println!(
            "#{:<5} rounds={:<11} threads={:<4} findings={:<3} [{rabbit}]  head {}  bot round {}  coderabbit round {}  traits [{}]{}",
            pr.number,
            rounds.join("/"),
            pr.merge.review_threads,
            pr.findings.len(),
            pr.head_sha.get(..8).unwrap_or(&pr.head_sha),
            at(last.review_bot, last.review_bot_commit.as_deref()),
            at(last.coderabbit, last.coderabbit_commit.as_deref()),
            traits.join("; "),
            if same {
                ""
            } else {
                "  (differs in the warm pass)"
            },
        );
    }
}
