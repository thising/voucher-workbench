use crate::{
    jobs::{AppState, GenerateRequest, Job, ProcessingSettings},
    model::{ScanRequest, ORIENTATION_DISCLAIMER_VERSION},
    pdf,
};
use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

#[path = "pdfjs_assets.rs"]
mod pdfjs_assets;

type ApiResult<T> = std::result::Result<T, ApiError>;
pub struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{:#}",self.0)})),
        )
            .into_response()
    }
}

async fn auth(State(state): State<Arc<AppState>>, request: Request<Body>, next: Next) -> Response {
    let token = request
        .headers()
        .get("x-session-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host_valid = host
        .strip_prefix("127.0.0.1:")
        .is_some_and(|p| p.parse::<u16>().is_ok());
    let origin_valid = request
        .headers()
        .get(header::ORIGIN)
        .map(|v| v.to_str().ok() == Some(format!("http://{host}").as_str()))
        .unwrap_or(true);
    let cross_site = request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site");
    if token != state.token || !host_valid || !origin_valid || cross_site {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"会话无效或请求来源不符，请使用启动时提供的完整链接重新打开"})),
        )
            .into_response();
    }
    if state
        .shutting_down
        .load(std::sync::atomic::Ordering::SeqCst)
        && !matches!(
            *request.method(),
            axum::http::Method::GET | axum::http::Method::HEAD
        )
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"PDF工作台正在保存进度并退出，请等待退出完成后重新打开"})),
        )
            .into_response();
    }
    next.run(request).await
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert("cache-control", "no-store".parse().unwrap());
    h.insert("content-security-policy","default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; font-src 'self' blob: data:; frame-src 'self' blob:; object-src blob:; connect-src 'self' ipc: http://ipc.localhost; worker-src 'self' blob:; base-uri 'none'; frame-ancestors 'none'; form-action 'self'".parse().unwrap());
    response
}

async fn status(State(s): State<Arc<AppState>>) -> Json<Value> {
    let running = s
        .data
        .lock()
        .unwrap()
        .jobs
        .values()
        .filter(|j| j.active())
        .cloned()
        .collect::<Vec<_>>();
    Json(
        json!({"version":env!("CARGO_PKG_VERSION"),"orientation_model":crate::orientation::MODEL_ID,"orientation_disclaimer_version":ORIENTATION_DISCLAIMER_VERSION,"model_ready":s.model_error.is_none(),"model_error":s.model_error,"running_jobs":running}),
    )
}

async fn settings(State(s): State<Arc<AppState>>) -> Json<ProcessingSettings> {
    Json(s.settings())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BatchPreferences {
    orientation_threads: usize,
    merge_threads: usize,
}

impl Default for BatchPreferences {
    fn default() -> Self {
        Self {
            orientation_threads: 4,
            merge_threads: 4,
        }
    }
}

impl BatchPreferences {
    fn validate(self) -> Result<Self> {
        anyhow::ensure!(
            (1..=10).contains(&self.orientation_threads) && (1..=10).contains(&self.merge_threads),
            "识别与合并线程数必须分别为1至10的整数"
        );
        Ok(self)
    }
}

fn read_batch_preferences(state: &AppState) -> Result<BatchPreferences> {
    let path = state.dir.join("batch-preferences.json");
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<BatchPreferences>(&bytes)
            .context("批量合并线程偏好无法读取，请保留记录并反馈错误")?
            .validate(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(BatchPreferences::default())
        }
        Err(error) => Err(error).context("无法读取批量合并线程偏好，请检查目录权限"),
    }
}

async fn batch_preferences(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<BatchPreferences>> {
    Ok(Json(read_batch_preferences(&state)?))
}

async fn update_batch_preferences(
    State(state): State<Arc<AppState>>,
    Json(preferences): Json<BatchPreferences>,
) -> ApiResult<Json<BatchPreferences>> {
    let preferences = preferences.validate()?;
    let data = state.data.lock().unwrap();
    if state
        .shutting_down
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(anyhow::anyhow!("工作台正在退出，请稍后重新打开").into());
    }
    if data.jobs.values().any(Job::active) {
        return Err(anyhow::anyhow!("任务运行中不能更改线程偏好，请等待完成或取消后重试").into());
    }
    pdf::atomic_json(&state.dir.join("batch-preferences.json"), &preferences)?;
    Ok(Json(preferences))
}
async fn update_settings(
    State(s): State<Arc<AppState>>,
    Json(settings): Json<ProcessingSettings>,
) -> ApiResult<Json<ProcessingSettings>> {
    Ok(Json(s.set_settings(settings)?))
}

async fn browse(Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    let raw = q
        .get("path")
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let path = raw.canonicalize().context("目录不存在或无法访问")?;
    if !path.is_dir() {
        return Err(anyhow::anyhow!("请选择目录").into());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = entry.file_type()?;
        if name.starts_with('.') || kind.is_symlink() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if kind.is_dir() || matches!(ext.as_str(), "xls" | "xlsx") {
            entries.push(
                json!({"name":name,"path":entry.path().to_string_lossy(),"is_dir":kind.is_dir()}),
            );
        }
    }
    entries.sort_by(|a, b| {
        b["is_dir"]
            .as_bool()
            .cmp(&a["is_dir"].as_bool())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(Json(
        json!({"path":path.to_string_lossy(),"parent":path.parent().map(|p|p.to_string_lossy()),"entries":entries}),
    ))
}
async fn sheets(Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    let path = q.get("path").context("请指定明细账路径")?.clone();
    let list =
        tokio::task::spawn_blocking(move || crate::ledger::inspect(FsPath::new(&path))).await??;
    Ok(Json(serde_json::to_value(list)?))
}
async fn scan(
    State(s): State<Arc<AppState>>,
    Json(req): Json<ScanRequest>,
) -> ApiResult<Json<Value>> {
    let p = tokio::task::spawn_blocking(move || crate::scan::scan(&req)).await??;
    Ok(Json(serde_json::to_value(s.insert_plan(p)?)?))
}
async fn plans(State(s): State<Arc<AppState>>) -> Json<Value> {
    let d = s.data.lock().unwrap();
    let mut plans=d.plans.values().map(|p|json!({"id":p.id,"root":p.root,"sheet":p.sheet,"created_at":p.created_at,"vouchers":p.vouchers.len(),"pages":p.vouchers.iter().flat_map(|v|&v.files).map(|f|f.pages).sum::<u32>(),"orientation_workflow":p.orientation_workflow,"review_count":p.vouchers.iter().flat_map(|v|&v.files).filter(|f|pdf::requires_manual_review(p,f)).count(),"algorithm_unresolved_count":p.vouchers.iter().flat_map(|v|&v.files).filter(|f|pdf::needs_review(f)).count()})).collect::<Vec<_>>();
    plans.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
    Json(json!(plans))
}
async fn plan(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(s.plan(&id)?)?))
}
#[derive(Deserialize)]
struct OrientationSettings {
    enabled: bool,
    #[serde(default)]
    accept_disclaimer: bool,
}
async fn orientation_settings(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<OrientationSettings>,
) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(s.set_orientation_workflow(
        &id,
        req.enabled,
        req.accept_disclaimer,
    )?)?))
}
async fn analyze(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(s.start(&id, "analyze", GenerateRequest::default())?))
}
async fn generate(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<GenerateRequest>,
) -> ApiResult<Json<Job>> {
    Ok(Json(s.start(&id, "generate", req)?))
}
#[derive(Deserialize)]
struct Rotation {
    rotation_cw: u16,
    #[serde(default)]
    reviewed: bool,
}
async fn rotate(
    State(s): State<Arc<AppState>>,
    Path((id, file)): Path<(String, String)>,
    Json(req): Json<Rotation>,
) -> ApiResult<Json<Value>> {
    if !req.reviewed {
        return Err(anyhow::anyhow!("人工修改方向需要同时确认复核").into());
    }
    Ok(Json(serde_json::to_value(s.edit_files(
        &id,
        &[file],
        Some(req.rotation_cw),
    )?)?))
}
#[derive(Deserialize)]
struct Review {
    file_ids: Vec<String>,
}
async fn review(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<Review>,
) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(s.edit_files(
        &id,
        &req.file_ids,
        None,
    )?)?))
}
fn bytes_response(bytes: Vec<u8>, content_type: &str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}
async fn source(
    State(s): State<Arc<AppState>>,
    Path((id, file_id)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let plan = s.plan(&id)?;
    let file = plan
        .vouchers
        .iter()
        .flat_map(|v| &v.files)
        .find(|f| f.id == file_id)
        .context("文件不属于此计划")?
        .clone();
    let path = pdf::check_source(FsPath::new(&plan.root), &file)?;
    let thumb = q.get("thumbnail").is_some_and(|v| v == "true");
    if file.kind == "image" && thumb {
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let image = crate::orientation::load_image(&path)?.thumbnail(1200, 1200);
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85)
                .encode_image(&crate::orientation::composite_white(&image))?;
            Ok(bytes)
        })
        .await??;
        Ok(bytes_response(bytes, "image/jpeg"))
    } else {
        Ok(bytes_response(
            tokio::fs::read(&path).await?,
            if file.kind == "pdf" {
                "application/pdf"
            } else {
                match path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "png" => "image/png",
                    "webp" => "image/webp",
                    "bmp" => "image/bmp",
                    "tif" | "tiff" => "image/tiff",
                    _ => "image/jpeg",
                }
            },
        ))
    }
}
async fn jobs(State(s): State<Arc<AppState>>) -> Json<Vec<Job>> {
    let mut j = s
        .data
        .lock()
        .unwrap()
        .jobs
        .values()
        .cloned()
        .collect::<Vec<_>>();
    j.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(j)
}
async fn job(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(s.job(&id)?))
}
async fn cancel(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(s.cancel(&id)?))
}
async fn retry(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(s.retry(&id)?))
}
async fn report(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let job = s.job(&id)?;
    Ok(Json(
        tokio::task::spawn_blocking(move || job_report(&s, &job)).await??,
    ))
}

/// One frozen-report contract is shared by the desktop save dialog and HTTP.
pub fn job_report(s: &AppState, job: &Job) -> Result<Value> {
    if job.kind.starts_with("batch_") {
        return crate::batch::report(s, job);
    }
    if job.kind == "generic_generate" {
        return crate::generic::report(s, job);
    }
    let plan = s.plan(&job.plan_id)?;
    let mut manifests = vec![];
    for result in &job.results {
        if result.output_path.is_some() {
            let path = s
                .dir
                .join("reports")
                .join(&job.id)
                .join(format!("{}.json", result.voucher_id));
            let bytes = fs::read(path).context("任务输出快照缺失，请检查任务状态目录")?;
            manifests.push(serde_json::from_slice::<Value>(&bytes)?);
        }
    }
    Ok(
        json!({"job":job,"plan_id":plan.id,"root":plan.root,"ledger":plan.ledger_path,"sheet":plan.sheet,"issues":plan.issues,"manifests":manifests}),
    )
}

async fn output(
    State(s): State<Arc<AppState>>,
    Path((job_id, unit_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let job = s.job(&job_id)?;
    let bytes = tokio::task::spawn_blocking(move || read_job_output(&s, &job, &unit_id)).await??;
    Ok(bytes_response(bytes, "application/pdf"))
}

/// Validate the registered task, unit, location, and its original output hash.
pub fn job_output(s: &AppState, job: &Job, unit_id: &str) -> Result<PathBuf> {
    if job.kind == "batch_generate" {
        return crate::batch::output(s, job, unit_id);
    }
    if job.kind == "generic_generate" {
        return crate::generic::output(s, job, unit_id);
    }
    anyhow::ensure!(job.kind == "generate", "该任务没有可保存的PDF成品");
    let result = job
        .results
        .iter()
        .find(|r| {
            r.voucher_id == unit_id
                && r.output_path.is_some()
                && matches!(r.status.as_str(), "completed" | "skipped")
        })
        .context("此凭证尚未成功生成")?;
    let plan = s.plan(&job.plan_id)?;
    let voucher = plan
        .vouchers
        .iter()
        .find(|v| v.id == result.voucher_id)
        .context("凭证不属于计划")?;
    let path = pdf::safe_source(
        FsPath::new(&plan.root),
        voucher
            .output_relative_path
            .as_deref()
            .context("无输出路径")?,
    )?;
    let record: pdf::Manifest = serde_json::from_slice(&fs::read(
        s.dir
            .join("reports")
            .join(&job.id)
            .join(format!("{unit_id}.json")),
    )?)?;
    anyhow::ensure!(
        pdf::sha256_file(&path)? == record.output_sha256,
        "该输出已更新，请从最新任务下载；历史版本保存在输出目录的 .history 中"
    );
    Ok(path)
}

/// Hash the bytes actually returned, closing the validate-then-read race for
/// both web downloads and native Save As. No arbitrary source path is accepted.
pub fn read_job_output(s: &AppState, job: &Job, unit_id: &str) -> Result<Vec<u8>> {
    use sha2::Digest;
    let path = job_output(s, job, unit_id)?;
    let snapshot: Value = serde_json::from_slice(&fs::read(
        s.dir
            .join("reports")
            .join(&job.id)
            .join(format!("{unit_id}.json")),
    )?)
    .context("任务成品快照无法读取，请保留处理记录")?;
    let expected = snapshot["output_sha256"]
        .as_str()
        .context("任务成品快照缺少校验值")?;
    let bytes = fs::read(path).context("无法读取成品PDF，请检查文件权限及磁盘连接")?;
    anyhow::ensure!(
        hex::encode(sha2::Sha256::digest(&bytes)) == expected,
        "成品在读取期间变化，请使用最近任务的成品或核对历史归档"
    );
    Ok(bytes)
}
#[derive(Deserialize)]
struct Item {
    item: String,
}
async fn open_output(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<Item>,
) -> ApiResult<Json<Value>> {
    let p = s.plan(&id)?;
    let v = p
        .vouchers
        .iter()
        .find(|v| v.item == req.item)
        .context("分项不属于此计划")?;
    let relative = v
        .output_relative_path
        .as_deref()
        .context("分项没有有效输出路径")?;
    let dir = FsPath::new(relative)
        .parent()
        .context("输出路径不合法")?
        .to_string_lossy();
    let path = pdf::safe_source(FsPath::new(&p.root), &dir).context("输出目录尚未生成")?;
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(target_os = "windows")]
    let command = "explorer.exe";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = "xdg-open";
    std::process::Command::new(command)
        .arg(&path)
        .spawn()
        .context("无法打开系统文件管理器")?;
    Ok(Json(json!({"ok":true})))
}

async fn pdfjs_asset(Path(asset): Path<String>) -> Response {
    let Some(data) = pdfjs_assets::get(&asset) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = match asset.rsplit('.').next().unwrap_or("") {
        "mjs" | "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json; charset=utf-8",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, mime)], data).into_response()
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/settings", get(settings).patch(update_settings))
        .route(
            "/batch/preferences",
            get(batch_preferences).patch(update_batch_preferences),
        )
        .route("/fs", get(browse))
        .route("/ledger/sheets", get(sheets))
        .route("/scan", post(scan))
        .route("/plans", get(plans))
        .route("/plans/{id}", get(plan))
        .route("/plans/{id}/orientation", patch(orientation_settings))
        .route("/plans/{id}/analyze", post(analyze))
        .route("/plans/{id}/generate", post(generate))
        .route("/plans/{id}/files/{file}", get(source).patch(rotate))
        .route("/plans/{id}/review", post(review))
        .route("/plans/{id}/open-output", post(open_output))
        .route("/jobs", get(jobs))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/cancel", post(cancel))
        .route("/jobs/{id}/retry", post(retry))
        .route("/jobs/{id}/report", get(report))
        .route("/jobs/{job}/outputs/{voucher}", get(output))
        .nest("/generic", crate::generic::router())
        .nest("/batch", crate::batch::router())
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Html(include_str!("../web/workbench.html")) }),
        )
        .route(
            "/legacy",
            get(|| async { axum::response::Html(include_str!("../web/index.html")) }),
        )
        .route("/pdfjs/{*asset}", get(pdfjs_asset))
        .route(
            "/workbench.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/workbench.js"),
                )
            }),
        )
        .route(
            "/workbench.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/workbench.css"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/generic",
            get(|| async { axum::response::Html(include_str!("../web/generic.html")) }),
        )
        .route(
            "/generic.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/generic.js"),
                )
            }),
        )
        .route(
            "/generic.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/generic.css"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .nest("/api", api)
        .with_state(state)
        .layer(middleware::from_fn(security_headers))
}

pub async fn run(
    port: u16,
    dir: PathBuf,
    model: PathBuf,
    runtime: Option<PathBuf>,
    open: bool,
) -> Result<()> {
    run_with_options(
        port,
        dir,
        model,
        runtime,
        open,
        crate::launcher::StartupOptions::default(),
    )
    .await
}

/// The desktop owner acquires its protected instance lock before preparing this
/// server. No activation state, authentication URL, or token is persisted here.
pub struct EmbeddedServer {
    pub state: Arc<AppState>,
    pub url: String,
    listener: tokio::net::TcpListener,
}

pub async fn prepare_embedded(
    dir: PathBuf,
    model: PathBuf,
    runtime: Option<PathBuf>,
) -> Result<EmbeddedServer> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("无法启动本机PDF工作台，请检查本地网络策略与安全软件对本程序的限制")?;
    let state =
        tokio::task::spawn_blocking(move || AppState::load(dir, &model, runtime.as_deref()))
            .await
            .context("读取处理记录的工作线程异常，请保留记录并重新启动")?
            .context(
                "无法读取处理记录，请检查目录权限、磁盘空间及具体原因；请保留记录，不要删除锁文件",
            )?;
    let url = format!(
        "http://127.0.0.1:{}/#token={}",
        listener.local_addr()?.port(),
        state.token
    );
    Ok(EmbeddedServer {
        state,
        url,
        listener,
    })
}

impl EmbeddedServer {
    pub async fn serve(
        self,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<()> {
        let state = self.state;
        let shutdown_state = state.clone();
        axum::serve(self.listener, router(state))
            .with_graceful_shutdown(async move {
                shutdown.await;
                shutdown_state
                    .shutting_down
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                for cancel in shutdown_state.data.lock().unwrap().cancels.values() {
                    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                // Never abandon an in-flight PDF transaction just because its
                // window closed. Workers save partial progress before exit.
                while shutdown_state
                    .data
                    .lock()
                    .unwrap()
                    .jobs
                    .values()
                    .any(Job::active)
                {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            })
            .await
            .context("PDF工作台本地服务异常，请保留处理记录和错误信息后重新启动")
    }
}

pub async fn run_with_options(
    port: u16,
    dir: PathBuf,
    model: PathBuf,
    runtime: Option<PathBuf>,
    open: bool,
    options: crate::launcher::StartupOptions,
) -> Result<()> {
    if options.startup_delay_ms > 10_000 {
        anyhow::bail!("启动延迟参数不能超过10000毫秒");
    }
    let opener = crate::launcher::browser_opener(options.browser_command);
    let primary = match crate::instance::enter(&dir, open, opener.clone()).await? {
        crate::instance::Entry::Reused => return Ok(()),
        crate::instance::Entry::Primary(primary) => primary,
    };
    let result = async {
        // The protected activation endpoint already exists while state/models
        // initialize. A second click waits rather than racing another service.
        if options.startup_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(options.startup_delay_ms)).await;
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await.map_err(|error| {
                let advice = if error.kind() == std::io::ErrorKind::AddrInUse {
                    format!("本地端口{port}已被占用。请检查占用该端口的应用，或手动使用 --port 0 分配可用端口后重新启动；原处理记录目录保持不变")
                } else if error.kind() == std::io::ErrorKind::PermissionDenied {
                    "当前账号无权监听本地端口。请检查安全软件或单位策略对本程序的限制，联系管理员确认后重试".into()
                } else {
                    "本地端口无法监听。请检查本机网络/安全策略和端口设置后重试，不会自动结束其他程序".into()
                };
                anyhow::Error::from(error).context(advice)
            })?;
        let state = tokio::task::spawn_blocking(move || AppState::load(dir, &model, runtime.as_deref()))
            .await.context("处理记录载入线程异常，请保留现有记录并重新启动")?
            .context("无法载入处理记录。请检查记录目录权限、可用磁盘空间及上方具体原因；保留记录并反馈错误，不要删除锁文件或清空记录")?;
        let url = format!("http://127.0.0.1:{}/#token={}", listener.local_addr()?.port(), state.token);
        primary.ready(url.clone());
        println!("凭证 PDF 工作台：{url}");
        println!("仅在本机访问。请保留此窗口，结束时按 Ctrl+C；重复双击启动入口可重新打开已有工作台，任务继续运行。");
        if let Some(error) = &state.model_error {
            eprintln!("方向模型暂不可用：{error}。可关闭方向识别或使用通用PDF模式；其他处理功能仍可使用。");
        }
        if open {
            let opened = tokio::task::spawn_blocking(move || opener(&url)).await;
            if !matches!(opened, Ok(Ok(()))) {
                eprintln!("浏览器打开失败，服务已经启动并保持运行。请检查系统默认浏览器设置，或复制本窗口上方的完整网址手动打开；也可修复浏览器设置后再次双击启动入口。结束服务请按 Ctrl+C。");
            }
        }
        let closing = primary.closing_notifier();
        axum::serve(listener, router(state.clone()))
            .with_graceful_shutdown(async move {
                let _ = tokio::signal::ctrl_c().await;
                closing();
                for cancel in state.data.lock().unwrap().cancels.values() {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                for _ in 0..150 {
                    if !state.data.lock().unwrap().jobs.values().any(Job::active) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            })
            .await.context("本地服务运行失败。请保留处理记录和错误信息，等待原服务窗口结束后重新启动")?;
        Ok(())
    }.await;
    // Close all IPC handles before the state-directory lock is released.
    primary.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn local_api_requires_token_loopback_host_and_same_origin() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::load(
            tmp.path().join("state"),
            &tmp.path().join("missing-model"),
            None,
        )
        .unwrap();
        let app = router(state.clone());
        for uri in [
            "/api/status",
            "/api/generic/plans",
            "/api/batch/plans",
            "/api/batch/preferences",
        ] {
            for (host, token, origin, expected) in [
                (
                    "127.0.0.1:43127",
                    state.token.as_str(),
                    None,
                    StatusCode::OK,
                ),
                ("127.0.0.1:43127", "", None, StatusCode::FORBIDDEN),
                (
                    "attacker.invalid:43127",
                    state.token.as_str(),
                    None,
                    StatusCode::FORBIDDEN,
                ),
                (
                    "127.0.0.1:43127",
                    state.token.as_str(),
                    Some("https://attacker.invalid"),
                    StatusCode::FORBIDDEN,
                ),
                (
                    "127.0.0.1:43127",
                    state.token.as_str(),
                    Some("http://127.0.0.1:43127"),
                    StatusCode::OK,
                ),
            ] {
                let mut req = Request::builder()
                    .uri(uri)
                    .header("host", host)
                    .header("x-session-token", token);
                if let Some(origin) = origin {
                    req = req.header("origin", origin);
                }
                let response = app
                    .clone()
                    .oneshot(req.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), expected);
                assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
                if expected == StatusCode::OK && uri == "/api/status" {
                    let bytes = response.into_body().collect().await.unwrap().to_bytes();
                    let status: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(status["model_ready"], false);
                }
            }
        }
    }

    async fn request_json(
        state: &Arc<AppState>,
        method: &str,
        uri: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("host", "127.0.0.1:43127")
                    .header("x-session-token", &state.token)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn batch_preferences_persist_independently_and_reject_invalid_or_busy_updates() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state");
        let model = tmp.path().join("missing-model");
        let state = AppState::load(dir.clone(), &model, None).unwrap();
        state
            .set_settings(ProcessingSettings { threads: 7 })
            .unwrap();
        let legacy_settings = fs::read(dir.join("settings.json")).unwrap();
        let uri = "/api/batch/preferences";
        assert_eq!(
            request_json(&state, "GET", uri, Value::Null).await,
            (
                StatusCode::OK,
                json!({"orientation_threads":4,"merge_threads":4})
            )
        );
        let chosen = json!({"orientation_threads":10,"merge_threads":1});
        assert_eq!(
            request_json(&state, "PATCH", uri, chosen.clone()).await,
            (StatusCode::OK, chosen.clone())
        );
        for invalid in [
            json!({"orientation_threads":0,"merge_threads":4}),
            json!({"orientation_threads":4,"merge_threads":11}),
            json!({"orientation_threads":1.5,"merge_threads":4}),
            json!({"orientation_threads":"4","merge_threads":4}),
            json!({"orientation_threads":4}),
            json!({"orientation_threads":4,"merge_threads":4,"extra":1}),
        ] {
            assert!(request_json(&state, "PATCH", uri, invalid)
                .await
                .0
                .is_client_error());
        }
        let job: Job = serde_json::from_value(json!({
            "id":"test-running", "plan_id":"test-plan", "kind":"batch_generate",
            "status":"running", "created_at":"test", "updated_at":"test",
            "total":1,"done":0,"current":"test","error":null,"results":[],
            "request":{"voucher_ids":null,"allow_unreviewed":false},"threads":4
        }))
        .unwrap();
        state.data.lock().unwrap().jobs.insert(job.id.clone(), job);
        let blocked = request_json(
            &state,
            "PATCH",
            uri,
            json!({"orientation_threads":2,"merge_threads":2}),
        )
        .await;
        assert_eq!(blocked.0, StatusCode::BAD_REQUEST);
        assert!(blocked.1["error"].as_str().unwrap().contains("任务运行中"));
        assert_eq!(
            fs::read(dir.join("settings.json")).unwrap(),
            legacy_settings
        );
        let reloaded = AppState::load(dir, &model, None).unwrap();
        assert_eq!(
            read_batch_preferences(&reloaded).unwrap(),
            BatchPreferences {
                orientation_threads: 10,
                merge_threads: 1
            }
        );
        assert_eq!(reloaded.settings().threads, 7);
    }

    #[tokio::test]
    async fn desktop_shutdown_blocks_mutations_and_keeps_status_readable() {
        let tmp = tempfile::tempdir().unwrap();
        let state =
            AppState::load(tmp.path().join("state"), &tmp.path().join("missing"), None).unwrap();
        state
            .shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
        for uri in ["/api/batch/scan", "/api/settings", "/api/batch/preferences"] {
            let (status, error) = request_json(&state, "POST", uri, json!({})).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert!(error["error"]
                .as_str()
                .unwrap()
                .contains("正在保存进度并退出"));
        }
        assert_eq!(
            request_json(&state, "GET", "/api/status", Value::Null)
                .await
                .0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn offline_pdf_preview_is_pinned_and_served_with_worker_wasm_policy() {
        use sha2::Digest;
        let manifest: Value =
            serde_json::from_slice(pdfjs_assets::get("manifest.json").unwrap()).unwrap();
        for entry in manifest["files"].as_array().unwrap() {
            let bytes = pdfjs_assets::get(entry["path"].as_str().unwrap()).unwrap();
            assert_eq!(bytes.len() as u64, entry["bytes"].as_u64().unwrap());
            assert_eq!(hex::encode(sha2::Sha256::digest(bytes)), entry["sha256"]);
        }
        let response = pdfjs_asset(Path("pdf.worker.mjs".into())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            pdfjs_asset(Path("../Cargo.toml".into())).await.status(),
            StatusCode::NOT_FOUND
        );
        let tmp = tempfile::tempdir().unwrap();
        let state =
            AppState::load(tmp.path().join("state"), &tmp.path().join("missing"), None).unwrap();
        let response = router(state)
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(csp.contains("worker-src 'self' blob:"));
        assert!(csp.contains("'wasm-unsafe-eval'"));
        assert!(!csp.contains("'unsafe-eval'"));
    }
}
