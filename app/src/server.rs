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
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

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
    next.run(request).await
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert("cache-control", "no-store".parse().unwrap());
    h.insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; frame-src 'self' blob:; object-src blob:; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'".parse().unwrap());
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
    if job.kind == "generic_generate" {
        return Ok(Json(crate::generic::report(&s, &job)?));
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
    Ok(Json(
        json!({"job":job,"plan_id":plan.id,"root":plan.root,"ledger":plan.ledger_path,"sheet":plan.sheet,"issues":plan.issues,"manifests":manifests}),
    ))
}
async fn output(
    State(s): State<Arc<AppState>>,
    Path((job_id, voucher_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let job = s.job(&job_id)?;
    if job.kind == "generic_generate" {
        let path = crate::generic::output(&s, &job, &voucher_id)?;
        return Ok(bytes_response(
            tokio::fs::read(path).await?,
            "application/pdf",
        ));
    }
    let result = job
        .results
        .iter()
        .find(|r| r.voucher_id == voucher_id && r.output_path.is_some())
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
            .join(format!("{}.json", voucher_id)),
    )?)?;
    if pdf::sha256_file(&path)? != record.output_sha256 {
        return Err(anyhow::anyhow!(
            "该输出已更新，请从最新任务下载；历史版本保存在输出目录的 .history 中"
        )
        .into());
    }
    Ok(bytes_response(
        tokio::fs::read(path).await?,
        "application/pdf",
    ))
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

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/settings", get(settings).patch(update_settings))
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
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Html(include_str!("../web/index.html")) }),
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
        for uri in ["/api/status", "/api/generic/plans"] {
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
}
