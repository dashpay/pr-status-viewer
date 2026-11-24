//! Starting processes: the one seam between the run engine and the
//! operating system. The app uses [`RealHost`]; the engine's tests use a
//! scripted fake, so they never start a real build, simulator or emulator.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::watch;

use crate::error::{AppError, AppResult};

/// How long a cancelled command's process group gets to stop after TERM
/// before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// One command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// The complete environment. `None` inherits the app's own.
    pub env: Option<Arc<HashMap<String, String>>>,
    /// Set on top of `env`.
    pub vars: Vec<(String, String)>,
}

impl Cmd {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: None,
            vars: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, env: Arc<HashMap<String, String>>) -> Self {
        self.env = Some(env);
        self
    }

    pub fn var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.push((key.into(), value.into()));
        self
    }

    /// The program's file name, for messages: `./build_ios.sh`, `xcrun`.
    pub fn short_name(&self) -> String {
        let name = Path::new(&self.program)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.program.clone());
        if self.program.starts_with("./") {
            format!("./{name}")
        } else {
            name
        }
    }
}

/// The command line as it is written in the log. Never includes the
/// environment, which can hold tokens.
impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", quote(&self.program))?;
        for arg in &self.args {
            write!(f, " {}", quote(arg))?;
        }
        Ok(())
    }
}

fn quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@+%".contains(c));
    if plain {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// How a streamed command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    /// Ended by a signal it did not get from us.
    Signal(i32),
    /// Stopped because the job was cancelled.
    Cancelled,
    /// Could not be started; the reason as a sentence fragment.
    NotStarted(String),
}

/// What a short query printed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    pub fn ok(stdout: &str) -> Self {
        Self {
            code: Some(0),
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Receives each line a command prints, stdout and stderr alike.
#[derive(Clone)]
pub struct LineSink(Arc<dyn Fn(String) + Send + Sync>);

impl LineSink {
    pub fn new(f: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    pub fn push(&self, line: String) {
        (self.0)(line)
    }
}

/// Becomes `true` once when the job is cancelled.
pub type CancelSignal = watch::Receiver<bool>;

/// Resolves when `cancel` is (or already was) set. Never resolves if the
/// sender is gone without cancelling.
pub async fn cancelled(cancel: &mut CancelSignal) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

pub trait Host: Send + Sync + 'static {
    /// Runs `cmd` to the end, sending every output line to `out`. When
    /// `cancel` fires, stops the command's whole process tree and returns
    /// `Exit::Cancelled`.
    fn run(
        &self,
        cmd: Cmd,
        out: LineSink,
        cancel: CancelSignal,
    ) -> impl Future<Output = Exit> + Send;

    /// Runs a short query and captures what it prints. Fails when it cannot
    /// start or takes longer than `timeout`.
    fn output(
        &self,
        cmd: Cmd,
        timeout: Duration,
    ) -> impl Future<Output = AppResult<Captured>> + Send;

    /// Starts a program that outlives the job and the app (an emulator).
    /// Its output goes to `log`, or nowhere.
    fn spawn_detached(&self, cmd: Cmd, log: Option<PathBuf>) -> AppResult<()>;
}

// ---- The real host ----

/// Runs commands on this machine, each streamed one in its own process
/// group so a cancel reaches every child (cargo's rustc, xcodebuild's
/// compilers, Gradle's workers).
#[derive(Debug, Clone, Copy, Default)]
pub struct RealHost;

fn command(cmd: &Cmd) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(&cmd.program);
    c.args(&cmd.args).stdin(Stdio::null());
    if let Some(dir) = &cmd.cwd {
        c.current_dir(dir);
    }
    if let Some(env) = &cmd.env {
        c.env_clear().envs(env.iter());
    }
    c.envs(cmd.vars.iter().map(|(k, v)| (k, v)));
    c
}

fn start_error(cmd: &Cmd, e: &std::io::Error) -> String {
    let why = match e.kind() {
        std::io::ErrorKind::NotFound => "it was not found".to_owned(),
        std::io::ErrorKind::PermissionDenied => "permission was denied".to_owned(),
        _ => e.to_string(),
    };
    format!("Could not start {}: {why}.", cmd.short_name())
}

impl Host for RealHost {
    async fn run(&self, cmd: Cmd, out: LineSink, mut cancel: CancelSignal) -> Exit {
        let mut c = command(&cmd);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        c.process_group(0);
        // A dropped future (the app quitting mid-step) still stops the leader.
        c.kill_on_drop(true);
        let mut child = match c.spawn() {
            Ok(child) => child,
            Err(e) => return Exit::NotStarted(start_error(&cmd, &e)),
        };
        let pgid = child.id();
        let readers = {
            let a = child.stdout.take().map(|s| pump(s, out.clone()));
            let b = child.stderr.take().map(|s| pump(s, out.clone()));
            tokio::spawn(async move {
                if let Some(a) = a {
                    let _ = a.await;
                }
                if let Some(b) = b {
                    let _ = b.await;
                }
            })
        };

        let status = tokio::select! {
            status = child.wait() => Some(status),
            () = cancelled(&mut cancel) => None,
        };
        let exit = match status {
            Some(Ok(status)) => exit_of(status),
            Some(Err(e)) => Exit::NotStarted(format!("Lost track of {}: {e}.", cmd.short_name())),
            None => {
                stop_group(&mut child, pgid).await;
                Exit::Cancelled
            }
        };
        // Output still in the pipes; a grandchild holding them open must not
        // hold up the job.
        let _ = tokio::time::timeout(Duration::from_secs(2), readers).await;
        exit
    }

    async fn output(&self, cmd: Cmd, timeout: Duration) -> AppResult<Captured> {
        let mut c = command(&cmd);
        c.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let fut = c.output();
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(o)) => Ok(Captured {
                code: o.status.code(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            }),
            Ok(Err(e)) => Err(AppError::Other(start_error(&cmd, &e))),
            Err(_) => Err(AppError::Other(format!(
                "{} did not answer in time.",
                cmd.short_name()
            ))),
        }
    }

    fn spawn_detached(&self, cmd: Cmd, log: Option<PathBuf>) -> AppResult<()> {
        let mut c = std::process::Command::new(&cmd.program);
        c.args(&cmd.args).stdin(Stdio::null());
        if let Some(dir) = &cmd.cwd {
            c.current_dir(dir);
        }
        if let Some(env) = &cmd.env {
            c.env_clear().envs(env.iter());
        }
        c.envs(cmd.vars.iter().map(|(k, v)| (k, v)));
        let file = log.and_then(|p| std::fs::File::create(p).ok());
        match file.as_ref().and_then(|f| f.try_clone().ok()) {
            Some(f2) => {
                c.stdout(Stdio::from(f2));
            }
            None => {
                c.stdout(Stdio::null());
            }
        }
        match file {
            Some(f) => {
                c.stderr(Stdio::from(f));
            }
            None => {
                c.stderr(Stdio::null());
            }
        }
        // Its own process group: a cancel or the app quitting never reaches it.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        let child = c
            .spawn()
            .map_err(|e| AppError::Other(start_error(&cmd, &e)))?;
        // Reap it whenever it ends, so it never lingers as a zombie while the
        // app runs.
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        Ok(())
    }
}

/// Reads one pipe and sends each line (split on `\n` and `\r`, so progress
/// bars do not pile up) to `out`.
fn pump<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    out: LineSink,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf);
                    let text = text.trim_end_matches(['\n', '\r']);
                    // A carriage return redraws the line: keep what is last.
                    let last = text.rsplit('\r').find(|s| !s.trim().is_empty());
                    out.push(last.unwrap_or("").to_owned());
                }
            }
        }
    })
}

#[cfg(unix)]
fn exit_of(status: std::process::ExitStatus) -> Exit {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => Exit::Code(code),
        (None, Some(sig)) => Exit::Signal(sig),
        (None, None) => Exit::Code(-1),
    }
}

#[cfg(not(unix))]
fn exit_of(status: std::process::ExitStatus) -> Exit {
    Exit::Code(status.code().unwrap_or(-1))
}

/// TERM to the whole group, KILL to whatever is left after the grace period.
#[cfg(unix)]
async fn stop_group(child: &mut tokio::process::Child, pgid: Option<u32>) {
    let Some(pgid) = pgid.and_then(|p| libc::pid_t::try_from(p).ok()) else {
        let _ = child.kill().await;
        return;
    };
    signal_group(pgid, libc::SIGTERM);
    let deadline = tokio::time::Instant::now() + STOP_GRACE;
    loop {
        // Reap the leader so a zombie does not keep the group alive.
        let _ = child.try_wait();
        if !group_alive(pgid) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    signal_group(pgid, libc::SIGKILL);
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
}

#[cfg(not(unix))]
async fn stop_group(child: &mut tokio::process::Child, _pgid: Option<u32>) {
    let _ = child.kill().await;
}

#[cfg(unix)]
fn signal_group(pgid: libc::pid_t, signal: libc::c_int) {
    // SAFETY: `kill` takes plain integers and has no memory effects. A
    // negative pid addresses the process group we created for this command.
    unsafe {
        libc::kill(-pgid, signal);
    }
}

#[cfg(unix)]
fn group_alive(pgid: libc::pid_t) -> bool {
    // SAFETY: as above; signal 0 only checks that the group exists.
    unsafe { libc::kill(-pgid, 0) == 0 }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn collect() -> (LineSink, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l = Arc::clone(&lines);
        (LineSink::new(move |s| l.lock().unwrap().push(s)), lines)
    }

    #[test]
    fn display_quotes_only_what_needs_it() {
        let cmd = Cmd::new("xcodebuild")
            .args(["-destination", "platform=iOS Simulator,id=ABC"])
            .arg("it's");
        assert_eq!(
            cmd.to_string(),
            r"xcodebuild -destination 'platform=iOS Simulator,id=ABC' 'it'\''s'"
        );
        assert_eq!(Cmd::new("./build_ios.sh").short_name(), "./build_ios.sh");
        assert_eq!(Cmd::new("/usr/bin/xcrun").short_name(), "xcrun");
    }

    #[tokio::test]
    async fn run_streams_lines_and_keeps_the_last_redraw() {
        let (sink, lines) = collect();
        let (_tx, rx) = watch::channel(false);
        let cmd = Cmd::new("/bin/sh").args([
            "-c",
            "printf 'a\\nb 10%%\\rb 100%%\\n'; echo err >&2; exit 3",
        ]);
        let exit = RealHost.run(cmd, sink, rx).await;
        assert_eq!(exit, Exit::Code(3));
        let mut got = lines.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, vec!["a", "b 100%", "err"]);
    }

    #[tokio::test]
    async fn run_reports_a_missing_program_as_a_sentence() {
        let (sink, _) = collect();
        let (_tx, rx) = watch::channel(false);
        let exit = RealHost.run(Cmd::new("/no/such/tool"), sink, rx).await;
        assert_eq!(
            exit,
            Exit::NotStarted("Could not start tool: it was not found.".into())
        );
    }

    #[tokio::test]
    async fn cancel_stops_the_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let (sink, _) = collect();
        let (tx, rx) = watch::channel(false);
        // The grandchild ignores nothing but runs in the background of the
        // shell, like rustc under cargo.
        let script = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
        let cmd = Cmd::new("/bin/sh").args(["-c", &script]);
        let run = tokio::spawn(async move { RealHost.run(cmd, sink, rx).await });
        for _ in 0..50 {
            if pidfile.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        tx.send(true).unwrap();
        assert_eq!(run.await.unwrap(), Exit::Cancelled);
        // The grandchild is gone (or a zombie awaiting init's reaping).
        let mut gone = false;
        for _ in 0..40 {
            // SAFETY: signal 0 only checks existence.
            if unsafe { libc::kill(pid, 0) } != 0 {
                gone = true;
                break;
            }
            let out = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            if String::from_utf8_lossy(&out.stdout).trim().starts_with('Z') {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(gone, "grandchild {pid} survived the cancel");
    }

    #[tokio::test]
    async fn output_times_out_with_a_sentence() {
        let err = RealHost
            .output(Cmd::new("/bin/sleep").arg("5"), Duration::from_millis(100))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "sleep did not answer in time.");
    }
}
