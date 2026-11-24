//! Fetches the live review feed and prints what the app makes of it.
//!
//! cargo run --example feed_smoke

use chrono::{DateTime, Utc};
use pr_status_viewer_lib::feed::FeedClient;
use pr_status_viewer_lib::merge::eta;

fn show(t: Option<DateTime<Utc>>) -> String {
    t.map(|t| t.format("%Y-%m-%d %H:%M:%SZ").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[tokio::main]
async fn main() {
    let feed = match FeedClient::new() {
        Ok(client) => client.fetch().await,
        Err(e) => Err(e),
    };
    let feed = match feed {
        Ok(feed) => feed,
        Err(e) => {
            eprintln!("could not read the review feed: {e}");
            std::process::exit(1);
        }
    };
    let as_of = feed.data_as_of;

    println!("data_as_of:   {}", show(as_of));
    println!(
        "running:      {} (heads {:?})",
        feed.active.len(),
        feed.heads_running
    );
    println!(
        "queued:       {} (heads {:?})",
        feed.queued.len(),
        feed.heads_queued
    );
    println!("median run:   {:?} min", feed.median_run_minutes);
    println!(
        "typical:      normal {:.0} s, priority {:.0} s (slot wait + duration)",
        eta::typical_review_seconds(&feed, false).unwrap_or(f64::NAN),
        eta::typical_review_seconds(&feed, true).unwrap_or(f64::NAN)
    );
    println!("degraded:     {}", feed.degraded);

    println!("\nactive:");
    for a in &feed.active {
        let finishes = as_of.and_then(|t| a.remaining_seconds.and_then(|r| eta::add_seconds(t, r)));
        let steps: Vec<_> = a
            .steps
            .iter()
            .map(|s| format!("{}={}", s.name, s.status.as_deref().unwrap_or("?")))
            .collect();
        println!(
            "  {}#{} phase={} progress={:.0}% tier={} overdue={} finishes={} steps=[{}] upcoming={:?}",
            a.repo,
            a.number,
            a.phase.as_deref().unwrap_or("?"),
            a.progress.unwrap_or(0.0) * 100.0,
            a.tier.as_deref().unwrap_or("?"),
            a.overdue,
            show(finishes),
            steps.join(" "),
            a.upcoming
        );
    }

    println!("\nqueued:");
    for q in &feed.queued {
        let starts = eta::queued_starts_at(q, as_of);
        let finishes = eta::queued_finishes_at(q, starts, &feed);
        println!(
            "  {}#{} position={:?} priority={} eligible={:?} starts={} finishes~{} reason={:?}",
            q.repo,
            q.number,
            q.position,
            q.priority,
            q.eligible,
            show(starts),
            show(finishes),
            q.reason.as_deref().unwrap_or("")
        );
    }
}
