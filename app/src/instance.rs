//! Same-user activation IPC. Authentication URLs never leave the primary process
//! through this protocol and are never written to an instance discovery file.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout, Instant},
};

#[cfg(unix)]
#[path = "instance_unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "instance_windows.rs"]
mod platform;

pub type OpenBrowser = Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;

const PROTOCOL: u8 = 1;
const DESKTOP_PROTOCOL: u8 = 2;
const MAX_MESSAGE: u64 = 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const REUSE_WAIT: Duration = Duration::from_secs(15);
const RETRY_DELAY: Duration = Duration::from_millis(100);
const CONTACT_HELP: &str = "同一处理记录目录已有实例，但无法联系新版工作台。可能是旧版程序、原程序仍未就绪或本地通信被拦截。请切回原服务窗口，等待当前任务完成后按 Ctrl+C 正常退出，再双击新版启动脚本；若原窗口已关闭，请先确认旧程序已经退出。请保留处理记录，不要删除锁文件。";
const BROWSER_HELP: &str = "已有工作台仍在运行，但浏览器打开失败。请检查 Windows 默认浏览器设置，或切回原服务窗口复制其完整地址手动打开，再次双击也可重试；正在处理的任务不会因此停止。";

#[derive(Clone)]
enum Readiness {
    Starting,
    Ready(Arc<str>),
    Closing,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u8,
    command: Command,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Command {
    Open,
    Ping,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Starting,
    Opened,
    Ready,
    Closing,
    BrowserFailed,
    Unsupported,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Response {
    protocol: u8,
    status: Status,
}

pub enum Entry {
    Primary(PrimaryInstance),
    Reused,
}

pub struct PrimaryInstance {
    // Keep the file open until the listener and its requests have stopped.
    _lock: File,
    readiness: watch::Sender<Readiness>,
    listener: Option<JoinHandle<()>>,
    stop: Option<oneshot::Sender<()>>,
}

impl PrimaryInstance {
    pub fn ready(&self, url: String) {
        self.readiness.send_replace(Readiness::Ready(url.into()));
    }

    pub fn closing(&self) {
        self.readiness.send_replace(Readiness::Closing);
    }

    pub fn closing_notifier(&self) -> impl Fn() + Send + Sync + 'static {
        let sender = self.readiness.clone();
        move || {
            sender.send_replace(Readiness::Closing);
        }
    }

    /// Call on every graceful return path, including startup errors. Waiting
    /// before releasing the file lock prevents a new primary from racing an
    /// old Windows pipe handle that a pending request has not dropped yet.
    pub async fn close(mut self) {
        self.closing();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.listener.take() {
            let _ = task.await;
        }
    }
}

impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        self.closing();
        if let Some(task) = self.listener.take() {
            task.abort();
        }
    }
}

/// Claim the existing record directory or activate its current primary. A lock
/// contention is the only condition that attempts activation; permission and
/// filesystem errors never masquerade as an already-running instance.
pub async fn enter(state_dir: &Path, reopen: bool, opener: OpenBrowser) -> Result<Entry> {
    enter_with_protocol(state_dir, reopen, opener, PROTOCOL).await
}

/// Desktop activations must never ask a legacy browser service to open a page.
/// Protocol separation preserves the same state lock and protected endpoint.
pub async fn enter_desktop(state_dir: &Path, reopen: bool, activate: OpenBrowser) -> Result<Entry> {
    enter_with_protocol(state_dir, reopen, activate, DESKTOP_PROTOCOL).await
}

async fn enter_with_protocol(
    state_dir: &Path,
    reopen: bool,
    opener: OpenBrowser,
    protocol: u8,
) -> Result<Entry> {
    fs::create_dir_all(state_dir).with_context(|| {
        format!(
            "处理记录目录无法创建或没有写入权限：{}。请确认磁盘已连接，并由当前账号检查该目录的读写权限后重试；不会自动改用其他记录目录",
            state_dir.display()
        )
    })?;
    let directory = state_dir.canonicalize().with_context(|| {
        format!(
            "处理记录目录无法访问：{}。请检查路径和当前账号权限后重试",
            state_dir.display()
        )
    })?;
    let lock_path = directory.join("instance.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("处理记录目录中的锁文件是符号链接，无法安全启动。请核对记录目录来源并保留现有文件，不会自动删除锁文件");
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| {
            format!(
                "处理记录目录无法写入锁文件：{}。请检查当前账号的目录读写权限、磁盘状态或安全软件拦截后重试；不会删除原锁文件",
                directory.display()
            )
        })?;
    let endpoint = platform::Endpoint::for_state(&directory).context("无法初始化仅当前用户可访问的本地通信位置。请检查当前账号的临时目录权限和安全软件设置后重试；不会放宽访问权限")?;
    if let Err(error) = fs2::FileExt::try_lock_exclusive(&lock) {
        let contended = error.kind() == std::io::ErrorKind::WouldBlock
            || error.raw_os_error() == fs2::lock_contended_error().raw_os_error();
        if !contended {
            return Err(error).context("无法锁定处理记录目录。请检查当前账号的目录权限和磁盘状态后重试；不会自动结束旧程序或删除锁文件");
        }
        println!("检测到同一记录目录已有实例，正在联系已有工作台…");
        notify_existing(&endpoint, reopen, REUSE_WAIT, protocol).await?;
        if reopen {
            println!("已打开正在运行的工作台；原任务继续运行。");
        } else {
            println!("已连接正在运行的工作台；原任务继续运行。");
        }
        return Ok(Entry::Reused);
    }
    let listener = endpoint.bind().context("无法创建仅当前用户可访问的本地通信通道。请确认同一记录目录没有旧程序正在退出，并检查当前账号权限或安全软件后重试；处理记录不会被删除")?;
    let (sender, receiver) = watch::channel(Readiness::Starting);
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(listen(listener, receiver, opener, stopped, protocol));
    Ok(Entry::Primary(PrimaryInstance {
        _lock: lock,
        readiness: sender,
        listener: Some(task),
        stop: Some(stop),
    }))
}

async fn notify_existing(
    endpoint: &platform::Endpoint,
    reopen: bool,
    wait: Duration,
    protocol: u8,
) -> Result<()> {
    let deadline = Instant::now() + wait;
    let mut reported_starting = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(CONTACT_HELP);
        }
        let exchange = async {
            let mut connection = endpoint.connect().await?;
            write_json(
                &mut connection,
                &Request {
                    protocol,
                    command: if reopen { Command::Open } else { Command::Ping },
                },
            )
            .await?;
            read_json::<_, Response>(&mut connection).await
        };
        match timeout(remaining.min(REQUEST_TIMEOUT + OPEN_TIMEOUT), exchange).await {
            Ok(Ok(response)) if response.protocol != protocol => {
                bail!("同一处理记录目录已有不同界面版本的工作台，无法安全复用。请先在原网页版服务窗口等待任务完成后按 Ctrl+C 退出，或从原桌面窗口正常退出，再启动 PDF工作台；记录保留，不要删除锁文件");
            }
            Ok(Ok(response)) => match response.status {
                Status::Opened | Status::Ready => return Ok(()),
                Status::Starting => {
                    if !reported_starting {
                        println!("原工作台正在启动，正在等待就绪…");
                        reported_starting = true;
                    }
                }
                Status::BrowserFailed => {
                    if protocol == DESKTOP_PROTOCOL {
                        bail!("原PDF工作台仍在运行，但窗口唤醒失败。请切回原窗口或任务栏，等待任务完成后正常退出并重试；不会自动结束原进程");
                    }
                    bail!(BROWSER_HELP)
                }
                Status::Closing => {
                    bail!("原工作台正在正常退出。请等待原服务窗口结束后再双击启动，处理记录会保留；不要删除锁文件")
                }
                Status::Unsupported => bail!(CONTACT_HELP),
            },
            // An old process holds the lock but exposes no activation endpoint.
            // A primary that has only just acquired the lock may also need a
            // short moment to create its endpoint, so retry until the bound.
            _ => {}
        }
        sleep(RETRY_DELAY.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

async fn listen(
    mut listener: platform::Listener,
    readiness: watch::Receiver<Readiness>,
    opener: OpenBrowser,
    mut stopped: oneshot::Receiver<()>,
    protocol: u8,
) {
    let mut requests = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            incoming = listener.accept(), if requests.len() < 8 => {
                match incoming {
                    Ok(connection) => {
                        let readiness = readiness.clone();
                        let opener = opener.clone();
                        requests.spawn(async move {
                            let _ = serve(connection, readiness, opener, protocol).await;
                        });
                    }
                    Err(_) => {
                        eprintln!("本地重复启动通信暂不可用；现有任务继续运行。请使用原服务窗口中的地址，结束后按 Ctrl+C 正常退出再重试");
                        break;
                    }
                }
            }
            _ = requests.join_next(), if !requests.is_empty() => {}
        }
    }
    // Await every aborted request before closing the listener and releasing the
    // record lock: a connected Windows pipe is also a server instance handle.
    requests.shutdown().await;
}

async fn serve<S>(
    mut connection: S,
    readiness: watch::Receiver<Readiness>,
    opener: OpenBrowser,
    protocol: u8,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let request = timeout(REQUEST_TIMEOUT, read_json::<_, Request>(&mut connection)).await;
    let request = match request {
        Ok(Ok(request)) if request.protocol == protocol => request,
        _ => {
            write_json(
                &mut connection,
                &Response {
                    protocol,
                    status: Status::Unsupported,
                },
            )
            .await?;
            return Ok(());
        }
    };
    let current = readiness.borrow().clone();
    let status = match current {
        Readiness::Starting => Status::Starting,
        Readiness::Closing => Status::Closing,
        Readiness::Ready(url) => match request.command {
            Command::Ping => Status::Ready,
            Command::Open => {
                // Do not include the URL, token, or opener's raw error in the
                // response: third-party error strings can include argv/URLs.
                let opened = timeout(
                    OPEN_TIMEOUT,
                    tokio::task::spawn_blocking(move || opener(&url)),
                )
                .await;
                match opened {
                    Ok(Ok(Ok(()))) => Status::Opened,
                    _ => {
                        if protocol == DESKTOP_PROTOCOL {
                            eprintln!("PDF工作台窗口唤醒失败，任务继续运行；请从任务栏切回原窗口");
                        } else {
                            eprintln!("{BROWSER_HELP}");
                        }
                        Status::BrowserFailed
                    }
                }
            }
        },
    };
    write_json(&mut connection, &Response { protocol, status }).await
}

async fn read_json<S: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    stream: &mut S,
) -> Result<T> {
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(stream.take(MAX_MESSAGE + 1));
    reader.read_until(b'\n', &mut bytes).await?;
    if bytes.len() as u64 > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
        bail!("本地通信消息无效");
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn write_json<S: AsyncWrite + Unpin, T: Serialize>(stream: &mut S, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    timeout(REQUEST_TIMEOUT, stream.write_all(&bytes)).await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn opener(counter: Arc<AtomicUsize>) -> OpenBrowser {
        Arc::new(move |url| {
            assert_eq!(url, "http://127.0.0.1:1234/#token=never-in-protocol");
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    async fn create_primary(directory: &Path, opener: OpenBrowser) -> PrimaryInstance {
        match enter(directory, false, opener).await.unwrap() {
            Entry::Primary(primary) => primary,
            Entry::Reused => panic!("expected primary"),
        }
    }

    #[tokio::test]
    async fn existing_primary_opens_in_its_own_callback_and_never_returns_token() {
        let temp = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let primary = create_primary(temp.path(), opener(count.clone())).await;
        primary.ready("http://127.0.0.1:1234/#token=never-in-protocol".into());
        let endpoint = platform::Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        let mut connection = endpoint.connect().await.unwrap();
        write_json(
            &mut connection,
            &Request {
                protocol: PROTOCOL,
                command: Command::Open,
            },
        )
        .await
        .unwrap();
        let response: Response = read_json(&mut connection).await.unwrap();
        assert_eq!(response.status, Status::Opened);
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(!encoded.contains("token") && !encoded.contains("127.0.0.1"));
        assert!(matches!(
            enter(
                temp.path(),
                false,
                Arc::new(|_| panic!("secondary opener used"))
            )
            .await
            .unwrap(),
            Entry::Reused
        ));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        primary.close().await;
        assert!(temp.path().join("instance.lock").exists());
    }

    #[tokio::test]
    async fn requests_wait_for_startup_then_reuse_and_can_restart_after_close() {
        let temp = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let primary = create_primary(temp.path(), opener(count.clone())).await;
        let endpoint = platform::Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        let waiting = tokio::spawn(async move {
            notify_existing(&endpoint, true, Duration::from_secs(2), PROTOCOL).await
        });
        sleep(Duration::from_millis(150)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        primary.ready("http://127.0.0.1:1234/#token=never-in-protocol".into());
        waiting.await.unwrap().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        primary.close().await;
        let restarted = create_primary(temp.path(), opener(count)).await;
        restarted.close().await;
    }

    #[tokio::test]
    async fn browser_failure_and_closing_do_not_open_from_the_new_process() {
        let temp = tempfile::tempdir().unwrap();
        let primary = create_primary(
            temp.path(),
            Arc::new(|_| bail!("private token=must-not-be-returned")),
        )
        .await;
        primary.ready("http://127.0.0.1:1234/#token=never-in-protocol".into());
        let endpoint = platform::Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        let failure = notify_existing(&endpoint, true, Duration::from_secs(1), PROTOCOL)
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("浏览器") && !failure.contains("must-not-be-returned"));
        primary.closing_notifier()();
        let failure = notify_existing(&endpoint, true, Duration::from_secs(1), PROTOCOL)
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("正常退出"));
        primary.close().await;
    }

    #[tokio::test]
    async fn old_or_unreachable_holder_fails_without_deleting_its_lock() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("instance.lock");
        let lock = File::create(&path).unwrap();
        fs2::FileExt::try_lock_exclusive(&lock).unwrap();
        let endpoint = platform::Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        let failure = notify_existing(&endpoint, true, Duration::from_millis(40), PROTOCOL)
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("旧版") && failure.contains("Ctrl+C"));
        assert!(path.exists());
        drop(lock);
    }

    #[tokio::test]
    async fn an_unlocked_directory_never_contacts_a_preexisting_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = platform::Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        let mut previous = endpoint.bind().unwrap();
        let result = enter(temp.path(), true, Arc::new(|_| panic!("unexpected opener"))).await;
        // Unix can safely replace its owned stale socket while holding the
        // record lock. Windows must reject an already-owned first pipe name.
        #[cfg(unix)]
        let primary = match result.unwrap() {
            Entry::Primary(primary) => primary,
            Entry::Reused => panic!("unlocked records must never reuse an endpoint"),
        };
        #[cfg(windows)]
        assert!(result.err().unwrap().to_string().contains("本地通信通道"));
        assert!(timeout(Duration::from_millis(40), previous.accept())
            .await
            .is_err());
        #[cfg(unix)]
        primary.close().await;
    }

    #[tokio::test]
    async fn oversized_or_command_injected_messages_cannot_activate_a_browser() {
        let temp = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let primary = create_primary(temp.path(), opener(count.clone())).await;
        primary.ready("http://127.0.0.1:1234/#token=never-in-protocol".into());
        let endpoint = platform::Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        for payload in [
            vec![b'x'; MAX_MESSAGE as usize + 2],
            b"{\"protocol\":1,\"command\":\"open\",\"url\":\"file:///not-allowed\"}\n".to_vec(),
        ] {
            let mut connection = endpoint.connect().await.unwrap();
            connection.write_all(&payload).await.unwrap();
            let response: Response = read_json(&mut connection).await.unwrap();
            assert_eq!(response.status, Status::Unsupported);
        }
        assert_eq!(count.load(Ordering::SeqCst), 0);
        primary.close().await;
    }

    #[tokio::test]
    async fn desktop_and_browser_instances_never_reuse_each_others_activation() {
        let temp = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let browser = create_primary(temp.path(), opener(count.clone())).await;
        browser.ready("http://127.0.0.1:1234/#token=never-in-protocol".into());
        let error = enter_desktop(temp.path(), true, opener(count.clone()))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("不同界面版本") && error.contains("Ctrl+C"));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        browser.close().await;
        let desktop = match enter_desktop(temp.path(), false, opener(count.clone()))
            .await
            .unwrap()
        {
            Entry::Primary(primary) => primary,
            Entry::Reused => panic!("expected primary"),
        };
        desktop.ready("http://127.0.0.1:1234/#token=never-in-protocol".into());
        assert!(enter(temp.path(), true, opener(count.clone()))
            .await
            .is_err());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(matches!(
            enter_desktop(temp.path(), true, opener(count.clone()))
                .await
                .unwrap(),
            Entry::Reused
        ));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        desktop.close().await;
    }

    #[tokio::test]
    async fn record_directory_errors_are_not_reported_as_existing_instances() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("not-a-directory");
        fs::write(&file, b"keep this file").unwrap();
        let failure = enter(&file, true, Arc::new(|_| panic!("unexpected opener")))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(failure.contains("处理记录目录") && !failure.contains("已有实例"));
        assert_eq!(fs::read(file).unwrap(), b"keep this file");
    }
}
