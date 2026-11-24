//! Tauri commands the UI calls. Names and argument names match `COMMANDS`
//! in `src/api.ts`; the logic lives in `sync::Engine`.

use std::sync::Arc;

use tauri::State;

use crate::config::ConfigStore;
use crate::error::AppError;
use crate::feed::FeedClient;
use crate::model::{
    Config, KnownContributor, MergeCheck, PrSummary, RunJob, RunOptions, RunProject, RunRequest,
    SelfReviewCheck, Snapshot,
};
use crate::run::host::RealHost;
use crate::run::source::{MachineLookups, SystemClock, TauriRunEvents};
use crate::run::{PrTarget, Runner};
use crate::sync::Engine;
use crate::sync::source::{TauriSink, TokenConnector};
use crate::t3::T3Cli;

/// The engine as the app builds it.
pub type AppEngine = Engine<TokenConnector, FeedClient, TauriSink, ConfigStore, T3Cli>;

/// The example-app runner as the app builds it.
pub type AppRunner = Runner<RealHost, TauriRunEvents, SystemClock, MachineLookups>;

type EngineState<'a> = State<'a, Arc<AppEngine>>;
type RunnerState<'a> = State<'a, AppRunner>;

#[tauri::command]
pub fn get_snapshot(engine: EngineState<'_>) -> Option<Snapshot> {
    engine.snapshot()
}

#[tauri::command]
pub fn get_config(engine: EngineState<'_>) -> Config {
    engine.config()
}

#[tauri::command]
pub fn refresh(engine: EngineState<'_>) {
    engine.request_sync();
}

#[tauri::command]
pub async fn add_project(engine: EngineState<'_>, input: String) -> Result<Config, AppError> {
    engine.add_project(&input).await
}

#[tauri::command]
pub fn remove_project(engine: EngineState<'_>, repo: String) -> Result<Config, AppError> {
    engine.remove_project(&repo)
}

#[tauri::command]
pub fn follow_pr(engine: EngineState<'_>, input: String) -> Result<Config, AppError> {
    engine.follow_pr(&input)
}

#[tauri::command]
pub fn unfollow_pr(engine: EngineState<'_>, id: String) -> Result<Config, AppError> {
    engine.unfollow_pr(&id)
}

#[tauri::command]
pub async fn list_open_prs(
    engine: EngineState<'_>,
    repo: String,
) -> Result<Vec<PrSummary>, AppError> {
    engine.list_open_prs(&repo).await
}

/// Tracks the contributor named by `input`: a login, `@login` or a GitHub
/// profile link. Checked with GitHub; stored in GitHub's casing.
#[tauri::command]
pub async fn track_contributor(engine: EngineState<'_>, input: String) -> Result<Config, AppError> {
    engine.track_contributor(&input).await
}

#[tauri::command]
pub fn untrack_contributor(engine: EngineState<'_>, login: String) -> Result<Config, AppError> {
    engine.untrack_contributor(&login)
}

/// The authors of open PRs in the tracked projects that could be tracked.
#[tauri::command]
pub async fn known_contributors(
    engine: EngineState<'_>,
) -> Result<Vec<KnownContributor>, AppError> {
    engine.known_contributors().await
}

/// A fresh check of whether the PR `id` can be squash-merged now.
#[tauri::command]
pub async fn merge_check(engine: EngineState<'_>, id: String) -> Result<MergeCheck, AppError> {
    engine.merge_check(&id).await
}

/// Squash-merges the PR `id` if it is still at `expected_head_sha` and ready.
#[tauri::command]
pub async fn squash_merge(
    engine: EngineState<'_>,
    id: String,
    expected_head_sha: String,
) -> Result<(), AppError> {
    engine.squash_merge(&id, &expected_head_sha).await
}

/// A fresh check of whether the user can post the self-review on the PR `id`.
#[tauri::command]
pub async fn self_review_check(
    engine: EngineState<'_>,
    id: String,
) -> Result<SelfReviewCheck, AppError> {
    engine.self_review_check(&id).await
}

/// Posts `/self-reviewed <head>` on the PR `id` as the user, if it is still at
/// `expected_head_sha` and they may. Only after the user confirms.
#[tauri::command]
pub async fn post_self_review(
    engine: EngineState<'_>,
    id: String,
    expected_head_sha: String,
) -> Result<(), AppError> {
    engine.post_self_review(&id, &expected_head_sha).await
}

// ---- Running a project's example apps ----

/// The PR `id` as it is in the current snapshot. The runner never acts on a
/// PR the snapshot does not hold.
fn pr_target(engine: &AppEngine, id: &str) -> Result<PrTarget, AppError> {
    let snapshot = engine.snapshot().ok_or_else(|| {
        AppError::InvalidInput(
            "The pull requests have not loaded yet. Try again in a moment.".into(),
        )
    })?;
    snapshot
        .projects
        .iter()
        .flat_map(|p| &p.prs)
        .find(|pr| pr.id.eq_ignore_ascii_case(id))
        .map(|pr| PrTarget {
            repo: pr.repo.clone(),
            number: pr.number,
            head_sha: pr.head_sha.clone(),
            head_branch: pr.head_branch.clone(),
        })
        .ok_or_else(|| {
            AppError::InvalidInput(format!(
                "{id} is not in the list of pull requests any more."
            ))
        })
}

#[tauri::command]
pub async fn run_targets(
    engine: EngineState<'_>,
    runner: RunnerState<'_>,
) -> Result<Vec<RunProject>, AppError> {
    Ok(runner.targets(&engine.config().projects).await)
}

#[tauri::command]
pub async fn set_clone(
    engine: EngineState<'_>,
    runner: RunnerState<'_>,
    repo: String,
    path: Option<String>,
) -> Result<Vec<RunProject>, AppError> {
    runner
        .set_clone(&engine.config().projects, &repo, path.as_deref())
        .await
}

#[tauri::command]
pub async fn run_options(
    engine: EngineState<'_>,
    runner: RunnerState<'_>,
    id: String,
) -> Result<RunOptions, AppError> {
    let pr = pr_target(&engine, &id)?;
    runner.options(&pr).await
}

#[tauri::command]
pub async fn run_start(
    engine: EngineState<'_>,
    runner: RunnerState<'_>,
    request: RunRequest,
) -> Result<RunJob, AppError> {
    let pr = pr_target(&engine, &request.id)?;
    runner.start(&pr, &request).await
}

#[tauri::command]
pub fn run_cancel(runner: RunnerState<'_>) {
    runner.cancel();
}

#[tauri::command]
pub fn run_job(runner: RunnerState<'_>) -> Option<RunJob> {
    runner.job()
}

#[tauri::command]
pub fn run_log(runner: RunnerState<'_>) -> Vec<String> {
    runner.log()
}

/// Opens the current job's log file, and only that file, in the user's
/// default app for it.
#[tauri::command]
pub fn run_open_log(app: tauri::AppHandle, runner: RunnerState<'_>) -> Result<(), AppError> {
    use tauri_plugin_opener::OpenerExt;
    let path = runner
        .log_path()
        .filter(|p| p.is_file())
        .ok_or_else(|| AppError::InvalidInput("There is no log to open.".into()))?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Other(format!("Could not open the log: {e}")))
}
