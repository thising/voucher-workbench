//! Synthetic end-to-end integrity tests. No business documents or model weights
//! are needed: the fixtures deliberately exercise human-confirmed orientations.
use std::{
    fs::{self, File, FileTimes, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use image::{Rgb, RgbImage};
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, ObjectId, Stream,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use voucher_workbench::{
    jobs::{AppState, GenerateRequest, Job},
    model::{
        Orientation, OrientationWorkflow, Plan, SourceFile, Voucher, ORIENTATION_DISCLAIMER_VERSION,
    },
    pdf::{self, BuildResult, Manifest},
};

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("测试分项/260101-00001")).unwrap();
        // build_voucher consumes the ledger fingerprint; ledger parsing has its
        // own tests. This is plainly synthetic, not a disguised Excel workbook.
        fs::write(root.join("synthetic-ledger.txt"), b"synthetic-ledger-v1").unwrap();
        Self { _temp: temp, root }
    }

    fn image(&self, sequence: u32, width: u32, height: u32) -> SourceFile {
        let relative = format!("测试分项/260101-00001/2601_00001_{sequence}.bmp");
        let path = self.root.join(&relative);
        let image = RgbImage::from_fn(width, height, |x, y| {
            if x < width / 2 && y < height / 2 {
                Rgb([230, 20, 30])
            } else if x >= width / 2 && y < height / 2 {
                Rgb([30, 210, 50])
            } else if x < width / 2 {
                Rgb([30, 60, 220])
            } else {
                Rgb([250, 240, 190])
            }
        });
        image.save(&path).unwrap();
        self.source(relative, sequence, "image", 1, Some(width), Some(height))
    }

    fn source(
        &self,
        relative: String,
        sequence: u32,
        kind: &str,
        pages: u32,
        width: Option<u32>,
        height: Option<u32>,
    ) -> SourceFile {
        let path = self.root.join(&relative);
        let meta = fs::metadata(&path).unwrap();
        SourceFile {
            id: uuid::Uuid::new_v4().to_string(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            relative_path: relative,
            sequence,
            kind: kind.into(),
            pages,
            bytes: meta.len(),
            modified_ms: meta
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            sha256: pdf::sha256_file(&path).unwrap(),
            width,
            height,
            orientation: None,
            rotation_cw: 0,
            reviewed: true,
            error: None,
        }
    }

    fn multipage_pdf(&self, sequence: u32) -> SourceFile {
        let relative = format!("测试分项/260101-00001/2601_00001_{sequence}.pdf");
        let path = self.root.join(&relative);
        let mut doc = Document::with_version("1.7");
        let root_pages = doc.new_object_id();
        let branch = doc.new_object_id();
        let font = doc.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        );
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let inherited_box = doc.add_object(Object::Array(vec![
            0.into(),
            0.into(),
            321.into(),
            654.into(),
        ]));
        let mut pages = Vec::new();
        for index in 1..=2 {
            let contents = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                    Operation::new("Td", vec![10.into(), 20.into()]),
                    Operation::new(
                        "Tj",
                        vec![Object::string_literal(format!("SYNTHETIC_PAGE_{index}"))],
                    ),
                    Operation::new("ET", vec![]),
                ],
            };
            let stream = doc.add_object(Stream::new(Dictionary::new(), contents.encode().unwrap()));
            let mut page =
                dictionary! { "Type" => "Page", "Parent" => branch, "Contents" => stream };
            if index == 2 {
                page.set("MediaBox", vec![0.into(), 0.into(), 600.into(), 400.into()]);
                page.set("Rotate", 270);
            }
            pages.push(Object::Reference(doc.add_object(page)));
        }
        doc.objects.insert(branch, Object::Dictionary(dictionary! {
            "Type" => "Pages", "Parent" => root_pages, "Kids" => pages, "Count" => 2,
            "MediaBox" => inherited_box, "CropBox" => vec![3.into(), 4.into(), 300.into(), 390.into()], "Rotate" => 90,
        }));
        doc.objects.insert(root_pages, Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(branch)], "Count" => 2, "Resources" => resources,
        }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root_pages });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
        self.source(relative, sequence, "pdf", 2, None, None)
    }

    fn plan(&self, files: Vec<SourceFile>) -> Plan {
        let ledger = self.root.join("synthetic-ledger.txt");
        Plan {
            id: uuid::Uuid::new_v4().to_string(),
            root: self.root.to_string_lossy().into_owned(),
            requested_root: Some(self.root.to_string_lossy().into_owned()),
            ledger_path: ledger.to_string_lossy().into_owned(),
            ledger_sha256: pdf::sha256_file(&ledger).unwrap(),
            sheet: "首工作表".into(),
            sheets: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
            orientation_workflow: OrientationWorkflow::disabled(),
            vouchers: vec![Voucher {
                id: uuid::Uuid::new_v4().to_string(),
                item: "测试分项".into(),
                folder: "260101-00001".into(),
                relative_path: "测试分项/260101-00001".into(),
                code: "00001".into(),
                subject: Some("合成科目".into()),
                ledger_row: Some(8),
                duplicate_rows: vec![9],
                output_relative_path: Some("测试分项-PDF/260101-00001-合成科目.pdf".into()),
                files,
                issues: vec![],
            }],
            issues: vec![],
            ignored: 0,
        }
    }

    fn state(&self) -> std::sync::Arc<AppState> {
        // The missing model is intentional: manual review must remain usable.
        AppState::load(
            self.root.join("state"),
            &self.root.join("missing-model.onnx"),
            None,
        )
        .unwrap()
    }
}

fn output_path(plan: &Plan) -> PathBuf {
    Path::new(&plan.root).join(plan.vouchers[0].output_relative_path.as_ref().unwrap())
}

fn state_dir(plan: &Plan) -> PathBuf {
    Path::new(&plan.root).join("state")
}

fn record_path(plan: &Plan) -> PathBuf {
    pdf::manifest_path(&state_dir(plan), &output_path(plan)).unwrap()
}

fn build(plan: &Plan) -> anyhow::Result<BuildResult> {
    pdf::build_voucher(
        &state_dir(plan),
        plan,
        &plan.vouchers[0],
        false,
        &AtomicBool::new(false),
        |_| {},
    )
}

fn error(result: anyhow::Result<BuildResult>) -> String {
    match result {
        Ok(_) => panic!("expected build failure"),
        Err(err) => format!("{err:#}"),
    }
}

fn box_values(doc: &Document, page: ObjectId, key: &[u8]) -> Vec<f32> {
    let value = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(key)
        .unwrap();
    doc.dereference(value)
        .unwrap()
        .1
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_float().unwrap())
        .collect()
}

fn wait_job(state: &AppState, id: &str) -> Job {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let job = state.job(id).unwrap();
        if !job.active() {
            return job;
        }
        assert!(
            Instant::now() < deadline,
            "job did not terminate: {}",
            job.current
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn image_quarter_turns_select_a4_and_preserve_rotation_direction() {
    let fixture = Fixture::new();
    let files = [0, 90, 180, 270]
        .into_iter()
        .enumerate()
        .map(|(index, angle)| {
            let mut file = fixture.image(index as u32 + 1, 40, 20);
            file.rotation_cw = angle;
            file
        })
        .collect();
    let plan = fixture.plan(files);
    let out = build(&plan).unwrap();
    let doc = Document::load(out.path).unwrap();
    assert_eq!(doc.get_pages().len(), 4);
    for ((number, id), angle) in doc.get_pages().into_iter().zip([0, 90, 180, 270]) {
        let bounds = box_values(&doc, id, b"MediaBox");
        let (w, h) = (bounds[2], bounds[3]);
        assert!((w.min(h) - 595.2756).abs() < 0.01);
        assert!((w.max(h) - 841.8898).abs() < 0.01);
        assert_eq!(w > h, angle % 180 == 0);
        let content = Content::decode(&doc.get_page_content(id).unwrap()).unwrap();
        let transform = content
            .operations
            .iter()
            .find(|op| op.operator == "cm")
            .unwrap();
        let m: Vec<_> = transform
            .operands
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect();
        if angle == 90 {
            assert!(m[1] < 0.0 && m[2] > 0.0);
        }
        if angle == 270 {
            assert!(m[1] > 0.0 && m[2] < 0.0);
        }
        assert!(
            doc.extract_text(&[number]).unwrap().trim().is_empty(),
            "no added header, footer or page number"
        );
    }
}

#[test]
fn mixed_sources_keep_page_order_pdf_inheritance_and_exact_page_count() {
    let fixture = Fixture::new();
    let first = fixture.image(1, 40, 20);
    let mut pdf_file = fixture.multipage_pdf(2);
    pdf_file.rotation_cw = 90;
    let original_hash = pdf_file.sha256.clone();
    let last = fixture.image(3, 20, 40);
    let plan = fixture.plan(vec![first, pdf_file, last]);
    let out = build(&plan).unwrap();
    let doc = Document::load(&out.path).unwrap();
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 4, "no covers or separators may be inserted");
    assert!(doc.extract_text(&[2]).unwrap().contains("SYNTHETIC_PAGE_1"));
    assert!(doc.extract_text(&[3]).unwrap().contains("SYNTHETIC_PAGE_2"));
    assert_eq!(
        box_values(&doc, pages[&2], b"MediaBox"),
        vec![0.0, 0.0, 321.0, 654.0]
    );
    assert_eq!(
        box_values(&doc, pages[&3], b"MediaBox"),
        vec![0.0, 0.0, 600.0, 400.0]
    );
    for (number, rotation) in [(2, 180), (3, 0)] {
        let page = doc.get_object(pages[&number]).unwrap().as_dict().unwrap();
        assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), rotation);
        assert_eq!(
            box_values(&doc, pages[&number], b"CropBox"),
            vec![3.0, 4.0, 300.0, 390.0]
        );
        let resources = doc
            .dereference(page.get(b"Resources").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        let font = doc
            .dereference(fonts.get(b"F1").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        assert_eq!(
            font.get(b"BaseFont").unwrap().as_name().unwrap(),
            b"Helvetica"
        );
    }
    let ranges: Vec<_> = out
        .manifest
        .sources
        .iter()
        .map(|m| (m.first_page, m.last_page))
        .collect();
    assert_eq!(ranges, [(1, 1), (2, 3), (4, 4)]);
    assert_eq!(out.manifest.sources[1].sha256, original_hash);
    assert_eq!(
        pdf::sha256_file(&fixture.root.join(&plan.vouchers[0].files[1].relative_path)).unwrap(),
        original_hash
    );
}

#[test]
fn cancellation_leaves_no_formal_or_partial_new_output() {
    for page_count in [1, 2] {
        let fixture = Fixture::new();
        let plan = fixture.plan((1..=page_count).map(|n| fixture.image(n, 40, 20)).collect());
        let cancel = AtomicBool::new(false);
        let result = pdf::build_voucher(
            &state_dir(&plan),
            &plan,
            &plan.vouchers[0],
            false,
            &cancel,
            |_| cancel.store(true, Ordering::Relaxed),
        );
        assert!(error(result).contains("取消"));
        assert!(!output_path(&plan).exists());
        assert!(!output_path(&plan).with_extension("manifest.json").exists());
        assert_eq!(
            fs::read_dir(output_path(&plan).parent().unwrap())
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn correct_outputs_are_reused_and_changed_directions_are_archived() {
    let fixture = Fixture::new();
    let mut plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let first = build(&plan).unwrap();
    let bytes = fs::read(&first.path).unwrap();
    let again = build(&plan).unwrap();
    assert!(again.skipped);
    assert_eq!(first.manifest.output_sha256, again.manifest.output_sha256);
    assert_eq!(first.manifest.created_at, again.manifest.created_at);
    assert_eq!(fs::read(&first.path).unwrap(), bytes);
    plan.vouchers[0].files[0].rotation_cw = 270;
    let changed = build(&plan).unwrap();
    assert!(!changed.skipped);
    assert_ne!(changed.manifest.output_sha256, first.manifest.output_sha256);
    let history = first.path.parent().unwrap().join(".history");
    let archive = fs::read_dir(history)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let old_pdf = archive.join(first.path.file_name().unwrap());
    assert_eq!(fs::read(&old_pdf).unwrap(), bytes);
    let old_manifest: Manifest = serde_json::from_slice(
        &fs::read(
            record_path(&plan)
                .parent()
                .unwrap()
                .join(".history")
                .join(archive.file_name().unwrap())
                .join(first.path.file_name().unwrap())
                .with_extension("manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(old_manifest.output_sha256, first.manifest.output_sha256);
}

#[test]
fn cancellation_during_rebuild_preserves_previous_deliverable() {
    let fixture = Fixture::new();
    let mut plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let original = build(&plan).unwrap();
    let pdf_bytes = fs::read(&original.path).unwrap();
    let manifest_bytes = fs::read(record_path(&plan)).unwrap();
    plan.vouchers[0].files[0].rotation_cw = 90;
    let cancel = AtomicBool::new(false);
    let result = pdf::build_voucher(
        &state_dir(&plan),
        &plan,
        &plan.vouchers[0],
        false,
        &cancel,
        |_| cancel.store(true, Ordering::Relaxed),
    );
    assert!(error(result).contains("取消"));
    assert_eq!(fs::read(&original.path).unwrap(), pdf_bytes);
    assert_eq!(fs::read(record_path(&plan)).unwrap(), manifest_bytes);
}

#[test]
fn unknown_or_modified_existing_outputs_are_never_overwritten() {
    let fixture = Fixture::new();
    let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let path = output_path(&plan);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"an unrelated user document").unwrap();
    assert!(error(build(&plan)).contains("拒绝覆盖"));
    assert_eq!(fs::read(&path).unwrap(), b"an unrelated user document");
    fs::remove_file(&path).unwrap();
    build(&plan).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"external edit")
        .unwrap();
    let altered = fs::read(&path).unwrap();
    assert!(error(build(&plan)).contains("拒绝覆盖"));
    assert_eq!(fs::read(&path).unwrap(), altered);
}

#[test]
fn generated_output_has_only_pdf_and_keeps_records_in_state() {
    let fixture = Fixture::new();
    let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let output = build(&plan).unwrap();
    let names: Vec<_> = fs::read_dir(output.path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [output.path.file_name().unwrap()]);
    let record = record_path(&plan);
    assert!(record.starts_with(fixture.root.join("state/outputs")));
    let saved: Manifest = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    assert_eq!(saved.output_sha256, output.manifest.output_sha256);
    assert!(build(&plan).unwrap().skipped);
}

#[test]
fn owned_legacy_sidecar_migrates_and_reuses_pdf_even_after_interrupted_copy() {
    for keep_copied_record in [false, true] {
        let fixture = Fixture::new();
        let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
        let first = build(&plan).unwrap();
        let pdf_bytes = fs::read(&first.path).unwrap();
        let current = record_path(&plan);
        let legacy = first.path.with_extension("manifest.json");
        let original = fs::read(&current).unwrap();
        fs::write(&legacy, &original).unwrap();
        if !keep_copied_record {
            fs::remove_file(&current).unwrap();
        }
        let next = build(&plan).unwrap();
        assert!(next.skipped);
        assert_eq!(fs::read(&first.path).unwrap(), pdf_bytes);
        assert_eq!(fs::read(&current).unwrap(), original);
        assert!(!legacy.exists());
        assert!(build(&plan).unwrap().skipped);
    }
}

#[test]
fn equivalent_legacy_previous_copy_is_removed_before_it_can_be_restored() {
    let fixture = Fixture::new();
    let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let first = build(&plan).unwrap();
    let current = record_path(&plan);
    let legacy = first.path.with_extension("manifest.json");
    let previous = legacy.with_extension("json.previous");
    let original = fs::read(&current).unwrap();
    // Different JSON whitespace must not turn equivalent records into a conflict.
    let compact = serde_json::to_vec(&first.manifest).unwrap();
    assert_ne!(original, compact);
    fs::write(&legacy, &original).unwrap();
    fs::write(&previous, compact).unwrap();
    fs::remove_file(&current).unwrap();
    for _ in 0..2 {
        assert!(build(&plan).unwrap().skipped);
        assert_eq!(fs::read(&current).unwrap(), original);
        assert_eq!(
            pdf::sha256_file(&first.path).unwrap(),
            first.manifest.output_sha256
        );
        assert!(!legacy.exists());
        assert!(!previous.exists());
    }
}

#[test]
fn lone_legacy_previous_copy_is_recovered_and_migrated() {
    let fixture = Fixture::new();
    let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let first = build(&plan).unwrap();
    let current = record_path(&plan);
    let legacy = first.path.with_extension("manifest.json");
    let previous = legacy.with_extension("json.previous");
    let original = fs::read(&current).unwrap();
    fs::write(&previous, &original).unwrap();
    fs::remove_file(&current).unwrap();
    for _ in 0..2 {
        assert!(build(&plan).unwrap().skipped);
        assert_eq!(fs::read(&current).unwrap(), original);
        assert_eq!(
            pdf::sha256_file(&first.path).unwrap(),
            first.manifest.output_sha256
        );
        assert!(!legacy.exists());
        assert!(!previous.exists());
    }
}

#[test]
fn conflicting_or_invalid_legacy_previous_copy_preserves_both_originals() {
    for invalid_json in [false, true] {
        let fixture = Fixture::new();
        let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
        let first = build(&plan).unwrap();
        let current = record_path(&plan);
        let legacy = first.path.with_extension("manifest.json");
        let previous = legacy.with_extension("json.previous");
        let original = fs::read(&current).unwrap();
        let mut conflicting = first.manifest.clone();
        conflicting.created_at = "different historical record".into();
        let previous_bytes = if invalid_json {
            b"invalid previous JSON".to_vec()
        } else {
            serde_json::to_vec(&conflicting).unwrap()
        };
        fs::write(&legacy, &original).unwrap();
        fs::write(&previous, &previous_bytes).unwrap();
        fs::remove_file(&current).unwrap();
        for _ in 0..2 {
            assert!(build(&plan).is_err());
            assert!(!current.exists(), "conflicting record must not be adopted");
            assert_eq!(fs::read(&legacy).unwrap(), original);
            assert_eq!(fs::read(&previous).unwrap(), previous_bytes);
            assert_eq!(
                pdf::sha256_file(&first.path).unwrap(),
                first.manifest.output_sha256
            );
        }
    }
}

#[test]
fn invalid_legacy_records_are_preserved_without_adopting_outputs() {
    for invalid in [
        "producer",
        "root",
        "folder",
        "output_sha256",
        "invalid-json",
    ] {
        let fixture = Fixture::new();
        let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
        let first = build(&plan).unwrap();
        let pdf_bytes = fs::read(&first.path).unwrap();
        let current = record_path(&plan);
        let legacy = first.path.with_extension("manifest.json");
        let mut value = serde_json::to_value(&first.manifest).unwrap();
        value[invalid] = "not this output".into();
        let bytes = if invalid == "invalid-json" {
            b"not valid json".to_vec()
        } else {
            serde_json::to_vec_pretty(&value).unwrap()
        };
        fs::write(&legacy, &bytes).unwrap();
        fs::remove_file(&current).unwrap();
        assert!(build(&plan).is_err(), "accepted {invalid}");
        assert_eq!(fs::read(&first.path).unwrap(), pdf_bytes);
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        assert!(!current.exists());
    }
}

#[test]
fn conflicting_current_and_legacy_records_preserve_both_copies() {
    let fixture = Fixture::new();
    let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let first = build(&plan).unwrap();
    let pdf_bytes = fs::read(&first.path).unwrap();
    let current = record_path(&plan);
    let current_bytes = fs::read(&current).unwrap();
    let legacy = first.path.with_extension("manifest.json");
    let mut old = first.manifest.clone();
    old.created_at = "conflicting-history".into();
    let legacy_bytes = serde_json::to_vec_pretty(&old).unwrap();
    fs::write(&legacy, &legacy_bytes).unwrap();
    assert!(error(build(&plan)).contains("冲突"));
    assert_eq!(fs::read(&current).unwrap(), current_bytes);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
    assert_eq!(fs::read(&first.path).unwrap(), pdf_bytes);
}

#[test]
fn legacy_committed_pdf_journal_recovers_before_record_migration() {
    let fixture = Fixture::new();
    let plan = fixture.plan(vec![fixture.image(1, 40, 20)]);
    let first = build(&plan).unwrap();
    let bytes = fs::read(&first.path).unwrap();
    fs::remove_file(record_path(&plan)).unwrap();
    let name = first.path.file_name().unwrap().to_str().unwrap();
    let journal = first.path.parent().unwrap().join(format!(
        ".voucher-{}.transaction.json",
        hex::encode(Sha256::digest(name.as_bytes()))
    ));
    pdf::atomic_json(
        &journal,
        &serde_json::json!({
            "temporary": ".already-committed.partial.pdf",
            "output": name,
            "backup": null,
            "previous_hash": null,
            "manifest": first.manifest,
        }),
    )
    .unwrap();
    let previous_journal = journal.with_extension("json.previous");
    fs::copy(&journal, &previous_journal).unwrap();
    assert!(build(&plan).unwrap().skipped);
    assert_eq!(fs::read(&first.path).unwrap(), bytes);
    assert!(record_path(&plan).exists());
    assert!(!journal.exists());
    assert!(!previous_journal.exists());
    assert!(!first.path.with_extension("manifest.json").exists());
    assert!(build(&plan).unwrap().skipped);
    assert!(!journal.exists());
    assert!(!previous_journal.exists());
}

#[test]
fn same_named_outputs_in_different_directories_use_separate_records() {
    let state = tempfile::tempdir().unwrap();
    let left = Fixture::new();
    let right = Fixture::new();
    let left_plan = left.plan(vec![left.image(1, 40, 20)]);
    let mut right_plan = right.plan(vec![right.image(1, 20, 40)]);
    let shared_build = |plan: &Plan| {
        pdf::build_voucher(
            state.path(),
            plan,
            &plan.vouchers[0],
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
    };
    let left_output = shared_build(&left_plan);
    let right_output = shared_build(&right_plan);
    assert_eq!(left_output.path.file_name(), right_output.path.file_name());
    let left_record = pdf::manifest_path(state.path(), &left_output.path).unwrap();
    let right_record = pdf::manifest_path(state.path(), &right_output.path).unwrap();
    assert_ne!(left_record, right_record);
    let left_bytes = fs::read(&left_record).unwrap();
    right_plan.vouchers[0].files[0].rotation_cw = 180;
    assert!(!shared_build(&right_plan).skipped);
    assert!(shared_build(&left_plan).skipped);
    assert_eq!(fs::read(left_record).unwrap(), left_bytes);
    assert_eq!(
        pdf::sha256_file(&left_output.path).unwrap(),
        left_output.manifest.output_sha256
    );
}

#[test]
fn fingerprint_rejects_changed_contents_even_if_size_and_mtime_are_restored() {
    let fixture = Fixture::new();
    let file = fixture.image(1, 40, 20);
    let path = fixture.root.join(&file.relative_path);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x7f;
    fs::write(&path, &bytes).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), file.bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let plan = fixture.plan(vec![file]);
    assert!(error(build(&plan)).contains("源文件已变化"));
    assert!(!output_path(&plan).exists());
}

#[test]
fn manual_review_clears_only_orientation_errors_and_survives_reload() {
    let fixture = Fixture::new();
    let mut prediction_error = fixture.image(1, 40, 20);
    prediction_error.reviewed = false;
    prediction_error.error = Some("方向识别失败：临时运行库错误".into());
    let mut source_error = fixture.image(2, 40, 20);
    source_error.error = Some("源文件损坏".into());
    let ids = vec![prediction_error.id.clone(), source_error.id.clone()];
    let plan = fixture.plan(vec![prediction_error, source_error]);
    let state = fixture.state();
    assert!(state.model_error.is_some());
    state.insert_plan(plan.clone()).unwrap();
    state.edit_files(&plan.id, &ids, Some(90)).unwrap();
    drop(state);
    let state = fixture.state();
    let saved = state.plan(&plan.id).unwrap();
    assert!(saved.vouchers[0].files[0].error.is_none());
    assert_eq!(
        saved.vouchers[0].files[1].error.as_deref(),
        Some("源文件损坏")
    );
    assert!(saved.vouchers[0]
        .files
        .iter()
        .all(|f| f.reviewed && f.rotation_cw == 90));
    assert!(error(build(&saved)).contains("源文件存在错误"));
}

#[test]
fn missing_model_does_not_block_manual_generation_or_restart_retry() {
    let fixture = Fixture::new();
    let mut file = fixture.image(1, 40, 20);
    file.error = Some("方向识别失败：临时故障".into());
    file.reviewed = false;
    let id = file.id.clone();
    let plan = fixture.plan(vec![file]);
    let state = fixture.state();
    state.insert_plan(plan.clone()).unwrap();
    state.edit_files(&plan.id, &[id], Some(270)).unwrap();
    let job = state
        .start(&plan.id, "generate", GenerateRequest::default())
        .unwrap();
    let mut completed = wait_job(&state, &job.id);
    assert_eq!(completed.status, "completed", "{:?}", completed.error);
    assert_eq!(completed.results[0].status, "completed");
    // Simulate a process ending after output commit but before terminal status.
    completed.status = "running".into();
    pdf::atomic_json(
        &fixture
            .root
            .join("state/jobs")
            .join(format!("{}.json", job.id)),
        &completed,
    )
    .unwrap();
    drop(state);
    let recovered = fixture.state();
    assert_eq!(recovered.job(&job.id).unwrap().status, "interrupted");
    let retry = recovered.retry(&job.id).unwrap();
    let retried = wait_job(&recovered, &retry.id);
    assert_eq!(retried.status, "completed", "{:?}", retried.error);
    assert_eq!(retried.results[0].status, "skipped");
    assert_eq!(retried.done, retried.total);
}

#[test]
fn enabled_generation_never_bypasses_manual_review() {
    let fixture = Fixture::new();
    let mut file = fixture.image(1, 40, 20);
    file.reviewed = false;
    let mut plan = fixture.plan(vec![file]);
    plan.orientation_workflow = OrientationWorkflow {
        enabled: true,
        consent_version: Some(ORIENTATION_DISCLAIMER_VERSION.into()),
        consented_at: Some("2026-01-01T00:00:00Z".into()),
        analysis_completed: true,
    };
    let result = pdf::build_voucher(
        &state_dir(&plan),
        &plan,
        &plan.vouchers[0],
        true,
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(
        result.is_err(),
        "the legacy bypass must never permit unchecked pages"
    );
    assert!(build(&plan).is_err());
    plan.vouchers[0].files[0].orientation = Some(Orientation {
        rotation_cw: 0,
        confidence: 0.6,
        scores: vec![0.6, 0.2, 0.1, 0.1],
        model: voucher_workbench::orientation::MODEL_ID.into(),
        needs_review: true,
        title: None,
        reason: "合成测试：未能读清标题".into(),
        candidates: vec![],
    });
    assert!(build(&plan).is_err());
    let bypass = pdf::build_voucher(
        &state_dir(&plan),
        &plan,
        &plan.vouchers[0],
        true,
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(bypass.is_err());
    plan.vouchers[0].files[0].reviewed = true;
    assert!(!build(&plan).unwrap().manifest.needs_review);
}
