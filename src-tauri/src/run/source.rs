//! The seams the run engine talks through besides [`super::host::Host`]:
//! events for the UI, the time, and lookups on this machine. The app plugs
//! in the real ones; the tests plug in fakes.

use std::future::Future;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter};

use crate::model::{RunJob, RunLog};
use crate::t3::{self, T3Reader};

use super::env::{self, BuildEnv};
use super::host::RealHost;

/// Event carrying a whole `RunJob`. Must match `EVENTS.runJob` in `src/api.ts`.
pub const EVENT_RUN_JOB: &str = "run-job";
/// Event carrying a `RunLog` batch. Must match `EVENTS.runLog` in `src/api.ts`.
pub const EVENT_RUN_LOG: &str = "run-log";

pub trait RunEvents: Send + Sync + 'static {
    fn job(&self, job: &RunJob);
    fn log(&self, log: &RunLog);
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// Facts about this machine the runner looks up rather than computes.
pub trait Lookups: Send + Sync + 'static {
    /// Directories of T3 Code's live projects, in T3 Code's order. Empty
    /// when T3 Code is not there or cannot be read.
    fn t3_workspaces(&self) -> impl Future<Output = Vec<PathBuf>> + Send;
    /// The environment builds run in (see `env.rs`).
    fn build_env(&self) -> impl Future<Output = BuildEnv> + Send;
}

// ---- Real implementations ----

pub struct TauriRunEvents(pub AppHandle);

impl RunEvents for TauriRunEvents {
    fn job(&self, job: &RunJob) {
        if let Err(e) = self.0.emit(EVENT_RUN_JOB, job) {
            log::warn!("could not emit {EVENT_RUN_JOB}: {e}");
        }
    }

    fn log(&self, log: &RunLog) {
        if let Err(e) = self.0.emit(EVENT_RUN_LOG, log) {
            log::warn!("could not emit {EVENT_RUN_LOG}: {e}");
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// T3 Code's database where it keeps it, and the user's login shell.
#[derive(Debug, Clone, Default)]
pub struct MachineLookups {
    /// Overrides T3 Code's database location.
    pub t3_db: Option<PathBuf>,
}

impl Lookups for MachineLookups {
    async fn t3_workspaces(&self) -> Vec<PathBuf> {
        let Some(path) = self.t3_db.clone().or_else(t3::default_db_path) else {
            return Vec::new();
        };
        let read = tokio::task::spawn_blocking(move || T3Reader::new(path).read_project_roots());
        match read.await {
            Ok(Ok(Some(roots))) => roots,
            Ok(Ok(None)) => Vec::new(),
            Ok(Err(e)) => {
                log::debug!("run: T3 Code's projects are unavailable: {e}");
                Vec::new()
            }
            Err(e) => {
                log::warn!("run: reading T3 Code's projects failed: {e}");
                Vec::new()
            }
        }
    }

    async fn build_env(&self) -> BuildEnv {
        env::gather(&RealHost).await
    }
}
