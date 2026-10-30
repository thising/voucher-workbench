use serde_json::Value;
use std::process::Command;

#[test]
fn runtime_diagnostic_reports_missing_models_as_json_without_creating_state() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("无关 工作目录");
    let state = temp.path().join("记录 不得创建");
    let model = temp.path().join("缺少的 模型");
    std::fs::create_dir_all(&cwd).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_voucher-workbench"))
        .current_dir(&cwd)
        .arg("--check-runtime")
        .arg("--state-dir")
        .arg(&state)
        .arg("--model")
        .arg(&model)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["model_ready"], false);
    assert_eq!(report["model"], model.to_str().unwrap());
    assert_eq!(report["state_dir"], state.to_str().unwrap());
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    assert!(report["error"].as_str().unwrap().contains("模型"));
    assert!(!state.exists());
    assert_eq!(std::fs::read_dir(&cwd).unwrap().count(), 0);
    assert_eq!(output.stdout.split(|byte| *byte == b'\n').count(), 2);
}

#[cfg(windows)]
#[test]
fn windows_default_diagnostic_ignores_cwd_and_does_not_create_user_state() {
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("中文 包目录");
    let cwd = temp.path().join("另一个 工作目录");
    let user = temp.path().join("用户 本地资料");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let executable = package.join("voucher-workbench.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_voucher-workbench"), &executable).unwrap();
    let output = Command::new(&executable)
        .current_dir(&cwd)
        .env("LOCALAPPDATA", &user)
        .arg("--check-runtime")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let actual_model = std::path::PathBuf::from(report["model"].as_str().unwrap());
    assert!(actual_model.is_absolute());
    assert!(actual_model.ends_with("models/title-ocr"));
    // Windows current_exe and canonicalize may use different extended prefixes.
    assert_eq!(
        actual_model
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap(),
        package.canonicalize().unwrap()
    );
    assert_eq!(
        report["state_dir"],
        user.join("VoucherWorkbench").to_str().unwrap()
    );
    assert!(!user.exists());
    assert_eq!(std::fs::read_dir(&cwd).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn windows_missing_user_directory_is_a_json_diagnostic_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_voucher-workbench"))
        .env_remove("LOCALAPPDATA")
        .arg("--check-runtime")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["model_ready"], false);
    assert!(report["error"].as_str().unwrap().contains("LOCALAPPDATA"));
    assert!(report["state_dir"].is_null());
}
