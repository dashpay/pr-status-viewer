//! Runs the real sync engine once without a window and prints the snapshot
//! the UI would receive, as JSON.
//!
//!   cargo run --example snapshot_dump -- owner/name [owner/name#123 ...] > snapshot.json
//!
//! Arguments with a `#` are followed PRs; the rest are projects. Uses a
//! throwaway config file, so it never touches the app's real config.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pr_status_viewer_lib::config::ConfigStore;
use pr_status_viewer_lib::feed::FeedClient;
use pr_status_viewer_lib::merge;
use pr_status_viewer_lib::model::{Config, Snapshot, SyncState};
use pr_status_viewer_lib::sync::source::{ConfigRules, EventSink, TokenConnector};
use pr_status_viewer_lib::sync::{Deps, Engine};
use pr_status_viewer_lib::t3::T3Cli;

#[derive(Default)]
struct Capture {
    latest: Mutex<Option<Snapshot>>,
    cycles_done: Mutex<u32>,
}

struct Sink(Arc<Capture>);

impl EventSink for Sink {
    fn snapshot(&self, snapshot: &Snapshot) {
        if let Ok(mut latest) = self.0.latest.lock() {
            *latest = Some(snapshot.clone());
        }
    }

    fn sync_state(&self, state: SyncState) {
        if !state.syncing
            && let Ok(mut done) = self.0.cycles_done.lock()
        {
            *done += 1;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (followed, projects): (Vec<String>, Vec<String>) =
        std::env::args().skip(1).partition(|a| a.contains('#'));
    let dir = std::env::temp_dir().join(format!("pr-status-dump-{}", std::process::id()));
    let store = ConfigStore::new(dir.join("config.json"));
    store.save(&Config {
        projects,
        followed,
        contributors: vec![],
    })?;

    let capture = Arc::new(Capture::default());
    let engine = Arc::new(Engine::new(Deps {
        github: TokenConnector,
        feed: FeedClient::new()?,
        t3: T3Cli::new(),
        events: Sink(Arc::clone(&capture)),
        store,
        rules: ConfigRules::REAL,
        merge: merge::build_snapshot,
    }));
    let runner = tokio::spawn(Arc::clone(&engine).run());

    // The first cycle ends with `sync-state { syncing: false }`.
    for _ in 0..240 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if capture.cycles_done.lock().map(|d| *d > 0).unwrap_or(false) {
            break;
        }
    }
    runner.abort();
    let _ = std::fs::remove_dir_all(&dir);

    let snapshot = capture.latest.lock().ok().and_then(|s| s.clone());
    match snapshot {
        Some(snapshot) => {
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
            Ok(())
        }
        None => Err("no snapshot was produced within 60 seconds".into()),
    }
}
