//! Bounded parallel jobs using generated inputs, never customer documents or
//! the default application state. OCR inference is exercised by package smoke;
//! these tests also cover cached/manual workflows with no runtime installed.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use image::{Rgb, RgbImage};
use lopdf::Document;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use voucher_workbench::{
    jobs::{AppState, GenerateRequest, Job, ProcessingSettings},
    model::{
        Issue, Orientation, OrientationWorkflow, Plan, SourceFile, Voucher,
        ORIENTATION_DISCLAIMER_VERSION,
    },
    orientation::MODEL_ID,
    pdf::{self, Manifest},
    server,
};

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    plan: Plan,
}

impl Fixture {
    fn new(vouchers: u32, pages: u32) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let ledger = root.join("synthetic-ledger.txt");
        fs::write(&ledger, b"generated fingerprint fixture, not a workbook").unwrap();
        let mut items = vec![];
        for code in 1..=vouchers {
            let relative = format!("synthetic/260101-{code:05}");
            let folder = root.join(&relative);
            fs::create_dir_all(&folder).unwrap();
            let mut files = vec![];
            for page in 1..=pages {
                let name = format!("2601_{code:05}_{page}.jpg");
                let path = folder.join(&name);
                RgbImage::from_fn(64, 48, |x, y| {
                    Rgb([(x + page) as u8, (y + code) as u8, (page * 5) as u8])
                })
                .save(&path)
                .unwrap();
                let meta = fs::metadata(&path).unwrap();
                files.push(SourceFile {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    relative_path: format!("{relative}/2601_{code:05}_{page}.jpg"),
                    sequence: page,
                    kind: "image".into(),
                    pages: 1,
                    bytes: meta.len(),
                    modified_ms: meta
                        .modified()
                        .unwrap()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                    sha256: pdf::sha256_file(&path).unwrap(),
                    width: Some(64),
                    height: Some(48),
                    orientation: None,
                    rotation_cw: 0,
                    reviewed: false,
                    error: None,
                });
            }
            items.push(Voucher {
                id: uuid::Uuid::new_v4().to_string(),
                item: "synthetic".into(),
                folder: format!("260101-{code:05}"),
                relative_path: relative,
                code: format!("{code:05}"),
                subject: Some("generated".into()),
                ledger_row: Some(code + 1),
                duplicate_rows: vec![],
                output_relative_path: Some(format!("synthetic-PDF/260101-{code:05}-generated.pdf")),
                files,
                issues: vec![],
            });
        }
        let plan = Plan {
            id: uuid::Uuid::new_v4().to_string(),
            root: root.to_string_lossy().into(),
            requested_root: Some(root.to_string_lossy().into()),
            ledger_path: ledger.to_string_lossy().into(),
            ledger_sha256: pdf::sha256_file(&ledger).unwrap(),
            sheet: "synthetic first".into(),
            sheets: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
            vouchers: items,
            issues: vec![],
            ignored: 0,
            orientation_workflow: OrientationWorkflow::disabled(),
        };
        Self {
            _temp: temp,
            root,
            plan,
        }
    }

    fn state(&self) -> Arc<AppState> {
        AppState::load(
            self.root.join("state"),
            &self.root.join("missing-model"),
            None,
        )
        .unwrap()
    }
}

fn wait_job(state: &AppState, id: &str) -> Job {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut done = 0;
    loop {
        let job = state.job(id).unwrap();
        assert!(job.done >= done, "progress regressed");
        assert!(job.done <= job.total, "parallel pages counted twice");
        done = job.done;
        if !job.active() {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck: {}", job.current);
        thread::sleep(Duration::from_millis(2));
    }
}

fn verify_voucher(state: &AppState, plan: &Plan, voucher: &Voucher) {
    let output = Path::new(&plan.root).join(voucher.output_relative_path.as_ref().unwrap());
    assert!(!output.with_extension("manifest.json").exists());
    let record: Manifest = serde_json::from_slice(
        &fs::read(pdf::manifest_path(&state.dir, &output).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(record.output_sha256, pdf::sha256_file(&output).unwrap());
    assert_eq!(record.sources.len(), voucher.files.len());
    let document = Document::load(output).unwrap();
    let pages = document.get_pages();
    assert_eq!(pages.len(), voucher.files.len());
    for ((source, mapping), (_, id)) in voucher.files.iter().zip(&record.sources).zip(pages) {
        assert_eq!(mapping.source, source.relative_path);
        assert_eq!(
            (mapping.first_page, mapping.last_page),
            (source.sequence, source.sequence)
        );
        let page = document.get_object(id).unwrap().as_dict().unwrap();
        let image_ref = page
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"XObject")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Scan")
            .unwrap();
        let image = document
            .dereference(image_ref)
            .unwrap()
            .1
            .as_stream()
            .unwrap();
        assert_eq!(
            image.content,
            fs::read(Path::new(&plan.root).join(&source.relative_path)).unwrap(),
            "page order/source bytes changed during parallel generation"
        );
    }
}

async fn api(state: &Arc<AppState>, method: &str, uri: &str, body: Value) -> (u16, Value) {
    let response = server::router(state.clone())
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
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

#[tokio::test]
async fn settings_reject_invalid_values_and_persist_between_restarts() {
    let fixture = Fixture::new(1, 1);
    let state = fixture.state();
    assert_eq!(
        api(&state, "GET", "/api/settings", Value::Null).await,
        (200, json!({"threads": 1}))
    );
    for threads in [1, 10] {
        assert_eq!(
            api(&state, "PATCH", "/api/settings", json!({"threads":threads})).await,
            (200, json!({"threads":threads}))
        );
    }
    let saved = fs::read(state.dir.join("settings.json")).unwrap();
    for invalid in [
        json!(0),
        json!(11),
        json!(-1),
        json!(1.5),
        json!("4"),
        Value::Null,
    ] {
        let (status, _) = api(&state, "PATCH", "/api/settings", json!({"threads":invalid})).await;
        assert!((400..500).contains(&status), "accepted {invalid}");
        assert_eq!(state.settings().threads, 10);
        assert_eq!(fs::read(state.dir.join("settings.json")).unwrap(), saved);
    }
    drop(state);
    assert_eq!(fixture.state().settings().threads, 10);
}

#[tokio::test]
async fn active_jobs_block_settings_changes_without_modifying_preferences() {
    let fixture = Fixture::new(1, 1);
    let state = fixture.state();
    state
        .set_settings(ProcessingSettings { threads: 3 })
        .unwrap();
    let saved = fs::read(state.dir.join("settings.json")).unwrap();
    let job = synthetic_job(&fixture.plan, "queued", 3);
    state.data.lock().unwrap().jobs.insert(job.id.clone(), job);
    let (status, _) = api(&state, "PATCH", "/api/settings", json!({"threads":10})).await;
    assert_eq!(status, 400);
    assert_eq!(state.settings().threads, 3);
    assert_eq!(fs::read(state.dir.join("settings.json")).unwrap(), saved);
}

fn synthetic_job(plan: &Plan, status: &str, threads: usize) -> Job {
    Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: plan.id.clone(),
        kind: "generate".into(),
        status: status.into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        total: 1,
        done: 0,
        threads,
        current: "synthetic job".into(),
        error: None,
        results: vec![],
        request: GenerateRequest::default(),
    }
}

#[test]
fn legacy_jobs_default_to_one_thread_and_retries_keep_the_job_snapshot() {
    let fixture = Fixture::new(2, 2);
    let state = fixture.state();
    state.insert_plan(fixture.plan.clone()).unwrap();
    state
        .set_settings(ProcessingSettings { threads: 10 })
        .unwrap();
    let legacy = synthetic_job(&fixture.plan, "completed", 1);
    let mut value = serde_json::to_value(&legacy).unwrap();
    value.as_object_mut().unwrap().remove("threads");
    pdf::atomic_json(
        &state.dir.join("jobs").join(format!("{}.json", legacy.id)),
        &value,
    )
    .unwrap();
    drop(state);
    let state = fixture.state();
    assert_eq!(state.job(&legacy.id).unwrap().threads, 1);
    let retry = state.retry(&legacy.id).unwrap();
    assert_eq!(retry.threads, 1);
    assert_eq!(wait_job(&state, &retry.id).status, "completed");
    let fresh = state
        .start(&fixture.plan.id, "generate", GenerateRequest::default())
        .unwrap();
    assert_eq!(fresh.threads, 10);
    assert_eq!(wait_job(&state, &fresh.id).status, "completed");
}

#[test]
fn one_and_ten_threads_preserve_every_voucher_page_and_result_order() {
    for threads in [1, 10] {
        let fixture = Fixture::new(12, 3);
        let state = fixture.state();
        state.insert_plan(fixture.plan.clone()).unwrap();
        state.set_settings(ProcessingSettings { threads }).unwrap();
        let job = state
            .start(&fixture.plan.id, "generate", GenerateRequest::default())
            .unwrap();
        assert_eq!(job.threads, threads);
        let finished = wait_job(&state, &job.id);
        assert_eq!(finished.status, "completed", "{:?}", finished.error);
        assert_eq!((finished.done, finished.total), (36, 36));
        assert_eq!(
            finished
                .results
                .iter()
                .map(|r| &r.voucher_id)
                .collect::<Vec<_>>(),
            fixture
                .plan
                .vouchers
                .iter()
                .map(|v| &v.id)
                .collect::<Vec<_>>()
        );
        for voucher in &fixture.plan.vouchers {
            verify_voucher(&state, &fixture.plan, voucher);
        }
    }
}

#[test]
fn failed_voucher_does_not_stop_siblings_and_retry_retains_ten_threads() {
    let fixture = Fixture::new(6, 3);
    let state = fixture.state();
    let mut plan = fixture.plan.clone();
    plan.vouchers[2].issues.push(Issue {
        severity: "error".into(),
        message: "generated invalid voucher".into(),
    });
    state.insert_plan(plan.clone()).unwrap();
    state
        .set_settings(ProcessingSettings { threads: 10 })
        .unwrap();
    let job = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    let finished = wait_job(&state, &job.id);
    assert_eq!(finished.status, "completed_with_errors");
    assert_eq!(
        (finished.done, finished.total, finished.results.len()),
        (18, 18, 6)
    );
    assert_eq!(finished.results[2].status, "failed");
    for (index, voucher) in plan.vouchers.iter().enumerate() {
        if index != 2 {
            verify_voucher(&state, &plan, voucher);
        }
    }
    plan.vouchers[2].issues.clear();
    state.insert_plan(plan.clone()).unwrap();
    state
        .set_settings(ProcessingSettings { threads: 1 })
        .unwrap();
    let retry = state.retry(&job.id).unwrap();
    assert_eq!(retry.threads, 10);
    let retried = wait_job(&state, &retry.id);
    assert_eq!(retried.status, "completed");
    assert_eq!(
        retried
            .results
            .iter()
            .filter(|r| r.status == "skipped")
            .count(),
        5
    );
    for voucher in &plan.vouchers {
        verify_voucher(&state, &plan, voucher);
    }
}

#[test]
fn cancellation_and_restart_keep_verified_outputs_and_retry_settings() {
    let fixture = Fixture::new(32, 5);
    let state = fixture.state();
    state.insert_plan(fixture.plan.clone()).unwrap();
    state
        .set_settings(ProcessingSettings { threads: 10 })
        .unwrap();
    let first = pdf::build_voucher(
        &state.dir,
        &fixture.plan,
        &fixture.plan.vouchers[0],
        false,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let bytes = fs::read(&first.path).unwrap();
    let job = state
        .start(&fixture.plan.id, "generate", GenerateRequest::default())
        .unwrap();
    state.cancel(&job.id).unwrap();
    let cancelled = wait_job(&state, &job.id);
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(fs::read(&first.path).unwrap(), bytes);
    for result in &cancelled.results {
        if result.status == "completed" || result.status == "skipped" {
            let voucher = fixture
                .plan
                .vouchers
                .iter()
                .find(|v| v.id == result.voucher_id)
                .unwrap();
            verify_voucher(&state, &fixture.plan, voucher);
        }
    }
    drop(state);
    let state = fixture.state();
    assert_eq!(state.settings().threads, 10);
    state
        .set_settings(ProcessingSettings { threads: 1 })
        .unwrap();
    let retry = state.retry(&job.id).unwrap();
    assert_eq!(retry.threads, 10);
    let finished = wait_job(&state, &retry.id);
    assert_eq!(finished.status, "completed");
    assert_eq!((finished.done, finished.total), (160, 160));
    assert_eq!(finished.results[0].status, "skipped");
    assert_eq!(fs::read(&first.path).unwrap(), bytes);
    for voucher in &fixture.plan.vouchers {
        verify_voucher(&state, &fixture.plan, voucher);
    }
}

#[test]
fn cached_analysis_at_ten_threads_still_requires_every_manual_confirmation() {
    let fixture = Fixture::new(2, 2);
    let state = fixture.state();
    let mut plan = fixture.plan.clone();
    plan.orientation_workflow = OrientationWorkflow {
        enabled: true,
        consent_version: Some(ORIENTATION_DISCLAIMER_VERSION.into()),
        consented_at: Some("2026-01-01T00:00:00Z".into()),
        analysis_completed: false,
    };
    for file in plan.vouchers.iter_mut().flat_map(|v| &mut v.files) {
        file.orientation = Some(Orientation {
            rotation_cw: 0,
            confidence: 0.99,
            scores: vec![],
            model: MODEL_ID.into(),
            needs_review: false,
            title: Some("合成记账凭证".into()),
            reason: "generated cached evidence".into(),
            candidates: vec![],
        });
    }
    state.insert_plan(plan.clone()).unwrap();
    state
        .set_settings(ProcessingSettings { threads: 10 })
        .unwrap();
    let job = state
        .start(&plan.id, "analyze", GenerateRequest::default())
        .unwrap();
    assert_eq!(job.threads, 10);
    let analyzed = wait_job(&state, &job.id);
    assert_eq!(
        (analyzed.status.as_str(), analyzed.done, analyzed.total),
        ("completed", 0, 0)
    );
    let current = state.plan(&plan.id).unwrap();
    assert!(current.orientation_workflow.analysis_completed);
    assert!(current
        .vouchers
        .iter()
        .flat_map(|v| &v.files)
        .all(|f| !f.reviewed));
    assert!(state
        .start(&plan.id, "generate", GenerateRequest::default())
        .is_err());
    let ids: Vec<_> = current
        .vouchers
        .iter()
        .flat_map(|v| &v.files)
        .map(|f| f.id.clone())
        .collect();
    state
        .edit_files(&plan.id, &ids[..ids.len() - 1], None)
        .unwrap();
    assert!(state
        .start(&plan.id, "generate", GenerateRequest::default())
        .is_err());
    state
        .edit_files(&plan.id, &ids[ids.len() - 1..], None)
        .unwrap();
    let generated = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    assert_eq!(wait_job(&state, &generated.id).status, "completed");
}
