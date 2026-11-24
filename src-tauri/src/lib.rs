//! PR Status: backend for the desktop app.
//!
//! Data flows one way: `github` and `feed` fetch raw facts, `merge` turns
//! them into a `model::Snapshot`, and `sync` hands that to the UI.

pub mod commands;
pub mod config;
pub mod error;
pub mod feed;
pub mod github;
pub mod merge;
pub mod model;
pub mod run;
pub mod sync;
pub mod t3;

use std::sync::Arc;

use tauri::{Manager, RunEvent, WindowEvent};

use commands::{AppEngine, AppRunner};
use sync::source::{ConfigRules, TauriSink, TokenConnector};
use sync::{Deps, Engine};

/// Label of the app's only window (Tauri's default).
const MAIN_WINDOW: &str = "main";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("config.json");
            let engine: Arc<AppEngine> = Arc::new(Engine::new(Deps {
                github: TokenConnector,
                feed: feed::FeedClient::new()?,
                t3: t3::T3Cli::new(),
                events: TauriSink(app.handle().clone()),
                store: config::ConfigStore::new(config_path),
                rules: ConfigRules::REAL,
                merge: merge::build_snapshot,
            }));
            app.manage(Arc::clone(&engine));
            tauri::async_runtime::spawn(engine.run());

            let runner: AppRunner = run::Runner::new(run::Deps {
                host: run::host::RealHost,
                events: run::source::TauriRunEvents(app.handle().clone()),
                clock: run::source::SystemClock,
                lookups: run::source::MachineLookups::default(),
                paths: run::RunPaths {
                    // Not the config dir: its path has a space in it, which
                    // build scripts may not survive.
                    cache_dir: app.path().app_cache_dir()?,
                    settings_file: app.path().app_config_dir()?.join("run.json"),
                },
                ios_supported: run::recipes::platform_supported(model::RunPlatform::Ios),
            });
            app.manage(runner);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(true) = event
                && window.label() == MAIN_WINDOW
                && let Some(engine) = window.try_state::<Arc<AppEngine>>()
            {
                engine.on_focus();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_config,
            commands::refresh,
            commands::add_project,
            commands::remove_project,
            commands::follow_pr,
            commands::unfollow_pr,
            commands::list_open_prs,
            commands::track_contributor,
            commands::untrack_contributor,
            commands::known_contributors,
            commands::merge_check,
            commands::squash_merge,
            commands::self_review_check,
            commands::post_self_review,
            commands::run_targets,
            commands::set_clone,
            commands::run_options,
            commands::run_start,
            commands::run_cancel,
            commands::run_job,
            commands::run_log,
            commands::run_open_log,
        ])
        .build(tauri::generate_context!());
    let app = match result {
        Ok(app) => app,
        Err(e) => {
            eprintln!("error while running PR Status: {e}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| {
        // A running build is stopped (its whole process tree) before the app
        // goes; an emulator or simulator it started keeps running.
        if let RunEvent::Exit = event
            && let Some(runner) = handle.try_state::<AppRunner>()
        {
            let runner = runner.inner().clone();
            tauri::async_runtime::block_on(runner.shutdown());
        }
    });
}
