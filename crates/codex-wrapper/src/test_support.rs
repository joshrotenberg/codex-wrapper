//! Helpers for the unix-only process-lifetime tests in [`crate::exec`] and
//! [`crate::streaming`].
//!
//! Those tests assert that a spawned `codex` dies when the future driving it
//! is dropped, which means observing the process from outside the wrapper: the
//! fake codex records its PID to a file, and the test reads that PID back and
//! watches it disappear.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{Codex, CodexBuilder};

/// A file that the fake codex writes its PID to, cleaned up on drop.
pub(crate) struct PidFile {
    path: PathBuf,
}

/// Complete environment recorded by the fake Codex child, cleaned up on
/// drop. Each caller supplies a unique label because unit tests share a pid.
pub(crate) struct EnvCapture {
    path: PathBuf,
}

impl EnvCapture {
    pub(crate) fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("codex-wrapper-{}-{label}.env", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self { path }
    }

    pub(crate) fn read(&self) -> std::collections::BTreeMap<String, String> {
        let contents = std::fs::read_to_string(&self.path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", self.path.display()));
        contents
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some((key.to_string(), value.to_string()))
            })
            .collect()
    }
}

impl Drop for EnvCapture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A [`Codex`] builder backed by a fake binary that records its full
/// environment and then emits valid JSONL for streaming callers.
pub(crate) fn env_capturing_codex(capture: &EnvCapture) -> CodexBuilder {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fake-codex-capture-env.sh");
    Codex::builder()
        .binary("/bin/bash")
        .arg(script.to_str().expect("fixture path is utf-8"))
        .env(
            "CODEX_WRAPPER_ENV_CAPTURE",
            capture.path.to_str().expect("capture path is utf-8"),
        )
}

impl PidFile {
    /// Reserve a PID file path. `label` distinguishes concurrent tests, which
    /// share a process ID because cargo runs them as threads.
    pub(crate) fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("codex-wrapper-{}-{label}.pid", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Read back the PID the fake codex recorded, waiting up to a second for
    /// it to appear. Panics rather than returning an error: no PID means the
    /// fake never ran, which makes the calling test meaningless.
    pub(crate) async fn read_pid(&self) -> u32 {
        for _ in 0..100 {
            if let Ok(contents) = std::fs::read_to_string(&self.path)
                && let Ok(pid) = contents.trim().parse()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("fake codex never recorded a pid at {}", self.path.display());
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A [`Codex`] client whose binary is a fake codex that blocks until killed,
/// recording its PID to `pid_file` first.
///
/// Returned as a builder so callers can decide whether the run carries a
/// wrapper timeout or is cancelled from outside.
pub(crate) fn blocking_codex(pid_file: &PidFile) -> CodexBuilder {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fake-codex-blocks.sh");
    Codex::builder()
        .binary("/bin/bash")
        .arg(script.to_str().expect("fixture path is utf-8"))
        .env(
            "CODEX_WRAPPER_TEST_PIDFILE",
            pid_file.path().to_str().expect("pid file path is utf-8"),
        )
}

/// A process group whose descendant survives SIGTERM, with markers that let
/// tests drop the owning future only after graceful termination has started.
pub(crate) struct GracePeriodFixture {
    pid_file: PidFile,
    cleanup_armed: AtomicBool,
}

impl GracePeriodFixture {
    pub(crate) fn new(label: &str) -> Self {
        let fixture = Self {
            pid_file: PidFile::new(label),
            cleanup_armed: AtomicBool::new(true),
        };
        for suffix in [".ready", ".term"] {
            let _ = std::fs::remove_file(fixture.marker(suffix));
        }
        fixture
    }

    fn marker(&self, suffix: &str) -> PathBuf {
        let mut path = self.pid_file.path().as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    }

    pub(crate) fn configure(&self, builder: CodexBuilder) -> CodexBuilder {
        builder
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                self.pid_file.path().to_str().unwrap(),
            )
            .env("CODEX_WRAPPER_TEST_IGNORE_TERM", "1")
            // The marker, rather than a guessed delay, determines when to
            // drop. This leaves ample time even on a loaded CI worker.
            .termination_grace(Duration::from_secs(30))
    }

    pub(crate) fn builder(&self) -> CodexBuilder {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-codex-spawns-child.sh");
        self.configure(
            Codex::builder()
                .binary("/bin/bash")
                .arg(script.to_str().unwrap()),
        )
    }

    fn pids(&self) -> Option<(u32, u32)> {
        let contents = std::fs::read_to_string(self.pid_file.path()).ok()?;
        let field = |key: &str| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|value| value.parse().ok())
        };
        Some((field("parent=")?, field("child=")?))
    }

    pub(crate) async fn ready(&self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.pids().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the fixture never recorded its ready process group");
    }

    pub(crate) async fn assert_drop_during_grace(&self, run: impl std::future::Future) {
        let mut run = Box::pin(run);
        let term = self.marker(".term");
        tokio::select! {
            _ = &mut run => panic!("the run finished before its grace period was observed"),
            observed = tokio::time::timeout(Duration::from_secs(10), async {
                while !term.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }) => observed.expect("the fixture never received SIGTERM"),
        }
        let (parent, child) = self.pids().expect("the fixture recorded its process group");
        assert!(is_running(child), "the descendant must survive SIGTERM");
        drop(run);

        let (parent_gone, child_gone) =
            tokio::join!(wait_until_gone(parent), wait_until_gone(child));
        if parent_gone && child_gone {
            self.cleanup_armed.store(false, Ordering::Relaxed);
        }
        assert!(parent_gone, "the direct child ({parent}) survived the drop");
        assert!(
            child_gone,
            "the SIGTERM-ignoring descendant ({child}) survived dropping during grace"
        );
    }
}

impl Drop for GracePeriodFixture {
    fn drop(&mut self) {
        // Regression failures must not leave the deliberately stubborn
        // process behind. Also clean up when an earlier assertion panics.
        if self.cleanup_armed.load(Ordering::Relaxed)
            && let Some((parent, _)) = self.pids()
        {
            crate::exec::signal_group(parent, libc::SIGKILL);
        }
        for suffix in [".ready", ".term"] {
            let _ = std::fs::remove_file(self.marker(suffix));
        }
    }
}

/// Wait for `pid` to stop running, up to five seconds.
///
/// Checks the process state rather than `kill -0`: a killed child lingers as a
/// zombie until tokio's orphan reaper collects it, and a zombie still answers
/// `kill -0`, which would read as "still running".
pub(crate) async fn wait_until_gone(pid: u32) -> bool {
    for _ in 0..250 {
        if !is_running(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Public-in-crate view of [`is_running`], for a test that asserts a process
/// is still alive rather than waiting for it to go.
pub(crate) fn is_running_for_test(pid: u32) -> bool {
    is_running(pid)
}

/// `true` while `pid` is a live process. Empty `ps` output means the PID is
/// gone; a leading `Z` means it exited and is awaiting reaping.
fn is_running(pid: u32) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "state=", "-p", &pid.to_string()])
        .output()
        .expect("ps must be available");
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}
