//! Owned subprocesses; dropping a caller requests termination and reap.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tokio::io::AsyncReadExt;
use tokio::sync::{oneshot, watch};
use tokio::time::{Duration, timeout};

pub const WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct Completion {
    pub output: Output,
    pub cancelled: bool,
}

pub type Retirement = watch::Receiver<Option<Result<Completion, String>>>;

pub fn select_binary(external: Option<&OsStr>) -> Result<PathBuf, String> {
    let requested = external.unwrap_or_else(|| OsStr::new(env!("CARGO_BIN_EXE_guide-first-read")));
    let path = Path::new(requested)
        .canonicalize()
        .map_err(|e| format!("sample binary {requested:?}: {e}"))?;
    if !path.is_file() {
        return Err(format!("sample binary is not a file: {}", path.display()));
    }
    Ok(path)
}

#[allow(clippy::print_stderr)] // Record the exact executable selected for this acceptance run.
pub fn binary() -> PathBuf {
    let external = std::env::var_os("BACNET_FIRST_READ_BINARY");
    let path = select_binary(external.as_deref()).expect("resolve selected first-read executable");
    eprintln!(
        "first-read executable: {} (external={})",
        path.display(),
        external.is_some()
    );
    path
}

pub struct Process {
    cancel: Option<oneshot::Sender<()>>,
    retired: Retirement,
}

impl Process {
    pub fn spawn(path: &Path, args: &[&str]) -> Result<Self, String> {
        let path = path.to_owned();
        let args: Vec<_> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let (cancel, cancelled) = oneshot::channel();
        let (done, retired) = watch::channel(None);
        let (started, ready) = std::sync::mpsc::sync_channel(1);
        // A dedicated owner outlives even the caller's Tokio runtime. It drains
        // both pipes concurrently, waits for the child, drops its runtime, then
        // reports retirement. No test panic can abort its cleanup task.
        std::thread::Builder::new()
            .name("first-read-child".into())
            .spawn(move || {
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run_child(path, args, cancelled, started)),
                    Err(error) => {
                        let error = error.to_string();
                        let _ = started.send(Err(error.clone()));
                        Err(error)
                    }
                };
                done.send_replace(Some(result));
            })
            .map_err(|e| e.to_string())?;
        ready.recv().map_err(|e| e.to_string())??;
        Ok(Self {
            cancel: Some(cancel),
            retired,
        })
    }

    pub fn retirement(&self) -> Retirement {
        self.retired.clone()
    }

    pub async fn finish(mut self) -> Output {
        let completion = retired(&mut self.retired).await;
        self.cancel.take();
        assert!(!completion.cancelled, "ordinary completion was cancelled");
        completion.output
    }
}

async fn run_child(
    path: PathBuf,
    args: Vec<String>,
    mut cancellation: oneshot::Receiver<()>,
    started: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<Completion, String> {
    let mut command = tokio::process::Command::new(&path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    assert_eq!(command.as_std().get_program(), path.as_os_str());
    let mut child = match command.spawn() {
        Ok(child) => {
            let _ = started.send(Ok(()));
            child
        }
        Err(error) => {
            let error = format!("spawn {}: {error}", path.display());
            let _ = started.send(Err(error.clone()));
            return Err(error);
        }
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut watchdog = false;
    let mut cancelled = false;
    let wait = async {
        tokio::select! {
            status = child.wait() => status,
            _ = &mut cancellation => {
                cancelled = true;
                let _ = child.start_kill();
                child.wait().await
            },
            _ = tokio::time::sleep(WAIT) => {
                watchdog = true;
                let _ = child.start_kill();
                child.wait().await
            }
        }
    };
    let (status, stdout, stderr) = tokio::join!(
        wait,
        timeout(WAIT + WAIT, stdout.read_to_end(&mut out)),
        timeout(WAIT + WAIT, stderr.read_to_end(&mut err)),
    );
    let status = status.map_err(|e| format!("reap child: {e}"))?;
    stdout
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    stderr
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if watchdog {
        return Err(format!(
            "sample exceeded safety watchdog; status={status}, stdout={out:?}, stderr={err:?}"
        ));
    }
    Ok(Completion {
        output: Output {
            status,
            stdout: out,
            stderr: err,
        },
        cancelled,
    })
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

pub async fn retired(done: &mut Retirement) -> Completion {
    timeout(WAIT + WAIT + WAIT, done.wait_for(Option::is_some))
        .await
        .expect("subprocess owner must retire")
        .expect("subprocess owner must report")
        .as_ref()
        .unwrap()
        .clone()
        .expect("subprocess completion")
}
