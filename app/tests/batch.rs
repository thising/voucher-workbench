//! Synthetic-only coverage of the unified batch workflow.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, Stream,
};
use serde_json::{json, Value};
use std::{
    fs::{self, FileTimes, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};
use tempfile::TempDir;
use tower::ServiceExt;
use voucher_workbench::{
    batch::{self, EditRequest, FileEdit, GroupEdit, ScanRequest, Sort},
    jobs::{AppState, GenerateRequest, Job},
    model::Orientation,
    orientation, pdf,
    rename_index::{self, Config},
};

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    state_dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("合成 数据");
        fs::create_dir(&root).unwrap();
        Self {
            _temp: temp,
            root,
            state_dir: base.join("处理 记录"),
        }
    }
    fn state(&self) -> Arc<AppState> {
        AppState::load(
            self.state_dir.clone(),
            &self.state_dir.join("missing-model"),
            None,
        )
        .unwrap()
    }
    fn request(&self, level: u16) -> ScanRequest {
        ScanRequest {
            root: self.root.to_str().unwrap().into(),
            level,
            exclude_rules: batch::DEFAULT_RULES.into(),
            index: None,
            sort: Sort::default(),
        }
    }
    fn scan(&self, level: u16) -> batch::Plan {
        batch::scan(self.request(level), None).unwrap()
    }
    fn pdf(&self, path: &str, pages: u32) {
        static_pdf(&self.root.join(path), pages, path);
    }
    fn image(&self, path: &str, w: u32, h: u32) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(w, h, image::Rgb([50, 100, 150]))
            .save(path)
            .unwrap();
    }
}
fn static_pdf(path: &Path, count: u32, label: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut doc = Document::with_version("1.7");
    let page_root = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let resources = doc.add_object(dictionary! {"Font"=>dictionary! {"F1"=>font}});
    let mut kids = vec![];
    for n in 1..=count {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Td", vec![20.into(), 30.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::string_literal(format!("{label}:PAGE{n}"))],
                ),
                Operation::new("ET", vec![]),
            ],
        };
        let stream = doc.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
        kids.push(Object::Reference(doc.add_object(dictionary! {"Type"=>"Page","Parent"=>page_root,"Contents"=>stream,"MediaBox"=>vec![0.into(),0.into(),(300+n).into(),600.into()],"Rotate"=>if n%2==0 {90} else {0}})));
    }
    doc.objects.insert(
        page_root,
        Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>count,"Resources"=>resources},
        ),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>page_root});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}
fn wait(state: &AppState, id: &str) -> Job {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut previous = 0;
    loop {
        let job = state.job(id).unwrap();
        assert!(job.done >= previous && job.done <= job.total);
        previous = job.done;
        if !job.active() {
            return job;
        }
        assert!(Instant::now() < deadline, "job stuck: {}", job.current);
        thread::sleep(Duration::from_millis(2));
    }
}
fn generated(state: &Arc<AppState>, plan: batch::Plan, threads: usize) -> Job {
    let ids = plan
        .groups
        .iter()
        .filter(|g| !g.empty && !g.issues.iter().any(|i| i.severity == "error"))
        .map(|g| g.id.clone())
        .collect();
    let plan = state.insert_batch_plan(plan).unwrap();
    let job = state
        .start_batch(
            &plan.id,
            "batch_generate",
            GenerateRequest {
                voucher_ids: Some(ids),
                allow_unreviewed: false,
            },
            threads,
        )
        .unwrap();
    wait(state, &job.id)
}
fn cached(plan: &mut batch::Plan, angle: u16) {
    for file in plan
        .groups
        .iter_mut()
        .flat_map(|g| &mut g.files)
        .filter(|f| f.kind == "image")
    {
        file.orientation = Some(Orientation {
            rotation_cw: angle,
            confidence: 0.99,
            scores: vec![],
            model: orientation::GENERAL_MODEL_ID.into(),
            needs_review: false,
            title: Some("Synthetic header".into()),
            reason: "Synthetic cache; no model inference".into(),
            candidates: vec![],
        });
    }
}
fn workbook(path: &Path) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let entries = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="first" sheetId="1" r:id="rId1"/><sheet name="selected" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>folder</t></is></c><c r="B1" t="inlineStr"><is><t>name</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>230501-04067</t></is></c><c r="B2" t="inlineStr"><is><t>首表名称</t></is></c></row></sheetData></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="3"><c r="C3" t="inlineStr"><is><t>文件夹索引</t></is></c><c r="D3" t="inlineStr"><is><t>重命名文件名</t></is></c></row><row r="4"><c r="C4" t="inlineStr"><is><t>(c)04067</t></is></c><c r="D4" t="inlineStr"><is><t>第一项</t></is></c></row><row r="5"><c r="C5" t="inlineStr"><is><t>(c)04067</t></is></c><c r="D5" t="inlineStr"><is><t>第二项</t></is></c></row><row r="6"><c r="C6" t="inlineStr"><is><t>(c)04068</t></is></c><c r="D6" t="inlineStr"><is><t>唯一名称</t></is></c></row><row r="7"><c r="C7" t="inlineStr"><is><t>Empty</t></is></c></row></sheetData></worksheet>"#,
        ),
    ];
    for (name, text) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}
fn config(path: &Path) -> Config {
    Config {
        path: path.to_str().unwrap().into(),
        sheet: "selected".into(),
        index_column: 2,
        name_column: 3,
        header_row: 3,
        match_rule: "auto".into(),
        naming: "index".into(),
        duplicates: "fallback".into(),
    }
}

#[test]
fn initialization_discovers_levels_and_mixed_shallow_sources_are_never_lost() {
    let f = Fixture::new();
    f.pdf("root.pdf", 2);
    f.image("A/a.png", 80, 40);
    f.pdf("A/X/2.pdf", 3);
    f.image("B/b.png", 30, 60);
    fs::create_dir_all(f.root.join("Empty/Deep/Final")).unwrap();
    let init = batch::initialize(f.root.to_str().unwrap(), None).unwrap();
    assert_eq!(init.max_level, 3);
    assert_eq!(init.file_count, 4);
    assert_eq!(
        init.levels.iter().map(|l| l.level).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert!(init.levels[2].samples.contains(&"A/X".into()));
    assert_eq!(
        f.scan(0).groups[0]
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["2.pdf", "a.png", "b.png", "root.pdf"]
    );
    for level in 0..=3 {
        let plan = f.scan(level);
        assert_eq!(plan.groups.len(), init.levels[level as usize].folder_count);
        assert_eq!(plan.groups.iter().map(|g| g.pages).sum::<u32>(), 7);
        let files: Vec<_> = plan.groups.iter().flat_map(|g| &g.files).collect();
        assert_eq!(files.len(), 4);
        assert_eq!(
            files
                .iter()
                .map(|f| &f.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
        if level > 0 {
            assert!(plan
                .groups
                .iter()
                .any(|g| g.relative_path.is_empty() && g.shallow));
        }
    }
    assert!(batch::initialize("", None).is_err());
}

#[test]
fn optional_index_selects_sheet_and_global_columns_and_conflicts_fall_back() {
    let f = Fixture::new();
    for folder in ["230501-04067", "230501-04068", "Unknown", "Empty"] {
        f.pdf(&format!("{folder}/1.pdf"), 1);
    }
    let path = f.root.parent().unwrap().join("rename.xlsx");
    workbook(&path);
    let first = rename_index::inspect(&path, None, None).unwrap();
    assert_eq!(first.sheet, "first");
    assert_eq!(first.header_row, 1);
    let view = rename_index::inspect(&path, Some("selected"), None).unwrap();
    assert_eq!(view.sheets.len(), 2);
    assert_eq!(view.header_row, 3);
    assert_eq!(view.suggested_index_column, Some(2));
    assert_eq!(view.suggested_name_column, Some(3));
    assert_eq!(view.preview_rows[0].row, 4);
    assert_eq!(view.preview_rows[0].values[2], "(c)04067");
    let mut req = f.request(1);
    req.index = Some(config(&path));
    let plan = batch::scan(req.clone(), None).unwrap();
    let conflict = plan
        .groups
        .iter()
        .find(|g| g.name == "230501-04067")
        .unwrap();
    assert_eq!(conflict.output_name, "230501-04067.pdf");
    assert_eq!(conflict.rename.status, "conflict");
    assert_eq!(conflict.rename.rows, vec![4, 5]);
    let matched = plan
        .groups
        .iter()
        .find(|g| g.name == "230501-04068")
        .unwrap();
    assert_eq!(matched.output_name, "唯一名称.pdf");
    assert_eq!(matched.rename.rule.as_deref(), Some("voucher_code"));
    assert_eq!(
        plan.groups
            .iter()
            .find(|g| g.name == "Unknown")
            .unwrap()
            .rename
            .status,
        "unmatched"
    );
    assert_eq!(
        plan.groups
            .iter()
            .find(|g| g.name == "Empty")
            .unwrap()
            .rename
            .status,
        "empty"
    );
    let c = req.index.as_mut().unwrap();
    c.duplicates = "first".into();
    c.naming = "folder_index".into();
    let first = batch::scan(req, None).unwrap();
    assert_eq!(
        first
            .groups
            .iter()
            .find(|g| g.name == "230501-04067")
            .unwrap()
            .output_name,
        "230501-04067-第一项.pdf"
    );
    let mut req = f.request(1);
    req.index = Some(Config {
        path: f.root.join("absent.xls").to_string_lossy().into(),
        ..config(&path)
    });
    let fallback = batch::scan(req, None).unwrap();
    assert!(fallback.index_error.is_some());
    assert!(fallback
        .groups
        .iter()
        .all(|g| g.output_name == format!("{}.pdf", g.name)));
}

#[test]
fn rule_errors_are_per_line_warnings_and_nested_output_exclusion_can_be_reversed() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    f.pdf("indexImg/hidden.pdf", 1);
    f.pdf("Nested/indexImg/hidden.pdf", 1);
    f.pdf("delivery/old.pdf", 2);
    fs::write(f.root.join("index.html"), "not used").unwrap();
    let mut req = f.request(0);
    req.exclude_rules
        .push_str("\n# comment\n../bad.pdf\n**x/*.pdf\n");
    let plan = batch::scan(req, None).unwrap();
    assert_eq!(plan.groups[0].files.len(), 2);
    assert_eq!(
        plan.issues
            .iter()
            .filter(|i| i.code == "invalid_exclusion")
            .count(),
        2
    );
    let changed = batch::edit(
        &plan,
        EditRequest {
            output_dir: Some(f.root.join("delivery").to_string_lossy().into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(changed.groups[0].files.len(), 1);
    assert_eq!(changed.omitted_sources.len(), 1);
    assert_eq!(changed.groups[0].pages, 1);
    let restored = batch::edit(
        &changed,
        EditRequest {
            output_dir: Some(plan.output_dir.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(restored.groups[0].files.len(), 2);
    assert!(restored.omitted_sources.is_empty());
    assert_eq!(restored.groups[0].pages, 3);
    assert_eq!(plan.groups[0].files.len(), 2);
}

#[test]
fn source_state_is_automatically_excluded_but_bad_files_still_block_their_group() {
    let f = Fixture::new();
    let state_dir = f.root.join("records");
    fs::create_dir(&state_dir).unwrap();
    static_pdf(&state_dir.join("secret.pdf"), 1, "not input");
    f.pdf("Good/1.pdf", 1);
    fs::create_dir(f.root.join("Bad")).unwrap();
    fs::write(f.root.join("Bad/1.pdf"), b"bad PDF").unwrap();
    let plan = batch::scan(f.request(1), Some(&state_dir)).unwrap();
    assert!(!plan.groups.iter().any(|g| g.name == "records"));
    assert!(
        plan.groups.iter().find(|g| g.name == "Bad").unwrap().files[0]
            .error
            .is_some()
    );
    let state = f.state();
    let plan = state.insert_batch_plan(plan).unwrap();
    assert!(state
        .start(&plan.id, "batch_generate", GenerateRequest::default())
        .is_err());
    let good = plan.groups.iter().find(|g| g.name == "Good").unwrap();
    let job = state
        .start(
            &plan.id,
            "batch_generate",
            GenerateRequest {
                voucher_ids: Some(vec![good.id.clone()]),
                allow_unreviewed: false,
            },
        )
        .unwrap();
    assert_eq!(job.threads, 4);
    assert_eq!(wait(&state, &job.id).status, "completed");
    assert!(batch::initialize(state_dir.to_str().unwrap(), Some(&state_dir)).is_err());
}

#[test]
fn all_sort_modes_and_manual_order_produce_stable_immutable_revisions() {
    let f = Fixture::new();
    for (i, name) in ["2_b_10.pdf", "10_a_2.pdf", "1_c_1.pdf"].iter().enumerate() {
        f.pdf(name, 1);
        OpenOptions::new()
            .write(true)
            .open(f.root.join(name))
            .unwrap()
            .set_times(
                FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000 + (2 - i) as u64)),
            )
            .unwrap();
    }
    let original = f.scan(0);
    let id = original.groups[0].id.clone();
    let names = |p: &batch::Plan| {
        p.groups[0]
            .files
            .iter()
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&original),
        vec!["1_c_1.pdf", "2_b_10.pdf", "10_a_2.pdf"]
    );
    for (mode, descending, expected) in [
        ("name", true, vec!["10_a_2.pdf", "2_b_10.pdf", "1_c_1.pdf"]),
        (
            "prefix",
            false,
            vec!["1_c_1.pdf", "2_b_10.pdf", "10_a_2.pdf"],
        ),
        (
            "suffix",
            false,
            vec!["1_c_1.pdf", "10_a_2.pdf", "2_b_10.pdf"],
        ),
        (
            "modified",
            true,
            vec!["2_b_10.pdf", "10_a_2.pdf", "1_c_1.pdf"],
        ),
    ] {
        let plan = batch::edit(
            &original,
            EditRequest {
                groups: vec![GroupEdit {
                    group_id: id.clone(),
                    sort: Some(Sort {
                        mode: mode.into(),
                        descending,
                        separator: None,
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(names(&plan), expected);
        assert_ne!(plan.id, original.id);
        assert_eq!(plan.parent_plan_id.as_ref(), Some(&original.id));
        assert_eq!(plan.groups[0].id, id);
    }
    let mut ids = original.groups[0]
        .files
        .iter()
        .map(|f| f.id.clone())
        .collect::<Vec<_>>();
    ids.rotate_left(1);
    let manual = batch::edit(
        &original,
        EditRequest {
            groups: vec![GroupEdit {
                group_id: id.clone(),
                file_ids: Some(ids.clone()),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        manual.groups[0]
            .files
            .iter()
            .map(|f| f.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(manual.groups[0].sort.mode, "manual");
    assert_eq!(original.groups[0].sort.mode, "name");
    ids[0] = ids[1].clone();
    assert!(batch::edit(
        &original,
        EditRequest {
            groups: vec![GroupEdit {
                group_id: id,
                file_ids: Some(ids),
                ..Default::default()
            }],
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn mixed_pdf_page_angles_preserve_original_rotate_and_review_is_not_implicit() {
    let f = Fixture::new();
    f.image("1.png", 80, 40);
    f.pdf("2.pdf", 2);
    let original = f.scan(0);
    let image = &original.groups[0].files[0];
    let pdf = &original.groups[0].files[1];
    let rotated = batch::edit(
        &original,
        EditRequest {
            files: vec![
                FileEdit {
                    file_id: image.id.clone(),
                    rotation_cw: Some(90),
                    ..Default::default()
                },
                FileEdit {
                    file_id: pdf.id.clone(),
                    rotation_cw: Some(90),
                    page_rotations: Some(vec![180, 270]),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rotated.groups[0].review_count, 2);
    assert!(rotated.groups[0].files.iter().all(|f| !f.reviewed));
    let state = f.state();
    let job = generated(&state, rotated.clone(), 3);
    assert_eq!(job.status, "completed", "{:?}", job.error);
    let bytes = batch::read_output(&state, &job, &rotated.groups[0].id).unwrap();
    let doc = Document::load_mem(&bytes).unwrap();
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 3);
    let first = doc.get_dictionary(pages[&1]).unwrap();
    let bounds = first.get(b"MediaBox").unwrap().as_array().unwrap();
    let number = |v: &Object| v.as_float().unwrap();
    assert!(number(&bounds[3]) > number(&bounds[2]));
    assert_eq!(
        doc.get_dictionary(pages[&2])
            .unwrap()
            .get(b"Rotate")
            .unwrap()
            .as_i64()
            .unwrap(),
        270
    );
    assert_eq!(
        doc.get_dictionary(pages[&3])
            .unwrap()
            .get(b"Rotate")
            .unwrap()
            .as_i64()
            .unwrap(),
        90
    );
    let report = batch::report(&state, &job).unwrap();
    assert_eq!(report["unreviewed_files"], 2);
    assert_eq!(report["unreviewed_pages"], 3);
    assert_eq!(
        report["manifests"][0]["sources"][1]["page_rotations"],
        json!([180, 270])
    );
    assert_eq!(report["manifests"][0]["unreviewed_pages"], 3);
    let confirmed = batch::edit(
        &rotated,
        EditRequest {
            files: vec![FileEdit {
                file_id: pdf.id.clone(),
                reviewed_pages: Some(vec![true, false]),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        confirmed.groups[0].files[1].reviewed_pages,
        vec![true, false]
    );
    assert!(!confirmed.groups[0].files[1].reviewed);
    assert!(batch::edit(
        &rotated,
        EditRequest {
            files: vec![FileEdit {
                file_id: pdf.id.clone(),
                page_rotations: Some(vec![90]),
                ..Default::default()
            }],
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn cached_optional_analysis_is_nonblocking_and_preserves_manual_direction() {
    let f = Fixture::new();
    f.image("1.png", 50, 30);
    f.image("2.png", 50, 30);
    f.pdf("3.pdf", 2);
    let mut plan = f.scan(0);
    cached(&mut plan, 90);
    let second = plan.groups[0].files[1].id.clone();
    let manual = batch::edit(
        &plan,
        EditRequest {
            files: vec![FileEdit {
                file_id: second.clone(),
                rotation_cw: Some(180),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let state = f.state();
    assert!(state.model_error.is_some());
    let original = state.insert_batch_plan(manual.clone()).unwrap();
    assert!(state
        .start(&original.id, "batch_analyze", GenerateRequest::default())
        .is_err());
    let derived = state
        .insert_batch_plan(batch::analysis_revision(&manual, 10).unwrap())
        .unwrap();
    let job = state
        .start_batch(&derived.id, "batch_analyze", GenerateRequest::default(), 10)
        .unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "completed", "{:?}", done.error);
    assert_eq!(done.total, 2);
    assert_eq!(done.done, 2);
    assert_eq!(done.threads, 10);
    let analyzed = state.batch_plan(&done.plan_id).unwrap();
    assert!(analyzed.orientation_workflow.analysis_completed);
    assert_eq!(
        analyzed.orientation_workflow.consent_version.as_deref(),
        Some(batch::DISCLAIMER_VERSION)
    );
    assert_eq!(analyzed.groups[0].files[0].rotation_cw, 90);
    assert_eq!(analyzed.groups[0].files[1].rotation_cw, 180);
    assert_eq!(analyzed.groups[0].review_count, 2);
    assert_eq!(
        state.batch_plan(&original.id).unwrap().groups[0].files[0].rotation_cw,
        0
    );
    let report = batch::report(&state, &done).unwrap();
    assert!(report["manifests"].as_array().unwrap().is_empty());
    let merge = state
        .start(&analyzed.id, "batch_generate", GenerateRequest::default())
        .unwrap();
    assert_eq!(merge.threads, 4);
    assert_eq!(wait(&state, &merge.id).status, "completed");
    let retry = state.retry(&done.id).unwrap();
    assert_eq!(retry.threads, 10);
    assert_ne!(retry.plan_id, done.plan_id);
    assert_eq!(wait(&state, &retry.id).status, "completed");
    let reloaded = f.state();
    assert_eq!(
        reloaded.batch_plan(&done.plan_id).unwrap().groups[0].files[1].rotation_cw,
        180
    );
}

#[test]
fn source_hash_change_with_same_size_and_time_never_silently_drops_a_page() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    let plan = f.scan(0);
    let path = f.root.join("1.pdf");
    let before = fs::metadata(&path).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"%PDF-1.7");
    bytes[7] = b'6';
    fs::write(&path, bytes).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    let state = f.state();
    let plan = state.insert_batch_plan(plan).unwrap();
    let job = state
        .start(&plan.id, "batch_generate", GenerateRequest::default())
        .unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "completed_with_errors");
    assert_eq!(done.results.len(), 1);
    assert!(done.results[0]
        .error
        .as_deref()
        .unwrap()
        .contains("来源文件"));
    assert!(!Path::new(&plan.groups[0].output_path).exists());
}

#[test]
fn changed_index_requires_rescan_even_if_output_filename_still_looks_correct() {
    let f = Fixture::new();
    f.pdf("230501-04068/1.pdf", 1);
    let path = f.root.parent().unwrap().join("rename.xlsx");
    workbook(&path);
    let mut req = f.request(1);
    req.index = Some(config(&path));
    let plan = batch::scan(req, None).unwrap();
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(b"changed").unwrap();
    let error = batch::build_group(
        &f.state_dir,
        &plan,
        &plan.groups[0],
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("索引已变化"));
    assert!(!Path::new(&plan.groups[0].output_path).exists());
}

#[test]
fn own_output_is_reused_and_revisions_archive_old_bytes_but_foreign_output_is_rejected() {
    let f = Fixture::new();
    f.pdf("1.pdf", 2);
    let state = f.state();
    let plan = f.scan(0);
    let first = generated(&state, plan.clone(), 1);
    assert_eq!(first.status, "completed");
    let first_hash =
        pdf::sha256_file(Path::new(first.results[0].output_path.as_ref().unwrap())).unwrap();
    let first_report = batch::report(&state, &first).unwrap();
    let same = generated(
        &state,
        batch::edit(&plan, EditRequest::default()).unwrap(),
        2,
    );
    assert_eq!(same.results[0].status, "skipped");
    let changed = batch::edit(
        &plan,
        EditRequest {
            files: vec![FileEdit {
                file_id: plan.groups[0].files[0].id.clone(),
                page_rotations: Some(vec![90, 0]),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let third = generated(&state, changed.clone(), 4);
    assert_eq!(third.status, "completed");
    assert_ne!(
        first_hash,
        pdf::sha256_file(Path::new(third.results[0].output_path.as_ref().unwrap())).unwrap()
    );
    assert!(batch::read_output(&state, &first, &plan.groups[0].id).is_err());
    assert_eq!(batch::report(&state, &first).unwrap(), first_report);
    let output = Path::new(&plan.output_dir);
    assert!(output.join(".history").exists());
    let manifest =
        pdf::manifest_path(&f.state_dir, Path::new(&changed.groups[0].output_path)).unwrap();
    let mut foreign: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    foreign["mode"] = "pdf_merge".into();
    pdf::atomic_json(&manifest, &foreign).unwrap();
    let mut inspected = changed;
    batch::inspect_existing_outputs(&f.state_dir, &mut inspected).unwrap();
    assert!(inspected.groups[0]
        .issues
        .iter()
        .any(|i| i.code == "output_existing"));
    assert!(batch::build_group(
        &f.state_dir,
        &plan,
        &plan.groups[0],
        &AtomicBool::new(false),
        &mut |_| {}
    )
    .is_err());
}

#[test]
fn cancellation_cleans_uncommitted_pdf_and_shutdown_prevents_new_tasks_and_edits() {
    let f = Fixture::new();
    f.pdf("1.pdf", 2);
    f.pdf("2.pdf", 2);
    let plan = f.scan(0);
    let cancel = AtomicBool::new(false);
    let result = batch::build_group(&f.state_dir, &plan, &plan.groups[0], &cancel, &mut |_| {
        cancel.store(true, Ordering::Release)
    });
    assert!(result.is_err());
    assert!(!Path::new(&plan.groups[0].output_path).exists());
    assert!(!fs::read_dir(&plan.output_dir).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("partial")));
    let state = f.state();
    let plan = state.insert_batch_plan(plan).unwrap();
    state.shutting_down.store(true, Ordering::Release);
    assert!(state
        .start(&plan.id, "batch_generate", GenerateRequest::default())
        .unwrap_err()
        .to_string()
        .contains("退出"));
    assert!(state
        .insert_batch_plan(batch::edit(&plan, EditRequest::default()).unwrap())
        .is_err());
}

async fn call(app: &axum::Router, uri: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn api_defaults_to_four_threads_and_requires_consent_only_for_analysis() {
    let f = Fixture::new();
    f.image("1.png", 30, 50);
    let state = f.state();
    assert_eq!(state.settings().threads, 1);
    let app = batch::router().with_state(state.clone());
    let (status, init) = call(&app, "/initialize", json!({"root":f.root})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(init["max_level"], 0);
    let (status, scan) = call(&app, "/scan", json!({"root":f.root})).await;
    assert_eq!(status, StatusCode::OK);
    let id = scan["id"].as_str().unwrap();
    let (status, error) = call(
        &app,
        &format!("/plans/{id}/analyze"),
        json!({"accept_disclaimer":false}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("免责声明"));
    let (status, error) = call(
        &app,
        &format!("/plans/{id}/analyze"),
        json!({"accept_disclaimer":true}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("模型未就绪"));
    for threads in [0, 11] {
        assert_eq!(
            call(
                &app,
                &format!("/plans/{id}/generate"),
                json!({"threads":threads})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (status, job) = call(&app, &format!("/plans/{id}/generate"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(job["threads"], 4);
    let done = wait(&state, job["id"].as_str().unwrap());
    assert_eq!(done.status, "completed");
    assert_eq!(
        batch::report(&state, &done).unwrap()["orientation_workflow"]["consent_version"],
        Value::Null
    );
    let file_id = scan["groups"][0]["files"][0]["id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/plans/{id}/files/{file_id}?thumbnail=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let image = image::load_from_memory(&bytes).unwrap();
    assert_eq!((image.width(), image.height()), (30, 50));
}

#[test]
fn generic_heading_selector_uses_text_geometry_and_keeps_ambiguity_unrotated() {
    let candidate =
        |angle, text: &str, height, confidence| voucher_workbench::model::TitleCandidate {
            rotation_cw: angle,
            text: text.into(),
            bbox: [20, 10, 200, height],
            confidence,
            keyword: "通用头部文字".into(),
            accepted: true,
        };
    let selected = orientation::select_generic_direction(vec![
        candidate(90, "研究报告", 30, 0.95),
        candidate(0, "abc", 15, 0.95),
    ]);
    assert_eq!(selected.rotation_cw, 90);
    assert_eq!(selected.model, orientation::GENERAL_MODEL_ID);
    assert_eq!(selected.title.as_deref(), Some("研究报告"));
    assert!(selected.reason.contains("不能保证"));
    let ambiguous = orientation::select_generic_direction(vec![
        candidate(90, "研究报告", 30, 0.95),
        candidate(270, "project report", 29, 0.95),
    ]);
    assert_eq!(ambiguous.rotation_cw, 0);
    assert!(ambiguous.needs_review);
    let noise = orientation::select_generic_direction(vec![
        candidate(180, "123456", 80, 0.99),
        candidate(90, "文档标题", 20, 0.4),
    ]);
    assert_eq!(noise.rotation_cw, 0);
    assert!(noise.needs_review);
    assert!(noise.candidates.iter().all(|c| !c.accepted));
}

#[test]
fn restart_marks_inflight_analysis_incomplete_and_retry_keeps_threads_and_group_selection() {
    let f = Fixture::new();
    f.image("A/1.png", 30, 50);
    f.image("B/1.png", 30, 50);
    let state = f.state();
    let mut plan = f.scan(1);
    cached(&mut plan, 90);
    let mut plan = batch::analysis_revision(&plan, 7).unwrap();
    plan.orientation_workflow.analysis_completed = true;
    let plan = state.insert_batch_plan(plan).unwrap();
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: plan.id.clone(),
        kind: "batch_analyze".into(),
        status: "running".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        total: 1,
        done: 0,
        current: "Synthetic interrupted analysis".into(),
        error: None,
        results: vec![],
        request: GenerateRequest {
            voucher_ids: Some(vec![plan.groups[0].id.clone()]),
            allow_unreviewed: false,
        },
        threads: 7,
    };
    state
        .data
        .lock()
        .unwrap()
        .jobs
        .insert(job.id.clone(), job.clone());
    pdf::atomic_json(
        &state.dir.join("jobs").join(format!("{}.json", job.id)),
        &job,
    )
    .unwrap();
    assert!(state
        .start(&plan.id, "batch_generate", GenerateRequest::default())
        .unwrap_err()
        .to_string()
        .contains("已有任务"));
    assert!(state
        .insert_batch_plan(batch::edit(&plan, EditRequest::default()).unwrap())
        .is_err());
    let loaded = f.state();
    assert_eq!(loaded.job(&job.id).unwrap().status, "interrupted");
    assert!(
        !loaded
            .batch_plan(&plan.id)
            .unwrap()
            .orientation_workflow
            .analysis_completed
    );
    let retry = loaded.retry(&job.id).unwrap();
    assert_eq!(retry.threads, 7);
    assert_eq!(retry.request.voucher_ids, job.request.voucher_ids);
    assert_eq!(retry.total, 1);
    assert_eq!(wait(&loaded, &retry.id).status, "completed");
    let final_plan = loaded.batch_plan(&retry.plan_id).unwrap();
    assert_eq!(final_plan.groups[0].files[0].rotation_cw, 90);
    assert_eq!(final_plan.groups[1].files[0].rotation_cw, 0);
}

#[test]
fn output_collisions_require_user_names_and_unknown_existing_pdf_is_preserved() {
    let f = Fixture::new();
    f.pdf("A/Same/1.pdf", 1);
    f.pdf("B/Same/1.pdf", 1);
    let plan = f.scan(2);
    assert!(plan
        .groups
        .iter()
        .all(|g| g.issues.iter().any(|i| i.code == "output_collision")));
    let renamed = batch::edit(
        &plan,
        EditRequest {
            groups: vec![GroupEdit {
                group_id: plan.groups[0].id.clone(),
                output_name: Some("A-Same".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(renamed
        .groups
        .iter()
        .all(|g| !g.issues.iter().any(|i| i.code == "output_collision")));
    fs::create_dir_all(&renamed.output_dir).unwrap();
    let target = Path::new(&renamed.groups[0].output_path);
    static_pdf(target, 3, "foreign");
    let before = fs::read(target).unwrap();
    let mut inspected = renamed.clone();
    batch::inspect_existing_outputs(&f.state_dir, &mut inspected).unwrap();
    assert!(inspected.groups[0]
        .issues
        .iter()
        .any(|i| i.code == "output_existing"));
    assert!(batch::build_group(
        &f.state_dir,
        &renamed,
        &renamed.groups[0],
        &AtomicBool::new(false),
        &mut |_| {}
    )
    .is_err());
    assert_eq!(fs::read(target).unwrap(), before);
    for path in ["\\\\server\\share", "\\\\?\\UNC\\server\\share"] {
        let mut req = f.request(0);
        req.root = path.into();
        assert!(batch::scan(req, None)
            .unwrap_err()
            .to_string()
            .contains("本地"));
    }
}

#[cfg(windows)]
#[test]
fn windows_verbatim_and_normal_drive_paths_preserve_batch_source_ownership() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    let verbatim = f.request(0);
    assert!(verbatim.root.starts_with(r"\\?\"));
    let plan = batch::scan(verbatim.clone(), None).unwrap();
    assert_eq!(plan.groups[0].pages, 1);
    let mut normal = verbatim;
    normal.root = normal.root.strip_prefix(r"\\?\").unwrap().into();
    let plan2 = batch::scan(normal, None).unwrap();
    assert_eq!(plan.root, plan2.root);
    assert_eq!(
        plan.groups[0].files[0].sha256,
        plan2.groups[0].files[0].sha256
    );
    let state = f.state();
    assert_eq!(generated(&state, plan, 2).status, "completed");
}

#[test]
fn interrupted_owned_transaction_recovers_but_a_foreign_transaction_cannot_write_output() {
    for foreign in [false, true] {
        let f = Fixture::new();
        f.pdf("1.pdf", 2);
        let plan = f.scan(0);
        let group = &plan.groups[0];
        let built = batch::build_group(
            &f.state_dir,
            &plan,
            group,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let record = pdf::manifest_path(&f.state_dir, &built.path).unwrap();
        let mut manifest = serde_json::to_value(&built.manifest).unwrap();
        if foreign {
            manifest["mode"] = "pdf_merge".into();
        }
        let bytes = fs::read(&built.path).unwrap();
        let pending = Path::new(&plan.output_dir).join(".pending.pdf");
        fs::rename(&built.path, &pending).unwrap();
        fs::remove_file(&record).unwrap();
        let journal = record.with_file_name("transaction.json");
        pdf::atomic_json(&journal,&json!({"temporary":".pending.pdf","output":group.output_name,"backup":null,"previous_hash":null,"manifest":manifest})).unwrap();
        let mut inspected = plan.clone();
        batch::inspect_existing_outputs(&f.state_dir, &mut inspected).unwrap();
        assert_eq!(
            inspected.groups[0]
                .issues
                .iter()
                .any(|i| i.code == "output_existing"),
            foreign
        );
        let retry = batch::build_group(
            &f.state_dir,
            &plan,
            group,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        if foreign {
            assert!(retry.is_err());
            assert!(!built.path.exists());
            assert_eq!(fs::read(&pending).unwrap(), bytes);
            assert!(journal.exists());
        } else {
            assert!(retry.unwrap().skipped);
            assert_eq!(fs::read(&built.path).unwrap(), bytes);
            assert!(!pending.exists());
            assert!(!journal.exists());
        }
    }
}

#[test]
fn empty_first_index_sheet_still_exposes_other_selectable_sheets() {
    use std::io::{Cursor, Read};
    let f = Fixture::new();
    let path = f.root.parent().unwrap().join("rename.xlsx");
    workbook(&path);
    let mut archive = zip::ZipArchive::new(Cursor::new(fs::read(&path).unwrap())).unwrap();
    let mut entries = vec![];
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let mut contents = vec![];
        file.read_to_end(&mut contents).unwrap();
        if file.name() == "xl/worksheets/sheet1.xml" {
            contents=br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec();
        }
        entries.push((file.name().to_owned(), contents));
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, contents) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&contents).unwrap();
    }
    zip.finish().unwrap();
    let first = rename_index::inspect(&path, None, None).unwrap();
    assert_eq!(first.sheet, "first");
    assert_eq!(first.sheets.len(), 2);
    assert!(first.sheet_error.as_deref().unwrap().contains("为空"));
    assert!(first.columns.is_empty());
    let selected = rename_index::inspect(&path, Some("selected"), None).unwrap();
    assert!(selected.sheet_error.is_none());
    assert_eq!(selected.suggested_index_column, Some(2));
}

#[test]
fn an_empty_namesake_directory_does_not_block_the_only_nonempty_output() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("A/Same")).unwrap();
    f.pdf("B/Same/1.pdf", 1);
    let plan = f.scan(2);
    assert_eq!(plan.groups.len(), 2);
    assert!(plan
        .groups
        .iter()
        .all(|g| !g.issues.iter().any(|i| i.code == "output_collision")));
    let state = f.state();
    let job = generated(&state, plan, 4);
    assert_eq!(job.status, "completed");
    assert_eq!(job.results.len(), 1);
    assert_eq!(job.results[0].pages, 1);
}
