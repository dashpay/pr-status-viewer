//! T3 Code's project list, the one thing still read from its database: its
//! `t3` command has no way to list projects.
//!
//! The database is T3 Code's own. We open it read-only, never write to it,
//! and read only `projection_projects`: `workspace_root`, `deleted_at`, and
//! `archived_at` and `created_at` when they exist (observed in T3 Code
//! 0.1.11). Anything else about the file is an error the caller treats as
//! "unavailable".

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::{AppError, AppResult};

/// Overrides the database location, for tests and unusual setups.
const DB_PATH_ENV: &str = "PR_STATUS_T3_DB";

const PROJECTS_TABLE: &str = "projection_projects";
const PROJECT_COLUMNS: &[&str] = &["workspace_root", "deleted_at"];

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);

pub(super) const MSG_OPEN: &str = "Could not open T3 Code's data.";
pub(super) const MSG_FORMAT: &str =
    "T3 Code's data is in a format this version does not understand.";
pub(super) const MSG_READ: &str = "Could not read T3 Code's data.";

/// Where T3 Code keeps its state database, if a home directory is known.
/// Does not check that the file exists.
pub fn default_db_path() -> Option<PathBuf> {
    db_path_from(std::env::var_os(DB_PATH_ENV), std::env::var_os("HOME"))
}

/// [`default_db_path`] with the environment passed in, so it can be tested
/// without touching the process environment.
pub(super) fn db_path_from(
    override_path: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let home = home.filter(|h| !h.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".t3")
            .join("userdata")
            .join("statev2.sqlite"),
    )
}

/// T3 Code's state database, read for its project list only.
#[derive(Debug, Clone)]
pub struct T3Reader {
    path: PathBuf,
}

impl T3Reader {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The directories of T3 Code's live projects (not deleted, not
    /// archived), oldest project first. `Ok(None)` when the database file
    /// does not exist. Blocking: call it from `spawn_blocking`.
    pub fn read_project_roots(&self) -> AppResult<Option<Vec<PathBuf>>> {
        match std::fs::metadata(&self.path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                log::debug!("t3: cannot stat the database: {e}");
                return Err(AppError::Other(MSG_OPEN.into()));
            }
        }
        let conn = open_read_only(&self.path)?;
        read_project_roots(&conn).map(Some)
    }
}

fn read_project_roots(conn: &Connection) -> AppResult<Vec<PathBuf>> {
    let columns = table_columns(conn, PROJECTS_TABLE).map_err(|e| {
        log::debug!("t3: cannot read the schema of {PROJECTS_TABLE}: {e}");
        AppError::Other(MSG_FORMAT.into())
    })?;
    if let Some(missing) = PROJECT_COLUMNS.iter().find(|c| !columns.contains(**c)) {
        log::debug!("t3: {PROJECTS_TABLE}.{missing} is missing");
        return Err(AppError::Other(MSG_FORMAT.into()));
    }
    let archived = if columns.contains("archived_at") {
        "AND archived_at IS NULL"
    } else {
        ""
    };
    let order = if columns.contains("created_at") {
        "ORDER BY created_at, rowid"
    } else {
        "ORDER BY rowid"
    };
    let sql = format!(
        "SELECT workspace_root FROM {PROJECTS_TABLE}
         WHERE deleted_at IS NULL {archived} {order}"
    );
    let read = || -> rusqlite::Result<Vec<PathBuf>> {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut roots = Vec::new();
        while let Some(row) = rows.next()? {
            // A row with an unexpected type or a relative path is skipped.
            let Ok(root) = row.get::<_, String>(0) else {
                continue;
            };
            let path = PathBuf::from(root.trim());
            if path.is_absolute() && !roots.contains(&path) {
                roots.push(path);
            }
        }
        Ok(roots)
    };
    read().map_err(|e| {
        log::debug!("t3: project query failed: {e}");
        AppError::Other(MSG_READ.into())
    })
}

/// Opens the database without the ability to create or write it. Works while
/// T3 Code holds it open in WAL mode: a read-only connection only takes read
/// locks and never checkpoints.
pub(super) fn open_read_only(path: &Path) -> AppResult<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let open = || -> rusqlite::Result<Connection> {
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "query_only", true)?;
        Ok(conn)
    };
    open().map_err(|e| {
        log::debug!("t3: cannot open the database: {e}");
        AppError::Other(MSG_OPEN.into())
    })
}

fn table_columns(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let names = stmt.query_map([table], |row| row.get::<_, String>(0))?;
    names.collect()
}
