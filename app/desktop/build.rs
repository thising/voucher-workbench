fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "select_folder",
            "select_index_file",
            "save_job_report",
            "save_job_output",
            "request_close",
        ]),
    ))
    .expect("Unable to build PDF工作台 desktop resources");
}
