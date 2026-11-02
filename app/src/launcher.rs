//! Resolve installed resources without relying on the shell's working directory.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct StartupOptions {
    pub browser_command: Option<PathBuf>,
    pub startup_delay_ms: u64,
}

/// Only the original server retains this opener and its authenticated URL.
/// The optional command is an explicit automation launcher, never an IPC input.
pub fn browser_opener(command: Option<PathBuf>) -> crate::instance::OpenBrowser {
    std::sync::Arc::new(move |url| {
        if let Some(command) = &command {
            if !command.is_absolute() || !command.is_file() {
                bail!("指定的浏览器启动器不存在或不是绝对路径，请检查 --browser-command");
            }
            let mut child = std::process::Command::new(command)
                .arg(url)
                .env("VOUCHER_BROWSER_OWNER_PID", std::process::id().to_string())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("无法运行指定的浏览器启动器，请检查文件权限")?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
            let status = loop {
                if let Some(status) = child.try_wait().context("无法读取浏览器启动器状态")?
                {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    // This explicit automation launcher is our own child, never
                    // an existing workbench or an independently running browser.
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("浏览器启动器超时，请检查 --browser-command 指定的快速启动程序后重试");
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            };
            if !status.success() {
                bail!("浏览器启动器返回失败，请检查浏览器设置后重新双击启动入口");
            }
        } else {
            webbrowser::open(url).map_err(|_| {
                anyhow::anyhow!("系统默认浏览器启动调用失败，请检查HTTP/HTTPS的默认应用设置")
            })?;
        }
        Ok(())
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct LaunchPaths {
    pub state_dir: PathBuf,
    pub model: PathBuf,
}

impl LaunchPaths {
    pub fn resolve(state_dir: Option<PathBuf>, model: Option<PathBuf>) -> Result<Self> {
        resolve_paths(
            state_dir,
            model,
            &std::env::current_exe().context("无法确定程序所在目录")?,
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            cfg!(target_os = "windows"),
        )
    }
}

fn resolve_paths(
    state_dir: Option<PathBuf>,
    model: Option<PathBuf>,
    executable: &Path,
    local_app_data: Option<PathBuf>,
    windows: bool,
) -> Result<LaunchPaths> {
    let state_dir = match state_dir {
        Some(path) => path,
        None if windows => local_app_data
            .filter(|path| path.is_absolute())
            .context("未找到有效的 LOCALAPPDATA 用户目录；请用 --state-dir 指定可写目录")?
            .join("VoucherWorkbench"),
        None => PathBuf::from(".voucher-state"),
    };
    let model = match model {
        Some(path) => path,
        None => {
            let packaged = executable
                .parent()
                .context("无法确定程序所在目录")?
                .join("models")
                .join("title-ocr");
            if windows || packaged.is_dir() {
                packaged
            } else {
                PathBuf::from("app/models/title-ocr")
            }
        }
    };
    Ok(LaunchPaths { state_dir, model })
}

/// A stable, machine-readable diagnostic for a distribution smoke test. This
/// deliberately does not load AppState, create directories, or bind a socket.
#[derive(Debug, Serialize)]
pub struct RuntimeDiagnostic {
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub model: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub runtime_override: Option<PathBuf>,
    pub model_ready: bool,
    pub error: Option<String>,
}

impl RuntimeDiagnostic {
    pub fn inspect(paths: Result<LaunchPaths>, runtime: Option<&Path>) -> Self {
        let mut report = Self {
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            model: None,
            state_dir: None,
            runtime_override: runtime.map(Path::to_path_buf),
            model_ready: false,
            error: None,
        };
        let result = paths.and_then(|paths| {
            report.model = Some(paths.model.clone());
            report.state_dir = Some(paths.state_dir);
            crate::orientation::Engine::new(&paths.model, runtime).map(|_| ())
        });
        match result {
            Ok(()) => report.model_ready = true,
            Err(error) => report.error = Some(format!("{error:#}")),
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_defaults_are_tied_to_executable_and_user_directory() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("中文 安装位置/voucher-workbench.exe");
        let user = temp.path().join("另一用户/本地 数据");
        let paths = resolve_paths(None, None, &exe, Some(user.clone()), true).unwrap();
        assert_eq!(paths.state_dir, user.join("VoucherWorkbench"));
        assert_eq!(paths.model, exe.parent().unwrap().join("models/title-ocr"));
        assert!(!paths.state_dir.exists());
        assert!(!paths.model.exists());
    }

    #[test]
    fn windows_missing_or_relative_user_directory_never_uses_cwd() {
        for local in [None, Some(PathBuf::new()), Some(PathBuf::from("relative"))] {
            let error = resolve_paths(None, None, Path::new("program.exe"), local, true)
                .unwrap_err()
                .to_string();
            assert!(error.contains("--state-dir"));
        }
    }

    #[test]
    fn explicit_paths_override_missing_windows_environment_and_keep_cli_semantics() {
        let paths = resolve_paths(
            Some(PathBuf::from("custom-state")),
            Some(PathBuf::from("custom-model")),
            Path::new("program.exe"),
            None,
            true,
        )
        .unwrap();
        assert_eq!(paths.state_dir, Path::new("custom-state"));
        assert_eq!(paths.model, Path::new("custom-model"));
    }

    #[test]
    fn non_windows_keeps_developer_paths_but_prefers_packaged_models() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("凭证 工具/voucher-workbench");
        let developer = resolve_paths(None, None, &exe, None, false).unwrap();
        assert_eq!(developer.state_dir, Path::new(".voucher-state"));
        assert_eq!(developer.model, Path::new("app/models/title-ocr"));
        let models = exe.parent().unwrap().join("models/title-ocr");
        std::fs::create_dir_all(&models).unwrap();
        let packaged = resolve_paths(None, None, &exe, None, false).unwrap();
        assert_eq!(packaged.model, models);
        assert_eq!(packaged.state_dir, developer.state_dir);
    }
}
