use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use pretty_assertions::assert_eq;
use rusqlite::{Connection, params};
use tempfile::TempDir;

use super::cli::{
    CLI_PATH_ENV, MSG_CLI, T3Project, cli_path_from, combine, excerpt, projects_to_read,
};
use super::parse::{ListedPr, ListedThread, links_of, parse_thread_list, state_of};
use super::projects::{MSG_FORMAT, db_path_from, open_read_only};
use super::*;
use crate::error::AppError;
use crate::model::T3ThreadState::{self, Idle, Waiting, Working};

const LIST: &str = include_str!("../../tests/fixtures/t3/thread_list.json");
const OTHER_PROJECT: &str = include_str!("../../tests/fixtures/t3/thread_list_other_project.json");

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// (repo, number, thread, state, watching) of each link.
fn summary(links: &[T3Link]) -> Vec<(String, u64, String, T3ThreadState, bool)> {
    links
        .iter()
        .map(|l| {
            (
                l.repo.clone(),
                l.number,
                l.thread_id.clone(),
                l.state,
                l.watching,
            )
        })
        .collect()
}

fn row(
    repo: &str,
    number: u64,
    thread: &str,
    state: T3ThreadState,
    watching: bool,
) -> (String, u64, String, T3ThreadState, bool) {
    (repo.into(), number, thread.into(), state, watching)
}

fn file_digest(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
    let meta = std::fs::metadata(path).unwrap();
    (std::fs::read(path).unwrap(), meta.modified().unwrap())
}

// ---- Locating things ----

#[test]
fn default_db_path_prefers_override_then_home() {
    assert_eq!(
        db_path_from(Some("/x/t3.sqlite".into()), Some("/home/u".into())),
        Some(PathBuf::from("/x/t3.sqlite"))
    );
    assert_eq!(
        db_path_from(Some("".into()), Some("/home/u".into())),
        Some(PathBuf::from("/home/u/.t3/userdata/statev2.sqlite"))
    );
    assert_eq!(
        db_path_from(None, Some("/home/u".into())),
        Some(PathBuf::from("/home/u/.t3/userdata/statev2.sqlite"))
    );
    assert_eq!(db_path_from(None, None), None);
    assert_eq!(db_path_from(None, Some("".into())), None);
}

#[test]
fn cli_path_prefers_override_then_t3_home_then_home_and_never_path() {
    assert_eq!(CLI_PATH_ENV, "PR_STATUS_T3_CLI");
    assert_eq!(
        cli_path_from(
            Some("/opt/t3".into()),
            Some("/t3home".into()),
            Some("/home/u".into())
        ),
        Some(PathBuf::from("/opt/t3"))
    );
    assert_eq!(
        cli_path_from(
            Some("".into()),
            Some("/t3home".into()),
            Some("/home/u".into())
        ),
        Some(PathBuf::from("/t3home/bin/t3"))
    );
    assert_eq!(
        cli_path_from(None, Some("".into()), Some("/home/u".into())),
        Some(PathBuf::from("/home/u/.t3/bin/t3"))
    );
    assert_eq!(cli_path_from(None, None, None), None);
    assert_eq!(cli_path_from(None, None, Some("".into())), None);
}

#[tokio::test]
async fn a_missing_command_means_not_installed_and_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let reader = T3Cli::with_paths(
        Some(dir.path().join("bin").join("t3")),
        Some(dir.path().join("statev2.sqlite")),
    );
    assert_eq!(reader.read(&["acme/app".into()]).await.unwrap(), None);
    // A directory where the command should be is not a command either.
    let reader = T3Cli::with_paths(Some(dir.path().to_path_buf()), None);
    assert_eq!(reader.read(&["acme/app".into()]).await.unwrap(), None);
    assert!(!dir.path().join("statev2.sqlite").exists());
}

// ---- Which projects are read ----

fn project(root: &str, repo: Option<&str>) -> T3Project {
    T3Project {
        root: PathBuf::from(root),
        repo: repo.map(str::to_owned),
    }
}

#[test]
fn only_projects_cloned_from_a_wanted_repo_are_read() {
    let projects = vec![
        project("/src/app", Some("Acme/App")),
        project("/src/scratch", None),
        project("/src/lib", Some("acme/lib")),
        project("/src/app-2", Some("acme/app")),
        project("/src/other", Some("someone/else")),
    ];
    let wanted = vec!["acme/app".to_owned(), " ACME/LIB ".to_owned()];
    assert_eq!(
        projects_to_read(&projects, &wanted),
        vec![
            Path::new("/src/app"),
            Path::new("/src/lib"),
            Path::new("/src/app-2")
        ]
    );
    assert!(projects_to_read(&projects, &[]).is_empty());
    assert!(projects_to_read(&[], &wanted).is_empty());
}

// ---- Parsing ----

#[test]
fn statuses_map_to_states() {
    for s in [
        "running",
        "RUNNING",
        " queued ",
        "starting",
        "preparing",
        "pending",
        "in_progress",
    ] {
        assert_eq!(state_of(Some(s)), Working, "{s}");
    }
    assert_eq!(state_of(Some("waiting")), Waiting);
    assert_eq!(state_of(Some("Waiting")), Waiting);
    for s in [
        "idle",
        "completed",
        "failed",
        "interrupted",
        "",
        "hibernating",
    ] {
        assert_eq!(state_of(Some(s)), Idle, "{s}");
    }
    assert_eq!(state_of(None), Idle);
}

#[test]
fn the_fixture_parses_into_the_expected_links() {
    let links = links_of(parse_thread_list(LIST).unwrap());
    assert_eq!(
        summary(&links),
        vec![
            row("acme/app", 101, "thread-running", Working, true),
            row("acme/app", 102, "thread-waiting", Waiting, true),
            // Pending tasks and a thread wait do not make an idle thread busy.
            row("acme/app", 103, "thread-idle-pending", Idle, false),
            row("acme/app", 104, "thread-failed", Idle, true),
            // Settled, but running again: kept.
            row(
                "acme/app",
                106,
                "thread-settled-but-running",
                Working,
                false
            ),
            // The same PR twice (case differs): one link, watched if either is.
            row("acme/app", 107, "thread-many-prs", Working, true),
            row("acme/app", 109, "thread-unknown-status", Idle, false),
            // No host but a github.com URL.
            row("acme/lib", 9, "thread-many-prs", Working, false),
            // From `linkedPullRequest`.
            row("acme/lib", 13, "thread-many-prs", Working, true),
        ]
    );
    let first = &links[0];
    assert_eq!(first.title, "Add the widget cache");
    assert_eq!(first.last_active_at, Some(at("2026-11-22T12:30:00Z")));
    let many = links.iter().find(|l| l.number == 107).unwrap();
    assert_eq!(many.title, "Split the migration");
    assert_eq!(many.last_active_at, None, "an unreadable time is no time");
}

#[test]
fn unreadable_threads_are_skipped_one_by_one() {
    let threads = parse_thread_list(LIST).unwrap();
    let ids: Vec<&str> = threads.iter().map(|t| t.thread_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "thread-running",
            "thread-waiting",
            "thread-idle-pending",
            "thread-failed",
            "thread-settled",
            "thread-settled-but-running",
            "thread-no-pr",
            "thread-many-prs",
            "thread-unknown-status",
        ]
    );
    let settled = threads
        .iter()
        .find(|t| t.thread_id == "thread-settled")
        .unwrap();
    assert!(settled.settled);
    assert!(settled.prs.len() == 1);
}

#[test]
fn output_that_is_not_a_thread_list_is_none() {
    for out in [
        "",
        "No T3 Code project matches '/tmp/x'. Pass a project id or a path inside a project.",
        "{\"error\": \"nope\"}",
        "42",
        "[1, 2",
    ] {
        assert_eq!(parse_thread_list(out), None, "{out}");
    }
    assert_eq!(parse_thread_list("[]"), Some(vec![]));
    assert_eq!(parse_thread_list("  [ ]\n"), Some(vec![]));
    // A list wrapped in an object, should a later version do that.
    let wrapped = format!("{{\"threads\": {OTHER_PROJECT}}}");
    assert_eq!(parse_thread_list(&wrapped).unwrap().len(), 2);
}

#[test]
fn settled_at_alone_marks_a_thread_settled() {
    let out = r#"[
        {"threadId": "a", "status": "idle", "settledAt": "2026-11-20T10:00:00Z",
         "pullRequests": [{"host": "github.com", "repository": "o/r", "number": 1}]},
        {"threadId": "b", "status": "idle", "settled": false, "settledAt": "2026-11-20T10:00:00Z",
         "pullRequests": [{"host": "github.com", "repository": "o/r", "number": 2}]},
        {"threadId": "c", "status": "waiting", "settled": true,
         "pullRequests": [{"host": "github.com", "repository": "o/r", "number": 3}]}
    ]"#;
    let links = links_of(parse_thread_list(out).unwrap());
    assert_eq!(
        summary(&links),
        vec![
            row("o/r", 2, "b", Idle, false),
            row("o/r", 3, "c", Waiting, false)
        ]
    );
}

#[test]
fn a_thread_listed_under_two_projects_counts_once_as_its_latest_copy() {
    let mut threads = parse_thread_list(OTHER_PROJECT).unwrap();
    threads.extend(parse_thread_list(LIST).unwrap());
    let links = links_of(threads);
    let waiting: Vec<_> = links
        .iter()
        .filter(|l| l.thread_id == "thread-waiting")
        .collect();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].state, Waiting);
    assert!(waiting[0].watching);
    assert_eq!(waiting[0].title, "Fix the flaky login test");
    assert!(
        links
            .iter()
            .any(|l| l.thread_id == "thread-lib" && l.number == 20)
    );
}

#[test]
fn links_are_sorted_by_repo_number_and_thread() {
    let thread = |id: &str, prs: &[(&str, u64)]| ListedThread {
        thread_id: id.into(),
        title: id.into(),
        state: Idle,
        settled: false,
        updated_at: None,
        prs: prs
            .iter()
            .map(|(repo, number)| ListedPr {
                repo: (*repo).into(),
                number: *number,
                watching: false,
            })
            .collect(),
    };
    let links = links_of(vec![
        thread("z", &[("b/r", 1), ("A/r", 2)]),
        thread("a", &[("a/r", 2)]),
    ]);
    let order: Vec<_> = links
        .iter()
        .map(|l| (l.repo.as_str(), l.number, l.thread_id.as_str()))
        .collect();
    assert_eq!(
        order,
        vec![("a/r", 2, "a"), ("A/r", 2, "z"), ("b/r", 1, "z")]
    );
}

// ---- Putting projects together ----

#[test]
fn every_project_failing_is_an_error_and_some_failing_is_fine() {
    let ok = |s: &str| Ok(s.to_owned());
    let failed = || Err::<String, String>("exit status: 1: No T3 Code project matches".into());

    let err = combine(vec![
        (PathBuf::from("/a"), failed()),
        (PathBuf::from("/b"), ok("not json")),
    ])
    .unwrap_err();
    assert_eq!(err.to_string(), MSG_CLI);

    let links = combine(vec![
        (PathBuf::from("/a"), failed()),
        (PathBuf::from("/b"), ok(OTHER_PROJECT)),
    ])
    .unwrap();
    assert_eq!(
        summary(&links),
        vec![
            row("acme/app", 102, "thread-waiting", Idle, false),
            row("acme/lib", 20, "thread-lib", Working, true),
        ]
    );

    // Nothing to read is no threads, not a failure.
    assert_eq!(combine(vec![]).unwrap(), vec![]);
    // A project with no threads at all was still read.
    assert_eq!(
        combine(vec![
            (PathBuf::from("/a"), ok("[]")),
            (PathBuf::from("/b"), failed())
        ])
        .unwrap(),
        vec![]
    );
}

#[test]
fn excerpts_are_short_and_on_one_line() {
    assert_eq!(excerpt("  one\n two\t three \n"), "one two three");
    let long = "x".repeat(500);
    let cut = excerpt(&long);
    assert_eq!(cut.chars().count(), 201);
    assert!(cut.ends_with('…'));
    assert_eq!(excerpt(""), "");
}

#[test]
fn query_only_connection_refuses_writes() {
    let (_dir, path, _conn) = projects_db(true);
    let conn = open_read_only(&path).unwrap();
    assert!(conn.execute("DELETE FROM projection_projects", []).is_err());
}

// ---- Projects (for finding a project's local clone) ----

fn projects_db(with_archived: bool) -> (TempDir, PathBuf, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("statev2.sqlite");
    let conn = Connection::open(&path).unwrap();
    let archived = if with_archived {
        ", archived_at TEXT"
    } else {
        ""
    };
    conn.execute_batch(&format!(
        "CREATE TABLE projection_projects (
            project_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            workspace_root TEXT NOT NULL,
            scripts_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT '2026-11-21T00:00:00.000Z',
            deleted_at TEXT{archived}
         );"
    ))
    .unwrap();
    (dir, path, conn)
}

#[test]
fn project_roots_are_live_ones_oldest_first() {
    let (_dir, path, conn) = projects_db(true);
    for (id, root, created, deleted, archived) in [
        ("p3", "/src/later", "2026-11-21T18:00:00.000Z", None, None),
        (
            "p1",
            "/src/platform",
            "2026-11-21T16:38:51.948Z",
            None,
            None,
        ),
        (
            "p2",
            "/src/deleted",
            "2026-11-21T16:00:00.000Z",
            Some("2026-11-21T17:00:00Z"),
            None,
        ),
        (
            "p4",
            "/src/archived",
            "2026-11-21T15:00:00.000Z",
            None,
            Some("2026-11-21T17:00:00Z"),
        ),
        (
            "p5",
            "relative/path",
            "2026-11-21T15:00:00.000Z",
            None,
            None,
        ),
        (
            "p6",
            "/src/platform",
            "2026-11-21T19:00:00.000Z",
            None,
            None,
        ),
    ] {
        conn.execute(
            "INSERT INTO projection_projects (project_id, title, workspace_root, created_at, deleted_at, archived_at)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5)",
            params![id, root, created, deleted, archived],
        )
        .unwrap();
    }
    let roots = T3Reader::new(path).read_project_roots().unwrap().unwrap();
    assert_eq!(
        roots,
        vec![PathBuf::from("/src/platform"), PathBuf::from("/src/later")]
    );
}

#[test]
fn project_roots_without_an_archived_column_still_read() {
    let (_dir, path, conn) = projects_db(false);
    conn.execute(
        "INSERT INTO projection_projects (project_id, title, workspace_root, created_at)
         VALUES ('p1', 'x', '/src/x', '2026-11-21T00:00:00Z')",
        [],
    )
    .unwrap();
    assert_eq!(
        T3Reader::new(path).read_project_roots().unwrap().unwrap(),
        vec![PathBuf::from("/src/x")]
    );
}

#[test]
fn project_roots_tolerate_a_missing_file_and_a_foreign_schema() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.sqlite");
    assert_eq!(
        T3Reader::new(missing.clone()).read_project_roots().unwrap(),
        None
    );
    assert!(!missing.exists(), "reading created the database");

    // Some other SQLite file, without the projects table.
    let foreign = dir.path().join("foreign.sqlite");
    Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE something_else (x TEXT);")
        .unwrap();
    assert_eq!(
        T3Reader::new(foreign)
            .read_project_roots()
            .unwrap_err()
            .to_string(),
        MSG_FORMAT
    );

    let not_sqlite = dir.path().join("text.sqlite");
    std::fs::write(
        &not_sqlite,
        b"this is not a database, just some text long enough",
    )
    .unwrap();
    assert!(matches!(
        T3Reader::new(not_sqlite).read_project_roots(),
        Err(AppError::Other(_))
    ));
}

#[test]
fn reading_project_roots_leaves_the_file_untouched() {
    let (_dir, path, conn) = projects_db(true);
    conn.execute(
        "INSERT INTO projection_projects (project_id, title, workspace_root, created_at)
         VALUES ('p1', 'x', '/src/x', '2026-11-21T00:00:00Z')",
        [],
    )
    .unwrap();
    let before = file_digest(&path);
    std::thread::sleep(std::time::Duration::from_millis(20));
    T3Reader::new(path.clone()).read_project_roots().unwrap();
    assert!(before == file_digest(&path), "the database file changed");
}
