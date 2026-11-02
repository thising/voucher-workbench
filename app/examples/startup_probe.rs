//! CI-only launcher probes. Never shipped in the customer package.
//! Browser records contain booleans and PIDs, never the URL or session token.
use anyhow::{bail, Context, Result};
#[cfg(windows)]
use serde::Deserialize;
use serde::Serialize;
#[cfg(windows)]
use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};
use std::{fs, io::Write, path::PathBuf, process::ExitCode};

#[derive(Serialize)]
struct BrowserRecord {
    owner_pid: u32,
    probe_pid: u32,
    loopback: bool,
    has_token: bool,
    token_length: usize,
    failed: bool,
}

fn browser(url: &str) -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("VOUCHER_BROWSER_PROBE_DIR").context("Missing browser probe directory")?,
    );
    if !root.is_absolute() {
        bail!("Browser probe directory must be absolute");
    }
    fs::create_dir_all(&root)?;
    let token = url
        .split_once("/#token=")
        .map(|(_, token)| token)
        .unwrap_or("");
    let record = BrowserRecord {
        owner_pid: std::env::var("VOUCHER_BROWSER_OWNER_PID")?.parse()?,
        probe_pid: std::process::id(),
        loopback: url.starts_with("http://127.0.0.1:"),
        has_token: token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
        token_length: token.len(),
        failed: root.join("fail").exists(),
    };
    let pending = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&pending, serde_json::to_vec(&record)?)?;
    fs::rename(&pending, pending.with_extension("json"))?;
    if record.failed {
        bail!("Synthetic browser failure");
    }
    if !record.loopback || !record.has_token {
        bail!("Invalid workbench browser request");
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Deserialize)]
struct CmdRequest {
    entry: PathBuf,
    arguments: Vec<String>,
    cwd: PathBuf,
    code_page: u16,
    environment: BTreeMap<String, String>,
}

#[cfg(windows)]
fn spawn_cmd(request_path: &str) -> Result<i32> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;

    let request: CmdRequest = serde_json::from_slice(&fs::read(request_path)?)?;
    if !request.entry.is_absolute()
        || !request.cwd.is_absolute()
        || ![936, 65001].contains(&request.code_page)
    {
        bail!("Invalid isolated CMD request");
    }
    let quote = |value: &str| -> Result<String> {
        if value.contains(['"', '\r', '\n']) {
            bail!("Unsupported test argument");
        }
        Ok(format!("\"{value}\""))
    };
    let mut parts = vec![quote(
        request.entry.to_str().context("Invalid entry path")?,
    )?];
    parts.extend(
        request
            .arguments
            .iter()
            .map(|arg| quote(arg))
            .collect::<Result<Vec<_>>>()?,
    );
    // The command text is passed as Unicode to cmd.exe. Only the package .cmd
    // is read as a code-page-sensitive batch file; no Unicode wrapper batch is used.
    let command = format!(
        "\"chcp {} >nul & call {}\"",
        request.code_page,
        parts.join(" ")
    );
    let executable = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
    let mut child = Command::new(executable)
        .raw_arg("/d /s /c")
        .raw_arg(command)
        .current_dir(request.cwd)
        .env_remove("ORT_DYLIB_PATH")
        .envs(request.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // CTRL+C is delivered only to this private console, never the CI shell.
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .context("Cannot create isolated cmd.exe console")?;
    println!("PROBE_CMD_PID={}", child.id());
    std::io::stdout().flush()?;
    let mut input = child.stdin.take().context("Missing child stdin")?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut input);
    });
    let mut stdout = child.stdout.take().context("Missing child stdout")?;
    let mut stderr = child.stderr.take().context("Missing child stderr")?;
    fn forward(mut source: impl std::io::Read, mut destination: impl Write) -> std::io::Result<()> {
        let mut buffer = [0u8; 4096];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            destination.write_all(&buffer[..count])?;
            destination.flush()?;
        }
    }
    let output = std::thread::spawn(move || forward(&mut stdout, &mut std::io::stdout().lock()));
    let errors = std::thread::spawn(move || forward(&mut stderr, &mut std::io::stderr().lock()));
    let status = child.wait()?;
    let _ = output.join();
    let _ = errors.join();
    Ok(status.code().unwrap_or(1))
}

#[cfg(not(windows))]
fn spawn_cmd(_: &str) -> Result<i32> {
    bail!("The CMD probe requires Windows");
}

#[cfg(windows)]
fn ctrl_c(pid: u32) -> Result<()> {
    use windows_sys::Win32::System::Console::{
        AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler, CTRL_C_EVENT,
    };
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Cannot attach the owned private console");
        }
        if SetConsoleCtrlHandler(None, 1) == 0 {
            FreeConsole();
            return Err(std::io::Error::last_os_error())
                .context("Cannot protect the signal helper");
        }
        let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
        let failure = std::io::Error::last_os_error();
        std::thread::sleep(std::time::Duration::from_millis(250));
        FreeConsole();
        if sent == 0 {
            return Err(failure).context("Cannot send CTRL+C to the owned private console");
        }
    }
    println!("{{\"sent\":true}}");
    Ok(())
}

#[cfg(not(windows))]
fn ctrl_c(_: u32) -> Result<()> {
    bail!("The console signal probe requires Windows");
}

fn run() -> Result<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [url] if url.starts_with("http://") => {
            browser(url)?;
            Ok(0)
        }
        [command, state] if command == "hold-lock" => {
            fs::create_dir_all(state)?;
            let lock = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(PathBuf::from(state).join("instance.lock"))?;
            fs2::FileExt::try_lock_exclusive(&lock)?;
            println!("PROBE_LOCK_READY");
            std::io::stdout().flush()?;
            loop {
                std::thread::park();
            }
        }
        [command, request] if command == "spawn-cmd" => spawn_cmd(request),
        [command, state] if command == "lock-held" => {
            let path = PathBuf::from(state).join("instance.lock");
            let held = if path.exists() {
                let file = fs::OpenOptions::new().write(true).open(path)?;
                match fs2::FileExt::try_lock_exclusive(&file) {
                    Ok(()) => false,
                    Err(error)
                        if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                    {
                        true
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                false
            };
            println!("{}", serde_json::json!({"held":held}));
            Ok(0)
        }
        [command, pid] if command == "ctrl-c" => {
            ctrl_c(pid.parse()?)?;
            Ok(0)
        }
        _ => bail!(
            "Use a browser URL, hold-lock STATE, lock-held STATE, spawn-cmd REQUEST, or ctrl-c PID"
        ),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("Startup probe: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(windows, test))]
mod tests {
    use super::*;

    #[test]
    fn real_cmd_handles_ascii_crlf_entry_with_unicode_and_spaced_paths() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("中文 含空格 启动入口");
        fs::create_dir(&root)?;
        let entry = root.join("启动入口.cmd");
        fs::write(
            &entry,
            b"@echo off\r\nif not exist \"%~1\" exit /b 8\r\nif not \"%~2\"==\"argument with spaces\" exit /b 7\r\nexit /b 0\r\n",
        )?;
        let input = root.join("中文 测试文件.txt");
        fs::write(&input, "synthetic")?;
        for code_page in [936, 65001] {
            let request = root.join(format!("request-{code_page}.json"));
            fs::write(
                &request,
                serde_json::to_vec(&serde_json::json!({
                    "entry": entry,
                    "arguments": [input.to_string_lossy(), "argument with spaces"],
                    "cwd": temp.path(),
                    "code_page": code_page,
                    "environment": {}
                }))?,
            )?;
            anyhow::ensure!(
                spawn_cmd(request.to_str().context("Invalid test path")?)? == 0,
                "The real CMD rejected a quoted Unicode path or spaced argument"
            );
        }
        Ok(())
    }
}
