//! Optional orientation workflow, using only disposable synthetic scans.
//! No model weights, runtime, real documents or default application state.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use image::{Rgb, RgbImage};
use lopdf::Document;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use voucher_workbench::{
    jobs::{AppState, GenerateRequest, Job},
    model::{Orientation, OrientationWorkflow, Plan, ScanRequest, ORIENTATION_DISCLAIMER_VERSION},
    orientation::MODEL_ID,
    pdf::{self, Manifest},
    scan, server,
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
        for code in 1..=vouchers {
            let folder = root.join(format!("item/260101-{code:05}"));
            fs::create_dir_all(&folder).unwrap();
            for page in 1..=pages {
                RgbImage::from_fn(40, 20, |x, y| Rgb([x as u8 * 4, y as u8 * 8, 50]))
                    .save(folder.join(format!("2601_{code:05}_{page}.bmp")))
                    .unwrap();
            }
        }
        let ledger = root.join("ledger.xlsx");
        write_ledger(&ledger);
        let plan = scan::scan(&ScanRequest {
            root: root.to_string_lossy().into_owned(),
            ledger: Some(ledger.to_string_lossy().into_owned()),
            sheet: None,
        })
        .unwrap();
        assert!(plan.issues.iter().all(|i| i.severity != "error"));
        assert_eq!(plan.vouchers.len(), vouchers as usize);
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

fn write_ledger(path: &Path) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let entries = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="first" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>凭证编号</t></is></c><c r="B1" t="inlineStr"><is><t>科目名称</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>(c)00001</t></is></c><c r="B2" t="inlineStr"><is><t>synthetic</t></is></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>(c)00002</t></is></c><c r="B3" t="inlineStr"><is><t>synthetic</t></is></c></row></sheetData></worksheet>"#,
        ),
    ];
    for (name, contents) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn enabled(completed: bool) -> OrientationWorkflow {
    OrientationWorkflow {
        enabled: true,
        consent_version: Some(ORIENTATION_DISCLAIMER_VERSION.into()),
        consented_at: Some("2026-01-01T01:02:03Z".into()),
        analysis_completed: completed,
    }
}

fn cache_current_titles(plan: &mut Plan, angle: u16) {
    for file in plan.vouchers.iter_mut().flat_map(|v| &mut v.files) {
        file.orientation = Some(Orientation {
            rotation_cw: angle,
            confidence: 0.9999,
            scores: vec![],
            model: MODEL_ID.into(),
            needs_review: false,
            title: Some("合成记账凭证".into()),
            reason: "Synthetic cached OCR: no inference required".into(),
            candidates: vec![],
        });
        file.rotation_cw = angle;
    }
}

fn build(plan: &Plan, voucher: usize, bypass: bool) -> anyhow::Result<pdf::BuildResult> {
    pdf::build_voucher(
        &Path::new(&plan.root).join("state"),
        plan,
        &plan.vouchers[voucher],
        bypass,
        &AtomicBool::new(false),
        |_| {},
    )
}

fn wait_job(state: &AppState, id: &str) -> Job {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let job = state.job(id).unwrap();
        if !job.active() {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck: {}", job.current);
        thread::sleep(Duration::from_millis(10));
    }
}

fn synthetic_job(plan: &Plan, kind: &str, status: &str) -> Job {
    Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: plan.id.clone(),
        kind: kind.into(),
        status: status.into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        total: 2,
        done: 1,
        threads: 1,
        current: "synthetic interrupted stage".into(),
        error: None,
        results: vec![],
        request: GenerateRequest::default(),
    }
}

#[test]
fn new_scan_is_disabled_but_legacy_plan_without_workflow_is_fail_closed() {
    let fixture = Fixture::new(1, 2);
    assert_eq!(
        fixture.plan.orientation_workflow,
        OrientationWorkflow::disabled()
    );
    assert!(fixture.plan.vouchers[0]
        .files
        .iter()
        .all(|f| !f.reviewed && f.orientation.is_none()));
    let mut old_json = serde_json::to_value(&fixture.plan).unwrap();
    old_json
        .as_object_mut()
        .unwrap()
        .remove("orientation_workflow");
    let old: Plan = serde_json::from_value(old_json).unwrap();
    assert!(old.orientation_workflow.enabled);
    assert!(old.orientation_workflow.consent_version.is_none());
    assert!(old.orientation_workflow.consented_at.is_none());
    assert!(!old.orientation_workflow.analysis_completed);
    let state = fixture.state();
    let invalid_consents = [
        old.orientation_workflow.clone(),
        OrientationWorkflow {
            consent_version: Some("obsolete-disclaimer".into()),
            ..enabled(true)
        },
        OrientationWorkflow {
            consented_at: Some(String::new()),
            ..enabled(true)
        },
    ];
    for workflow in invalid_consents {
        let mut unconsented = old.clone();
        unconsented.orientation_workflow = workflow;
        state.insert_plan(unconsented.clone()).unwrap();
        for kind in ["analyze", "generate"] {
            let err = state
                .start(&old.id, kind, GenerateRequest::default())
                .unwrap_err();
            assert!(err.to_string().contains("免责"), "{kind}: {err:#}");
        }
        assert!(build(&unconsented, 0, false).is_err());
    }
}

#[test]
fn disabled_generation_needs_no_model_and_ignores_old_automatic_rotation() {
    let fixture = Fixture::new(1, 2);
    let mut plan = fixture.plan.clone();
    cache_current_titles(&mut plan, 90);
    plan.vouchers[0].files[0]
        .orientation
        .as_mut()
        .unwrap()
        .model = "old-classifier".into();
    plan.vouchers[0].files[0].error = Some("方向识别失败：旧运行库不可用".into());
    let state = fixture.state();
    assert!(state.model_error.is_some());
    state.insert_plan(plan.clone()).unwrap();
    let manual_id = plan.vouchers[0].files[1].id.clone();
    let updated = state.edit_files(&plan.id, &[manual_id], Some(270)).unwrap();
    assert!(!updated.vouchers[0].files[0].reviewed);
    assert_eq!(
        pdf::effective_rotation(&updated, &updated.vouchers[0].files[0]),
        0
    );
    assert_eq!(
        pdf::effective_rotation(&updated, &updated.vouchers[0].files[1]),
        270
    );
    assert!(!pdf::requires_manual_review(
        &updated,
        &updated.vouchers[0].files[0]
    ));
    let job = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    let completed = wait_job(&state, &job.id);
    assert_eq!(completed.status, "completed", "{:?}", completed.results);
    let path = Path::new(completed.results[0].output_path.as_ref().unwrap());
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(pdf::manifest_path(&state.dir, path).unwrap()).unwrap())
            .unwrap();
    assert!(!manifest.orientation_workflow.enabled);
    assert_eq!(manifest.sources[0].rotation_cw, 0);
    assert_eq!(manifest.sources[1].rotation_cw, 270);
    assert_eq!(
        manifest.sources[0].orientation,
        plan.vouchers[0].files[0].orientation
    );
    assert_eq!(manifest.sources[0].sha256, plan.vouchers[0].files[0].sha256);
    let doc = Document::load(path).unwrap();
    for ((_, page), landscape) in doc.get_pages().into_iter().zip([true, false]) {
        let bounds = doc
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            bounds[2].as_float().unwrap() > bounds[3].as_float().unwrap(),
            landscape
        );
    }
    assert!(!manifest.needs_review);
    let checked_original = state
        .edit_files(&plan.id, &[plan.vouchers[0].files[0].id.clone()], None)
        .unwrap();
    assert!(checked_original.vouchers[0].files[0].reviewed);
    assert_eq!(
        checked_original.vouchers[0].files[0].rotation_cw, 0,
        "confirming the displayed original must not silently accept a hidden old automatic angle"
    );
}

#[test]
fn high_confidence_auto_zero_still_requires_completed_analysis_and_each_human_confirmation() {
    let fixture = Fixture::new(1, 2);
    let state = fixture.state();
    let mut plan = fixture.plan.clone();
    cache_current_titles(&mut plan, 0);
    state.insert_plan(plan.clone()).unwrap();
    assert!(state
        .set_orientation_workflow(&plan.id, true, false)
        .is_err());
    let enabled_plan = state
        .set_orientation_workflow(&plan.id, true, true)
        .unwrap();
    assert_eq!(
        enabled_plan.orientation_workflow.consent_version.as_deref(),
        Some(ORIENTATION_DISCLAIMER_VERSION)
    );
    chrono::DateTime::parse_from_rfc3339(
        enabled_plan
            .orientation_workflow
            .consented_at
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let ids: Vec<_> = plan.vouchers[0]
        .files
        .iter()
        .map(|f| f.id.clone())
        .collect();
    assert!(state.edit_files(&plan.id, &ids, None).is_err());
    assert!(state.edit_files(&plan.id, &ids, Some(90)).is_err());
    assert!(state
        .start(&plan.id, "generate", GenerateRequest::default())
        .is_err());
    // Every page has cached current evidence: completion needs no model inference.
    let analysis = state
        .start(&plan.id, "analyze", GenerateRequest::default())
        .unwrap();
    assert_eq!(wait_job(&state, &analysis.id).status, "completed");
    let ready = state.plan(&plan.id).unwrap();
    assert!(ready.orientation_workflow.analysis_completed);
    assert!(!pdf::needs_review(&ready.vouchers[0].files[0]));
    assert!(pdf::requires_manual_review(
        &ready,
        &ready.vouchers[0].files[0]
    ));
    for bypass in [false, true] {
        assert!(build(&ready, 0, bypass).is_err());
        assert!(state
            .start(
                &plan.id,
                "generate",
                GenerateRequest {
                    allow_unreviewed: bypass,
                    ..Default::default()
                }
            )
            .is_err());
    }
    state.edit_files(&plan.id, &ids[..1], None).unwrap();
    assert!(state
        .start(&plan.id, "generate", GenerateRequest::default())
        .is_err());
    let confirmed = state.edit_files(&plan.id, &ids[1..], None).unwrap();
    assert!(
        build(&confirmed, 0, true).is_err(),
        "even confirmed pages cannot revive the removed bypass API"
    );
    let generated = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    assert_eq!(wait_job(&state, &generated.id).status, "completed");
}

#[test]
fn selected_voucher_can_generate_without_confirming_unselected_vouchers() {
    let fixture = Fixture::new(2, 1);
    let mut plan = fixture.plan.clone();
    cache_current_titles(&mut plan, 0);
    plan.orientation_workflow = enabled(true);
    plan.vouchers[0].files[0].reviewed = true;
    let state = fixture.state();
    state.insert_plan(plan.clone()).unwrap();
    assert!(state
        .start(&plan.id, "generate", GenerateRequest::default())
        .is_err());
    let selected = GenerateRequest {
        voucher_ids: Some(vec![plan.vouchers[0].id.clone()]),
        allow_unreviewed: false,
    };
    let job = state.start(&plan.id, "generate", selected).unwrap();
    let completed = wait_job(&state, &job.id);
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.results.len(), 1);
    assert_eq!(completed.results[0].voucher_id, plan.vouchers[0].id);
    assert!(!fixture
        .root
        .join(plan.vouchers[1].output_relative_path.as_ref().unwrap())
        .exists());
    assert!(build(&plan, 1, false).is_err());
}

#[test]
fn switching_modes_preserves_ocr_and_manual_rotations_and_requires_fresh_stage_completion() {
    let fixture = Fixture::new(1, 2);
    let mut plan = fixture.plan.clone();
    cache_current_titles(&mut plan, 90);
    plan.orientation_workflow = enabled(true);
    plan.vouchers[0].files[1].reviewed = true;
    plan.vouchers[0].files[1].rotation_cw = 270;
    let state = fixture.state();
    state.insert_plan(plan.clone()).unwrap();
    let disabled = state
        .set_orientation_workflow(&plan.id, false, false)
        .unwrap();
    assert_eq!(disabled.vouchers[0].files[0].rotation_cw, 0);
    assert_eq!(
        disabled.vouchers[0].files[0].orientation,
        plan.vouchers[0].files[0].orientation
    );
    assert_eq!(disabled.vouchers[0].files[1].rotation_cw, 270);
    assert!(disabled.vouchers[0].files[1].reviewed);
    drop(state);
    let state = fixture.state();
    assert!(!state.plan(&plan.id).unwrap().orientation_workflow.enabled);
    let restored = state
        .set_orientation_workflow(&plan.id, true, true)
        .unwrap();
    assert_eq!(restored.vouchers[0].files[0].rotation_cw, 90);
    assert_eq!(restored.vouchers[0].files[1].rotation_cw, 270);
    assert!(!restored.orientation_workflow.analysis_completed);
    assert!(state
        .edit_files(&plan.id, &[restored.vouchers[0].files[0].id.clone()], None)
        .is_err());
    assert_eq!(
        restored.vouchers[0].files[0].sha256,
        plan.vouchers[0].files[0].sha256
    );
}

#[test]
fn active_tasks_block_mode_changes_and_confirmation_without_mutating_plan() {
    let fixture = Fixture::new(1, 2);
    let mut plan = fixture.plan.clone();
    plan.orientation_workflow = enabled(true);
    let state = fixture.state();
    state.insert_plan(plan.clone()).unwrap();
    for kind in ["analyze", "generate"] {
        for status in ["queued", "running"] {
            let job = synthetic_job(&plan, kind, status);
            state
                .data
                .lock()
                .unwrap()
                .jobs
                .insert(job.id.clone(), job.clone());
            assert!(state
                .set_orientation_workflow(&plan.id, false, false)
                .is_err());
            assert!(state
                .edit_files(&plan.id, &[plan.vouchers[0].files[0].id.clone()], Some(90))
                .is_err());
            assert_eq!(
                serde_json::to_value(state.plan(&plan.id).unwrap()).unwrap(),
                serde_json::to_value(&plan).unwrap()
            );
            state.data.lock().unwrap().jobs.remove(&job.id);
        }
    }
}

#[test]
fn cancelled_or_interrupted_analysis_requires_resume_even_with_complete_cached_predictions() {
    for simulate_cancel in [true, false] {
        let fixture = Fixture::new(1, 2);
        let mut plan = fixture.plan.clone();
        cache_current_titles(&mut plan, 0);
        // Simulate the narrow final-page window: result persistence completed, but
        // the analyze job has not committed its terminal state yet.
        plan.orientation_workflow = enabled(true);
        let state = fixture.state();
        state.insert_plan(plan.clone()).unwrap();
        let mut cancelled = synthetic_job(&plan, "analyze", "running");
        if simulate_cancel {
            let flag = Arc::new(AtomicBool::new(false));
            {
                let mut data = state.data.lock().unwrap();
                data.jobs.insert(cancelled.id.clone(), cancelled.clone());
                data.cancels.insert(cancelled.id.clone(), flag.clone());
            }
            state.cancel(&cancelled.id).unwrap();
            assert!(flag.load(Ordering::Relaxed));
            assert!(
                !state
                    .plan(&plan.id)
                    .unwrap()
                    .orientation_workflow
                    .analysis_completed
            );
            cancelled.status = "cancelled".into();
        }
        pdf::atomic_json(
            &state
                .dir
                .join("jobs")
                .join(format!("{}.json", cancelled.id)),
            &cancelled,
        )
        .unwrap();
        drop(state);
        let state = fixture.state();
        assert_eq!(
            state.job(&cancelled.id).unwrap().status,
            if simulate_cancel {
                "cancelled"
            } else {
                "interrupted"
            }
        );
        assert!(
            !state
                .plan(&plan.id)
                .unwrap()
                .orientation_workflow
                .analysis_completed
        );
        assert!(state
            .edit_files(&plan.id, &[plan.vouchers[0].files[0].id.clone()], None)
            .is_err());
        assert!(state
            .start(&plan.id, "generate", GenerateRequest::default())
            .is_err());
        let resumed = state.retry(&cancelled.id).unwrap();
        assert_eq!(resumed.total, 0, "reuse the existing current OCR evidence");
        assert_eq!(wait_job(&state, &resumed.id).status, "completed");
        assert!(
            state
                .plan(&plan.id)
                .unwrap()
                .orientation_workflow
                .analysis_completed
        );
        assert!(state
            .edit_files(&plan.id, &[plan.vouchers[0].files[0].id.clone()], None)
            .is_ok());
        assert!(
            state
                .start(&plan.id, "generate", GenerateRequest::default())
                .is_err(),
            "the second page still needs human confirmation"
        );
    }
}

#[test]
fn disabling_direction_cannot_clear_source_errors_or_bypass_source_fingerprints() {
    let fixture = Fixture::new(1, 2);
    let mut plan = fixture.plan.clone();
    plan.orientation_workflow = enabled(true);
    plan.vouchers[0].files[0].error = Some("方向识别失败：运行库错误".into());
    plan.vouchers[0].files[1].error = Some("源文件损坏".into());
    let state = fixture.state();
    state.insert_plan(plan.clone()).unwrap();
    let disabled = state
        .set_orientation_workflow(&plan.id, false, false)
        .unwrap();
    assert_eq!(
        disabled.vouchers[0].files[1].error.as_deref(),
        Some("源文件损坏")
    );
    assert!(build(&disabled, 0, false).is_err());
    let mut repaired = disabled;
    repaired.vouchers[0].files[1].error = None;
    assert!(
        build(&repaired, 0, false).is_ok(),
        "only the old direction error remains, which is inactive in disabled mode"
    );
    fs::write(
        fixture
            .root
            .join(&repaired.vouchers[0].files[0].relative_path),
        b"modified source",
    )
    .unwrap();
    assert!(build(&repaired, 0, false).is_err());
    assert!(build(&repaired, 0, true).is_err());
}

#[test]
fn manifest_mode_and_consent_changes_create_history_without_changing_sources() {
    let fixture = Fixture::new(1, 2);
    let mut plan = fixture.plan.clone();
    cache_current_titles(&mut plan, 0);
    for file in &mut plan.vouchers[0].files {
        file.reviewed = true;
    }
    plan.orientation_workflow = enabled(true);
    let first = build(&plan, 0, false).unwrap();
    assert_eq!(
        first.manifest.orientation_workflow,
        plan.orientation_workflow
    );
    assert!(build(&plan, 0, false).unwrap().skipped);
    plan.orientation_workflow = OrientationWorkflow::disabled();
    let disabled = build(&plan, 0, false).unwrap();
    assert!(
        !disabled.skipped,
        "same source rotation cannot reuse output under a different workflow"
    );
    assert!(!disabled.manifest.orientation_workflow.enabled);
    plan.orientation_workflow = enabled(true);
    plan.orientation_workflow.consented_at = Some("2026-01-02T01:02:03Z".into());
    let reenabled = build(&plan, 0, false).unwrap();
    assert!(!reenabled.skipped);
    let history = pdf::manifest_path(&fixture.root.join("state"), &first.path)
        .unwrap()
        .parent()
        .unwrap()
        .join(".history");
    let mut archived = Vec::new();
    for entry in fs::read_dir(history).unwrap() {
        let path = entry
            .unwrap()
            .path()
            .join(first.path.file_name().unwrap())
            .with_extension("manifest.json");
        let manifest: Manifest = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        archived.push(manifest);
    }
    assert_eq!(archived.len(), 2);
    assert!(archived
        .iter()
        .any(|m| m.orientation_workflow == first.manifest.orientation_workflow));
    assert!(archived
        .iter()
        .any(|m| m.orientation_workflow == disabled.manifest.orientation_workflow));
    for file in &plan.vouchers[0].files {
        assert_eq!(
            pdf::sha256_file(&fixture.root.join(&file.relative_path)).unwrap(),
            file.sha256
        );
        assert!(archived.iter().all(|m| m
            .sources
            .iter()
            .any(|s| s.source == file.relative_path && s.sha256 == file.sha256)));
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
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn separate_metadata_preserves_report_snapshots_and_download_integrity_after_restart() {
    let fixture = Fixture::new(1, 1);
    let plan = &fixture.plan;
    let state = fixture.state();
    state.insert_plan(plan.clone()).unwrap();
    let first = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    assert_eq!(wait_job(&state, &first.id).status, "completed");
    let report_route = format!("/api/jobs/{}/report", first.id);
    let (status, original_report) = api(&state, "GET", &report_route, Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(
        original_report["manifests"][0]["sources"][0]["rotation_cw"],
        0
    );
    state
        .edit_files(&plan.id, &[plan.vouchers[0].files[0].id.clone()], Some(90))
        .unwrap();
    let second = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    let completed = wait_job(&state, &second.id);
    assert_eq!(completed.status, "completed");
    let output = Path::new(completed.results[0].output_path.as_ref().unwrap());
    assert!(!output.with_extension("manifest.json").exists());
    assert!(pdf::manifest_path(&state.dir, output).unwrap().exists());
    drop(state);

    let state = fixture.state();
    let (status, historic_report) = api(&state, "GET", &report_route, Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(historic_report["manifests"], original_report["manifests"]);
    let (status, current_report) = api(
        &state,
        "GET",
        &format!("/api/jobs/{}/report", second.id),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        current_report["manifests"][0]["sources"][0]["rotation_cw"],
        90
    );
    for (job, expected_status) in [(&first.id, 400), (&second.id, 200)] {
        let response = server::router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/api/jobs/{job}/outputs/{}", plan.vouchers[0].id))
                    .header("host", "127.0.0.1:43127")
                    .header("x-session-token", &state.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected_status);
        if expected_status == 200 {
            assert_eq!(response.headers()["content-type"], "application/pdf");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(bytes.as_ref(), fs::read(output).unwrap());
        }
    }
}

#[tokio::test]
async fn api_exposes_current_disclaimer_and_enforces_explicit_consent_and_stage_order() {
    let fixture = Fixture::new(1, 2);
    let state = fixture.state();
    let mut cached = fixture.plan.clone();
    cache_current_titles(&mut cached, 0);
    state.insert_plan(cached).unwrap();
    let (status, payload) = api(&state, "GET", "/api/status", Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(
        payload["orientation_disclaimer_version"],
        ORIENTATION_DISCLAIMER_VERSION
    );
    let (_, listing) = api(&state, "GET", "/api/plans", Value::Null).await;
    assert_eq!(listing[0]["review_count"], 0);
    assert_eq!(listing[0]["algorithm_unresolved_count"], 0);
    let route = format!("/api/plans/{}/orientation", fixture.plan.id);
    assert_eq!(
        api(&state, "PATCH", &route, json!({"enabled":true}))
            .await
            .0,
        400
    );
    let (status, plan) = api(
        &state,
        "PATCH",
        &route,
        json!({"enabled":true,"accept_disclaimer":true}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        plan["orientation_workflow"]["consent_version"],
        ORIENTATION_DISCLAIMER_VERSION
    );
    assert_eq!(plan["orientation_workflow"]["analysis_completed"], false);
    let (_, listing) = api(&state, "GET", "/api/plans", Value::Null).await;
    assert_eq!(
        listing[0]["review_count"], 2,
        "all enabled images require human confirmation"
    );
    assert_eq!(
        listing[0]["algorithm_unresolved_count"], 0,
        "clear model suggestions are not human confirmations"
    );
    assert_eq!(
        api(
            &state,
            "POST",
            &format!("/api/plans/{}/review", fixture.plan.id),
            json!({"file_ids":[fixture.plan.vouchers[0].files[0].id]})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        api(
            &state,
            "POST",
            &format!("/api/plans/{}/generate", fixture.plan.id),
            json!({"allow_unreviewed":true})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        api(&state, "PATCH", &route, json!({"enabled":false}))
            .await
            .0,
        200
    );
    assert_eq!(
        api(
            &state,
            "POST",
            &format!("/api/plans/{}/generate", fixture.plan.id),
            json!({"allow_unreviewed":true})
        )
        .await
        .0,
        400
    );
}
