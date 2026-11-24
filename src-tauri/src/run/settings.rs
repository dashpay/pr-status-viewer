//! The run feature's own small settings file, `<app config dir>/run.json`:
//! where the user said each project's local clone is.
//!
//! A missing or unreadable file means "nothing set" and is never an error
//! when reading. A file that exists but cannot be parsed is never written
//! over: changing a setting then fails with a sentence instead.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    /// `owner/name` -> absolute path of the clone.
    #[serde(default)]
    clones: BTreeMap<String, String>,
    /// Fields a later version wrote, kept as they are.
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

enum Loaded {
    Missing,
    Parsed(File),
    Unreadable,
}

#[derive(Debug, Clone)]
pub struct RunSettings {
    path: PathBuf,
}

impl RunSettings {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Loaded {
        match fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(file) => Loaded::Parsed(file),
                Err(e) => {
                    log::warn!("run: {} is not valid: {e}", self.path.display());
                    Loaded::Unreadable
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::Missing,
            Err(e) => {
                log::warn!("run: cannot read {}: {e}", self.path.display());
                Loaded::Unreadable
            }
        }
    }

    /// The clone the user chose for `repo`, ignoring case.
    pub fn clone_for(&self, repo: &str) -> Option<PathBuf> {
        match self.load() {
            Loaded::Parsed(file) => file
                .clones
                .iter()
                .find(|(k, v)| k.eq_ignore_ascii_case(repo) && !v.trim().is_empty())
                .map(|(_, v)| PathBuf::from(v)),
            Loaded::Missing | Loaded::Unreadable => None,
        }
    }

    /// Remembers `path` as the clone of `repo`, or forgets the choice.
    pub fn set_clone(&self, repo: &str, path: Option<&Path>) -> AppResult<()> {
        let mut file = match self.load() {
            Loaded::Parsed(file) => file,
            Loaded::Missing => File::default(),
            Loaded::Unreadable => {
                return Err(AppError::Config(format!(
                    "{} could not be read, so it was left as it is. Fix or delete it and try again.",
                    self.path.display()
                )));
            }
        };
        file.clones.retain(|k, _| !k.eq_ignore_ascii_case(repo));
        if let Some(p) = path {
            file.clones
                .insert(repo.to_owned(), p.to_string_lossy().into_owned());
        }
        self.save(&file)
    }

    /// Temp file and rename, so a crash never leaves half a file.
    fn save(&self, file: &File) -> AppResult<()> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| AppError::Config("run.json has no folder".into()))?;
        let io = |e: std::io::Error| AppError::Config(format!("{}: {e}", self.path.display()));
        fs::create_dir_all(dir).map_err(io)?;
        let json = serde_json::to_string_pretty(file)?;
        let tmp = dir.join(format!(".run.json.{}.tmp", std::process::id()));
        let write = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
            fs::rename(&tmp, &self.path)
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io(e)
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn round_trip_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        let s = RunSettings::new(dir.path().join("sub/run.json"));
        assert_eq!(s.clone_for("dashpay/platform"), None);
        s.set_clone("dashpay/platform", Some(Path::new("/src/platform")))
            .unwrap();
        assert_eq!(
            s.clone_for("DashPay/Platform"),
            Some(PathBuf::from("/src/platform"))
        );
        // Setting it under another spelling replaces it.
        s.set_clone("DASHPAY/platform", Some(Path::new("/src/p2")))
            .unwrap();
        let text = fs::read_to_string(s.path()).unwrap();
        assert!(!text.contains("/src/platform\""));
        assert_eq!(
            s.clone_for("dashpay/platform"),
            Some(PathBuf::from("/src/p2"))
        );
        s.set_clone("dashpay/platform", None).unwrap();
        assert_eq!(s.clone_for("dashpay/platform"), None);
        // No temp files left behind.
        let names: Vec<_> = fs::read_dir(dir.path().join("sub"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec!["run.json"]);
    }

    #[test]
    fn unknown_fields_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.json");
        fs::write(&path, r#"{"clones":{"a/b":"/x"},"future":{"k":1}}"#).unwrap();
        let s = RunSettings::new(path.clone());
        s.set_clone("c/d", Some(Path::new("/y"))).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["future"]["k"], 1);
        assert_eq!(v["clones"]["a/b"], "/x");
        assert_eq!(v["clones"]["c/d"], "/y");
    }

    #[test]
    fn an_unparseable_file_reads_as_empty_and_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.json");
        fs::write(&path, "{ not json").unwrap();
        let s = RunSettings::new(path.clone());
        assert_eq!(s.clone_for("a/b"), None);
        let err = s.set_clone("a/b", Some(Path::new("/x"))).unwrap_err();
        assert!(err.to_string().contains("could not be read"), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
        assert!(s.set_clone("a/b", None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
