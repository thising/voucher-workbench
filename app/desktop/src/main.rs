#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Serialize;
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, Url, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, FilePath, MessageDialogButtons, MessageDialogKind};
use tokio::sync::{oneshot, watch};
use voucher_workbench::{
    instance::{self, Entry, OpenBrowser},
    jobs::AppState,
    launcher::{LaunchPaths, RuntimeDiagnostic, StartupOptions},
};

#[derive(Clone, Debug, Parser)]
#[command(version, about = "PDF工作台：本地批量整理与合并")]
struct Args {
    #[arg(long)]
    state_dir: Option<PathBuf>,
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long)]
    runtime: Option<PathBuf>,
    #[arg(long, help = "只读检查OCR组件并输出JSON，不启动窗口或处理记录")]
    check_runtime: bool,
    #[arg(long, hide = true)]
    headless: bool,
    #[arg(long, hide = true)]
    no_browser: bool,
    #[arg(long, hide = true, default_value_t = 0)]
    port: u16,
    #[arg(long, hide = true)]
    browser_command: Option<PathBuf>,
    #[arg(long, hide = true, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=10000))]
    startup_delay_ms: u64,
    #[arg(long, hide = true)]
    test_probe_dir: Option<PathBuf>,
    #[arg(long, hide = true)]
    test_force_missing_webview2: bool,
}

struct DesktopState {
    backend: Mutex<Option<Arc<AppState>>>,
    origin: RwLock<Option<String>>,
    stop: watch::Sender<bool>,
    exiting: AtomicBool,
    exit_authorized: AtomicBool,
    activated: AtomicUsize,
    loaded: AtomicBool,
    saving: AtomicUsize,
    probe_dir: Option<PathBuf>,
}

impl DesktopState {
    fn write_probe(&self, event: &str, window: Option<&WebviewWindow>) {
        let Some(directory) = &self.probe_dir else {
            return;
        };
        // CI/development evidence only. Never persist a URL, token, file list,
        // command arguments, or source content in this bounded state record.
        let record = json!({
            "version":env!("CARGO_PKG_VERSION"), "pid":std::process::id(),
            "event":event,"activations":self.activated.load(Ordering::SeqCst),
            "webview_loaded":self.loaded.load(Ordering::SeqCst),
            "window_visible":window.and_then(|w|w.is_visible().ok()),
            "window_minimized":window.and_then(|w|w.is_minimized().ok()),
            "exiting":self.exiting.load(Ordering::SeqCst),
            "saving":self.saving.load(Ordering::SeqCst),
        });
        if fs::create_dir_all(directory).is_ok() {
            let _ =
                voucher_workbench::pdf::atomic_json(&directory.join("desktop-probe.json"), &record);
        }
    }
    fn backend(&self) -> Result<Arc<AppState>> {
        self.backend
            .lock()
            .unwrap()
            .clone()
            .context("工作台尚未就绪，请稍候")
    }
    fn request_stop(&self) {
        self.exiting.store(true, Ordering::SeqCst);
        self.stop.send_replace(true);
        self.write_probe("safe_shutdown_requested", None);
    }
}

struct Saving(Arc<DesktopState>);
impl Saving {
    fn begin(state: Arc<DesktopState>) -> Result<Self> {
        state.saving.fetch_add(1, Ordering::SeqCst);
        let saving = Self(state);
        if saving.0.exiting.load(Ordering::SeqCst) {
            bail!("工作台正在退出，尚未开始另存，请重新打开后保存");
        }
        Ok(saving)
    }
}
impl Drop for Saving {
    fn drop(&mut self) {
        self.0.saving.fetch_sub(1, Ordering::SeqCst);
    }
}

fn sanitized_error(error: &str) -> String {
    regex::Regex::new(r#"(?i)#token=[^\s"<>]+"#)
        .expect("fixed redaction expression")
        .replace_all(error, "#token=[已隐藏]")
        .into_owned()
}

fn record_error(directory: Option<&Path>, error: &str) {
    let Some(directory) = directory else { return };
    let record = json!({"version":env!("CARGO_PKG_VERSION"),"pid":std::process::id(),"error":sanitized_error(error)});
    let directory = directory.join("startup-errors");
    if fs::create_dir_all(&directory).is_ok() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let _ = voucher_workbench::pdf::atomic_json(
            &directory.join(format!("{stamp}-{}.json", std::process::id())),
            &record,
        );
    }
}

#[cfg(windows)]
fn native_startup_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "PDF工作台 - 无法启动"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: both buffers are terminated, live UTF-16; no foreign window owns
    // the startup failure dialog and only standard message-box flags are used.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}
#[cfg(not(windows))]
fn native_startup_error(message: &str) {
    rfd::MessageDialog::new()
        .set_title("PDF工作台 - 无法启动")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn guard_window(window: &WebviewWindow) -> Result<Arc<DesktopState>> {
    if window.label() != "main" {
        bail!("该窗口没有本机文件操作权限");
    }
    let state = window
        .app_handle()
        .state::<Arc<DesktopState>>()
        .inner()
        .clone();
    let expected = state
        .origin
        .read()
        .unwrap()
        .clone()
        .context("工作台访问来源尚未就绪")?;
    if window.url()?.origin().ascii_serialization() != expected {
        bail!("请求来源不属于当前本机工作台，已拒绝本机文件操作");
    }
    if state.exiting.load(Ordering::SeqCst) {
        bail!("工作台正在安全退出，请稍候");
    }
    Ok(state)
}

fn display_path(path: PathBuf) -> Result<String> {
    path.into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("路径包含无法显示的字符，请改名后重试"))
}
fn picker_path(path: Option<FilePath>) -> Result<Option<String>> {
    path.map(|path| display_path(path.into_path().context("请选择本机文件路径")?))
        .transpose()
}
fn initial_directory(raw: Option<String>) -> Option<PathBuf> {
    let path = PathBuf::from(raw?);
    let text = path.to_string_lossy();
    if !path.is_absolute()
        || text.starts_with("//")
        || (text.starts_with("\\\\") && !text.starts_with("\\\\?\\"))
        || text.to_ascii_lowercase().starts_with("\\\\?\\unc\\")
    {
        return None;
    }
    let candidate = if path.is_dir() {
        path
    } else {
        path.parent()?.to_owned()
    };
    fs::symlink_metadata(&candidate)
        .ok()
        .filter(|m| m.is_dir() && !m.file_type().is_symlink())
        .map(|_| candidate)
}

#[tauri::command]
async fn select_folder(
    window: WebviewWindow,
    initial_path: Option<String>,
) -> std::result::Result<Option<String>, String> {
    guard_window(&window).map_err(|e| e.to_string())?;
    let mut dialog = window
        .app_handle()
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("选择本地文件夹");
    if let Some(path) = initial_directory(initial_path) {
        dialog = dialog.set_directory(path);
    }
    let (sender, receiver) = oneshot::channel();
    dialog.pick_folder(move |path| {
        let _ = sender.send(path);
    });
    picker_path(
        receiver
            .await
            .map_err(|_| "文件夹选择器未返回结果，请重试".to_string())?,
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
async fn select_index_file(
    window: WebviewWindow,
    initial_path: Option<String>,
) -> std::result::Result<Option<String>, String> {
    guard_window(&window).map_err(|e| e.to_string())?;
    let mut dialog = window
        .app_handle()
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("选择重命名索引文件")
        .add_filter("Excel索引", &["xls", "xlsx"]);
    if let Some(path) = initial_directory(initial_path) {
        dialog = dialog.set_directory(path);
    }
    let (sender, receiver) = oneshot::channel();
    dialog.pick_file(move |path| {
        let _ = sender.send(path);
    });
    picker_path(
        receiver
            .await
            .map_err(|_| "索引选择器未返回结果，请重试".to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct SaveResult {
    saved: bool,
    path: Option<String>,
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("保存目标没有有效的上级文件夹")?;
    let mut file = tempfile::Builder::new()
        .prefix(".pdf-workbench-save-")
        .tempfile_in(parent)
        .context("无法建立另存临时文件，请检查目录权限")?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .context("另存失败，请检查磁盘空间和目录权限")?;
    file.persist_noclobber(path)
        .map_err(|e| e.error)
        .context("目标文件已存在或无法写入；为保护原件不会覆盖，请选择新的文件名")?;
    Ok(())
}

async fn choose_save(
    window: &WebviewWindow,
    name: &str,
    extension: &str,
) -> Result<Option<PathBuf>> {
    let mut dialog = window
        .app_handle()
        .dialog()
        .file()
        .set_parent(window)
        .set_title("另存处理结果")
        .set_file_name(name)
        .add_filter(extension, &[extension]);
    if let Ok(path) = window.app_handle().path().download_dir() {
        dialog = dialog.set_directory(path);
    }
    let (sender, receiver) = oneshot::channel();
    dialog.save_file(move |path| {
        let _ = sender.send(path);
    });
    receiver
        .await
        .context("保存选择器未返回结果，请重试")?
        .map(|p| p.into_path().map_err(anyhow::Error::from))
        .transpose()
}

#[tauri::command]
async fn save_job_report(
    window: WebviewWindow,
    job_id: String,
) -> std::result::Result<SaveResult, String> {
    let result = async {
        let desktop = guard_window(&window)?;
        let state = desktop.backend()?;
        let job = state.job(&job_id)?;
        let bytes =
            serde_json::to_vec_pretty(&voucher_workbench::server::job_report(&state, &job)?)?;
        let Some(path) = choose_save(
            &window,
            &format!("PDF工作台-处理报告-{}.json", job.id),
            "json",
        )
        .await?
        else {
            return Ok(SaveResult {
                saved: false,
                path: None,
            });
        };
        tokio::task::spawn_blocking({
            let path = path.clone();
            let saving = Saving::begin(desktop)?;
            move || {
                let _saving = saving;
                write_new_file(&path, &bytes)
            }
        })
        .await??;
        Ok::<_, anyhow::Error>(SaveResult {
            saved: true,
            path: Some(display_path(path)?),
        })
    }
    .await;
    result.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn save_job_output(
    window: WebviewWindow,
    job_id: String,
    group_id: String,
) -> std::result::Result<SaveResult, String> {
    let result = async {
        let desktop = guard_window(&window)?;
        let state = desktop.backend()?;
        let job = state.job(&job_id)?;
        let original = voucher_workbench::server::job_output(&state, &job, &group_id)?;
        let name = original
            .file_name()
            .and_then(|n| n.to_str())
            .context("输出名称无效")?
            .to_owned();
        let Some(path) = choose_save(&window, &name, "pdf").await? else {
            return Ok(SaveResult {
                saved: false,
                path: None,
            });
        };
        let saved = tokio::task::spawn_blocking({
            let path = path.clone();
            let saving = Saving::begin(desktop)?;
            move || {
                let _saving = saving;
                let bytes = voucher_workbench::server::read_job_output(&state, &job, &group_id)?;
                if path.exists() && path.canonicalize()? == original.canonicalize()? {
                    return Ok(());
                }
                write_new_file(&path, &bytes)
            }
        })
        .await?;
        saved?;
        Ok::<_, anyhow::Error>(SaveResult {
            saved: true,
            path: Some(display_path(path)?),
        })
    }
    .await;
    result.map_err(|e| format!("{e:#}"))
}

fn request_window_close(app: &tauri::AppHandle) {
    let state = app.state::<Arc<DesktopState>>().inner().clone();
    if state.exiting.load(Ordering::SeqCst) {
        return;
    }
    let active = state.saving.load(Ordering::SeqCst) > 0
        || state
            .backend
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|backend| {
                backend
                    .data
                    .lock()
                    .unwrap()
                    .jobs
                    .values()
                    .any(|j| j.active())
            });
    if !active {
        state.request_stop();
        return;
    }
    // Set only while the native choice is displayed, preventing duplicate dialogs.
    if state.exiting.swap(true, Ordering::SeqCst) {
        return;
    }
    state.write_probe(
        "active_task_close_confirmation",
        app.get_webview_window("main").as_ref(),
    );
    let app = app.clone();
    app.dialog().message("任务或文件保存仍在进行。选择“安全停止并退出”会请求取消任务，并等待当前处理及文件保存安全收尾；选择“继续处理”保留窗口与任务。").title("PDF工作台 - 任务正在运行").kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("安全停止并退出".into(),"继续处理".into())).show(move|stop|{
            if stop {state.request_stop();}
            else {state.exiting.store(false,Ordering::SeqCst);state.write_probe("close_cancelled_task_continues",app.get_webview_window("main").as_ref());}
        });
}
#[tauri::command]
fn request_close(window: WebviewWindow) -> std::result::Result<(), String> {
    guard_window(&window).map_err(|e| e.to_string())?;
    request_window_close(window.app_handle());
    Ok(())
}

async fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    task: impl FnOnce(tauri::AppHandle) -> Result<T> + Send + 'static,
) -> Result<T> {
    let (sender, receiver) = oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(task(handle));
    })?;
    receiver.await.context("桌面窗口线程没有响应")?
}

fn activation_callback(app: tauri::AppHandle, state: Arc<DesktopState>) -> OpenBrowser {
    Arc::new(move |_| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let handle = app.clone();
        let state = state.clone();
        app.run_on_main_thread(move || {
            let result = (|| -> Result<()> {
                if state.exiting.load(Ordering::SeqCst) {
                    bail!("工作台正在退出");
                }
                let window = handle
                    .get_webview_window("main")
                    .context("原桌面窗口不存在")?;
                window.show()?;
                window.unminimize()?;
                window.set_focus()?;
                state.activated.fetch_add(1, Ordering::SeqCst);
                state.write_probe("existing_window_activated", Some(&window));
                Ok(())
            })();
            let _ = sender.send(result);
        })?;
        receiver
            .recv_timeout(Duration::from_secs(3))
            .context("原窗口唤醒超时")?
    })
}

async fn launch_service(
    app: tauri::AppHandle,
    args: Args,
    paths: LaunchPaths,
    state: Arc<DesktopState>,
    mut stopped: watch::Receiver<bool>,
) {
    let directory = paths.state_dir.clone();
    let primary = match instance::enter_desktop(
        &directory,
        true,
        activation_callback(app.clone(), state.clone()),
    )
    .await
    {
        Ok(Entry::Primary(primary)) => primary,
        Ok(Entry::Reused) => {
            state.exit_authorized.store(true, Ordering::SeqCst);
            app.exit(0);
            return;
        }
        Err(error) => {
            finish_error(&app, &state, &directory, error);
            return;
        }
    };
    let result=async {
        if args.startup_delay_ms>0 {tokio::time::sleep(Duration::from_millis(args.startup_delay_ms)).await;}
        let server=voucher_workbench::server::prepare_embedded(paths.state_dir,paths.model,args.runtime).await?;
        let url:Url=server.url.parse().context("本地窗口地址无效")?;
        let origin=url.origin().ascii_serialization();
        *state.origin.write().unwrap()=Some(origin.clone());
        *state.backend.lock().unwrap()=Some(server.state.clone());
        let window_state=state.clone();
        on_main(&app,move|app|{
            app.add_capability(json!({"identifier":"desktop-main-files","description":"仅当前PDF工作台窗口可以打开系统选择器及另存登记任务结果","windows":["main"],"local":false,"remote":{"urls":[format!("{origin}/*")]},"permissions":["allow-select-folder","allow-select-index-file","allow-save-job-report","allow-save-job-output","allow-request-close"]}).to_string())?;
            let navigation_origin=origin.clone();let loaded_state=window_state.clone();
            let mut builder=WebviewWindowBuilder::new(&app,"main",tauri::WebviewUrl::External(url)).title("PDF工作台").inner_size(1420.0,900.0).min_inner_size(1000.0,640.0).center()
                .on_navigation(move|url|url.origin().ascii_serialization()==navigation_origin)
                .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny)
                .on_page_load(move|window,payload|{
                    if payload.event()==tauri::webview::PageLoadEvent::Finished {
                        loaded_state.loaded.store(true,Ordering::SeqCst);
                        loaded_state.write_probe("workbench_page_loaded",Some(&window));
                    }
                });
            if let Ok(Some(monitor))=app.primary_monitor() {
                let scale=monitor.scale_factor();let size=monitor.size();
                builder=builder.inner_size((size.width as f64/scale*0.9).min(1420.0),(size.height as f64/scale*0.9).min(900.0));
            }
            let window=builder.build().context("无法创建PDF工作台窗口，请检查WebView2运行时与显示环境")?;
            window.show()?;
            window_state.activated.store(1,Ordering::SeqCst);
            window_state.write_probe("main_window_created",Some(&window));
            Ok(())
        }).await?;
        primary.ready(server.url.clone());
        server.serve(async move{if !*stopped.borrow(){let _=stopped.changed().await;}}).await
    }.await;
    while state.saving.load(Ordering::SeqCst) > 0 {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    primary.close().await;
    match result {
        Ok(()) => {
            state.write_probe("service_closed", None);
            state.exit_authorized.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        Err(error) => finish_error(&app, &state, &directory, error),
    }
}
fn finish_error(
    app: &tauri::AppHandle,
    state: &Arc<DesktopState>,
    directory: &Path,
    error: anyhow::Error,
) {
    let message=sanitized_error(&format!("PDF工作台无法启动：{error:#}\n\n请保留处理记录，按上方原因处理后重试；不会自动结束其他进程、删除锁文件或切换记录目录。"));
    record_error(Some(directory), &message);
    state.write_probe("startup_failed", None);
    let state = state.clone();
    let app = app.clone();
    app.dialog()
        .message(message)
        .title("PDF工作台 - 无法启动")
        .kind(MessageDialogKind::Error)
        .show(move |_| {
            state.exit_authorized.store(true, Ordering::SeqCst);
            app.exit(1);
        });
}

fn run_desktop(args: Args, paths: LaunchPaths) -> Result<()> {
    if args.test_force_missing_webview2 || tauri::webview_version().is_err() {
        let message="未检测到可用的WebView2运行时。请运行独立提供的MicrosoftEdgeWebView2RuntimeInstallerX64.exe，或使用微软官方WebView2安装器，完成后重新打开PDF工作台。处理记录会保留，不会打开外部浏览器。";
        record_error(Some(&paths.state_dir), message);
        native_startup_error(message);
        bail!("WebView2运行时缺失");
    }
    let (stop, stopped) = watch::channel(false);
    let state = Arc::new(DesktopState {
        backend: Mutex::new(None),
        origin: RwLock::new(None),
        stop,
        exiting: AtomicBool::new(false),
        exit_authorized: AtomicBool::new(false),
        activated: AtomicUsize::new(0),
        loaded: AtomicBool::new(false),
        saving: AtomicUsize::new(0),
        probe_dir: args.test_probe_dir.clone(),
    });
    let callback_state = state.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            select_folder,
            select_index_file,
            save_job_report,
            save_job_output,
            request_close
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    request_window_close(window.app_handle());
                }
            }
        })
        .setup(move |app| {
            let app = app.handle().clone();
            tauri::async_runtime::spawn(launch_service(app, args, paths, callback_state, stopped));
            Ok(())
        })
        .build(tauri::generate_context!())?;
    app.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if !state.exit_authorized.load(Ordering::SeqCst) {
                api.prevent_exit();
                request_window_close(app);
            }
        }
    });
    Ok(())
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let paths = LaunchPaths::resolve(args.state_dir.clone(), args.model.clone());
    if args.check_runtime {
        let report = RuntimeDiagnostic::inspect(paths, args.runtime.as_deref());
        println!(
            "{}",
            serde_json::to_string(&report).unwrap_or_else(|_| "{\"model_ready\":false}".into())
        );
        return if report.model_ready {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        };
    }
    let result = (|| -> Result<()> {
        let paths = paths?;
        if args.headless {
            let runtime = tokio::runtime::Runtime::new()?;
            return runtime.block_on(voucher_workbench::server::run_with_options(
                args.port,
                paths.state_dir,
                paths.model,
                args.runtime,
                !args.no_browser,
                StartupOptions {
                    browser_command: args.browser_command,
                    startup_delay_ms: args.startup_delay_ms,
                },
            ));
        }
        run_desktop(args, paths)
    })();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let message = sanitized_error(&format!("PDF工作台无法启动：{error:#}"));
            eprintln!("{message}");
            // Errors shown inside the event loop already exit with code 1.
            // This path handles configuration/build errors before a loop exists.
            if !message.contains("WebView2运行时缺失") {
                native_startup_error(&message);
            }
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_save_never_overwrites_existing_sources_or_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("原件.pdf");
        fs::write(&path, b"source").unwrap();
        assert!(write_new_file(&path, b"replacement").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"source");
        let new = temp.path().join("新副本.pdf");
        write_new_file(&new, b"copy").unwrap();
        assert_eq!(fs::read(new).unwrap(), b"copy");
    }
    #[test]
    fn startup_diagnostics_never_store_session_fragments() {
        let redacted = sanitized_error("failed http://127.0.0.1:1/#token=secret-token text");
        assert!(!redacted.contains("secret-token"));
        assert!(redacted.contains("[已隐藏]"));
    }
}
