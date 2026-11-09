//! Running the official vanilla server: world copy, startup, clean stop.
//!
//! The harness is deliberately dumb: it gives each test run a fresh world
//! copied from the frozen template, waits for vanilla's own `Done (...)`
//! readiness line in `logs/latest.log`, and stops the server by piping
//! `stop` to stdin (a graceful shutdown that persists the world).

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

use crate::env::ParityEnv;

/// How long the vanilla server may take to print `Done`.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a graceful `stop` may take.
const STOP_TIMEOUT: Duration = Duration::from_secs(60);
/// Interval between readiness polls of the server log.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Errors of the vanilla server harness.
#[derive(Debug, thiserror::Error)]
pub enum VanillaError {
    /// The previous run's world could not be removed.
    #[error("cannot remove the old world {path}: {source}")]
    WorldRemove {
        /// World directory that stayed behind.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The template world could not be copied for this run.
    #[error("cannot copy the world template to {path}: {source}")]
    WorldCopy {
        /// Destination world directory.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The old readiness log could not be cleared.
    #[error("cannot remove the previous vanilla log {path}: {source}")]
    LogRemove {
        /// Previous log file.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The java process could not be started.
    #[error("cannot start the vanilla server ({jar}): {source}")]
    Spawn {
        /// Jar path passed to java.
        jar: PathBuf,
        /// Underlying spawn error (usually a missing `java`).
        #[source]
        source: std::io::Error,
    },
    /// The server died while starting up instead of reporting readiness.
    #[error("the vanilla server exited during startup (code {code:?})")]
    ExitedEarly {
        /// Exit code, if any.
        code: Option<i32>,
    },
    /// The server never printed `Done` in time.
    #[error("the vanilla server did not become ready within {0:?}")]
    StartupTimeout(Duration),
    /// Reading the log went wrong (a missing file is just "not yet").
    #[error("cannot read the vanilla log {path}: {source}")]
    LogRead {
        /// Log that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The stop command could not be written to stdin.
    #[error("cannot send `stop` to the vanilla server: {0}")]
    StopWrite(#[source] std::io::Error),
    /// A console command (other than `stop`) could not be written.
    #[error("cannot send a console command to the vanilla server: {0}")]
    CommandWrite(#[source] std::io::Error),
    /// The server ignored `stop` for too long.
    #[error("the vanilla server did not stop within {0:?}")]
    StopTimeout(Duration),
    /// The server exited with a failure code.
    #[error("the vanilla server exited with code {code:?} after `stop`")]
    StopFailed {
        /// Exit code reported by the process.
        code: Option<i32>,
    },
}

/// A running vanilla server process.
#[derive(Debug)]
pub struct Vanilla {
    child: Child,
    vanilla_dir: PathBuf,
    log_path: PathBuf,
}

impl Vanilla {
    /// Prepares a fresh world and starts the vanilla server.
    ///
    /// Startup blocks (with retries reading the log) until vanilla writes
    /// `Done (...)`, so on success the server is fully ready to accept
    /// connections.
    pub async fn start(env: &ParityEnv) -> Result<Self, VanillaError> {
        let vanilla_dir = env.vanilla_dir.clone();
        prepare_world(env)?;

        let log_path = vanilla_dir.join("logs").join("latest.log");
        if log_path.exists() {
            // Old `Done` lines would fake readiness of this run.
            std::fs::remove_file(&log_path).map_err(|source| VanillaError::LogRemove {
                path: log_path.clone(),
                source,
            })?;
        }

        let mut child = Command::new("java")
            .args(["-Xmx1G", "-jar"])
            .arg(&env.vanilla_jar)
            .arg("--nogui")
            .current_dir(&vanilla_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| VanillaError::Spawn {
                jar: env.vanilla_jar.clone(),
                source,
            })?;

        wait_until_ready(&mut child, &log_path).await?;

        tracing::info!(dir = %vanilla_dir.display(), "vanilla server ready");
        Ok(Self {
            child,
            vanilla_dir,
            log_path,
        })
    }

    /// Stops the server with `stop` on stdin and waits for a clean exit.
    pub async fn stop(mut self) -> Result<(), VanillaError> {
        self.stop_inner().await
    }

    /// Stops the server the way the walk runs need it: `save-all flush`
    /// first (the world is compared file-by-file afterwards), a short
    /// pause for the save to land, then `stop`.
    pub async fn stop_with_flush(mut self) -> Result<(), VanillaError> {
        if let Some(stdin) = self.child.stdin.as_mut() {
            stdin
                .write_all(b"save-all flush\n")
                .await
                .map_err(VanillaError::StopWrite)?;
            stdin.flush().await.map_err(VanillaError::StopWrite)?;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.stop_inner().await
    }

    /// Writes one console command (`say`, `data get`, …) to stdin without
    /// stopping the server; the command's own output lands in the log.
    pub async fn command(&mut self, line: &str) -> Result<(), VanillaError> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| VanillaError::CommandWrite(std::io::Error::other("stdin closed")))?;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(VanillaError::CommandWrite)?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(VanillaError::CommandWrite)?;
        stdin.flush().await.map_err(VanillaError::CommandWrite)?;
        Ok(())
    }

    /// Writes one console command and waits for the process to exit.
    async fn stop_inner(&mut self) -> Result<(), VanillaError> {
        if let Some(stdin) = self.child.stdin.as_mut() {
            stdin
                .write_all(b"stop\n")
                .await
                .map_err(VanillaError::StopWrite)?;
            stdin.flush().await.map_err(VanillaError::StopWrite)?;
        }

        match tokio::time::timeout(STOP_TIMEOUT, self.child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    tracing::info!(
                        dir = %self.vanilla_dir.display(),
                        "vanilla server stopped"
                    );
                    Ok(())
                } else {
                    Err(VanillaError::StopFailed {
                        code: status.code(),
                    })
                }
            }
            Ok(Err(source)) => Err(VanillaError::StopWrite(source)),
            Err(_) => {
                let _ = self.child.kill().await;
                Err(VanillaError::StopTimeout(STOP_TIMEOUT))
            }
        }
    }

    /// Path of the server's own log, for troubleshooting failed runs.
    pub fn log_path(&self) -> &std::path::Path {
        &self.log_path
    }
}

impl Drop for Vanilla {
    fn drop(&mut self) {
        // A forgotten test process must not leave a listening server behind;
        // `kill_on_drop` also covers the case where the child guard fired.
        let _ = self.child.start_kill();
    }
}

/// Replaces `vanilla_dir/world` with a copy of the frozen world template.
fn prepare_world(env: &ParityEnv) -> Result<(), VanillaError> {
    let world = env.vanilla_dir.join("world");
    if world.exists() {
        std::fs::remove_dir_all(&world).map_err(|source| VanillaError::WorldRemove {
            path: world.clone(),
            source,
        })?;
    }
    copy_dir_all(&env.world_template, &world).map_err(|source| VanillaError::WorldCopy {
        path: world,
        source,
    })
}

/// Copies a directory tree recursively (std has no directory copy).
///
/// The template is a frozen world directory made of regular files.
pub fn copy_dir_all(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Polls the log until vanilla prints `Done`, or fails fast if java dies.
async fn wait_until_ready(
    child: &mut Child,
    log_path: &std::path::Path,
) -> Result<(), VanillaError> {
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().map_err(|source| VanillaError::LogRead {
            path: log_path.to_path_buf(),
            source,
        })? {
            return Err(VanillaError::ExitedEarly {
                code: status.code(),
            });
        }
        match std::fs::read_to_string(log_path) {
            Ok(text) if text.contains("Done (") => return Ok(()),
            Ok(_) => {} // log exists but is not finished yet
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(VanillaError::LogRead {
                    path: log_path.to_path_buf(),
                    source,
                });
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(VanillaError::StartupTimeout(STARTUP_TIMEOUT));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory under the system temp folder.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("minerust-parity-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn copy_dir_all_copies_nested_trees() {
        let from = scratch_dir("copy-from");
        let to = scratch_dir("copy-to");
        std::fs::create_dir_all(from.join("a").join("b")).expect("create source tree");
        std::fs::write(from.join("level.dat"), b"root").expect("write root file");
        std::fs::write(from.join("a").join("b").join("chunk.mca"), b"chunk").expect("write leaf");

        copy_dir_all(&from, &to).expect("copy succeeds");
        assert_eq!(
            std::fs::read(to.join("level.dat")).expect("root copy"),
            b"root"
        );
        assert_eq!(
            std::fs::read(to.join("a").join("b").join("chunk.mca")).expect("leaf copy"),
            b"chunk"
        );

        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn copy_dir_all_missing_source_is_an_error() {
        let missing = scratch_dir("no-such-template");
        let to = scratch_dir("copy-missing-target");
        let error = copy_dir_all(&missing, &to).expect_err("missing source fails");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        let _ = std::fs::remove_dir_all(&to);
    }
}
