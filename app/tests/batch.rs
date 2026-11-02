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
            output_names: vec![],
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let current = state.job(&job.id).unwrap();
        if !current.active() {
            return current;
        }
        if let Some(conflict) = current.output_conflict {
            batch::resolve_output_conflict(state, &job.id, &conflict.id, "overwrite", false)
                .unwrap();
        }
        assert!(Instant::now() < deadline, "synthetic generation stuck");
        thread::sleep(Duration::from_millis(2));
    }
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
fn optional_index_selects_sheet_and_global_columns_and_uses_first_matching_row() {
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
    assert_eq!(conflict.output_name, "第一项.pdf");
    assert_eq!(conflict.rename.status, "matched");
    assert_eq!(conflict.rename.rows, vec![4, 5]);
    assert_eq!(
        conflict.rename.message,
        "匹配多行，已使用表内第一条（第 4 行）"
    );
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
    assert_eq!(rotated.groups[0].review_count, 0);
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
    assert_eq!(report["unreviewed_files"], 0);
    assert_eq!(report["unreviewed_pages"], 0);
    assert_eq!(
        report["manifests"][0]["sources"][1]["page_rotations"],
        json!([180, 270])
    );
    assert_eq!(report["manifests"][0]["unreviewed_pages"], 0);
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
    assert_eq!(analyzed.groups[0].review_count, 1);
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
fn owned_batch_revisions_replace_without_backups_and_noninteractive_foreign_output_is_rejected() {
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
    assert_eq!(same.results[0].status, "completed");
    assert!(
        batch::build_group(
            &f.state_dir,
            &plan,
            &plan.groups[0],
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .unwrap()
        .skipped
    );
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
    assert!(!output.join(".history").exists());
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
        .all(|i| i.code != "output_existing"));
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
    assert!(batch::start_scan(&state, f.request(0)).is_err());
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
        output_conflict: None,
        orientation_summary: None,
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
        .all(|i| i.code != "output_existing"));
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
        assert!(inspected.groups[0]
            .issues
            .iter()
            .all(|i| i.code != "output_existing"));
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

#[test]
fn metadata_preview_applies_exclusions_without_decoding_or_hashing_bad_sources() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("A")).unwrap();
    fs::create_dir_all(f.root.join("B/Deep")).unwrap();
    fs::write(f.root.join("A/bad.png"), b"broken image").unwrap();
    fs::write(f.root.join("B/Deep/bad.pdf"), b"broken PDF").unwrap();
    let mut request = f.request(1);
    request.exclude_rules.push_str("\nB/**\n");
    let prepared = batch::preview(request.clone(), None).unwrap();
    assert_eq!(prepared.summary.candidate_count, 1);
    assert_eq!(prepared.summary.nonempty_groups, 1);
    assert_eq!(prepared.max_level, 1);
    assert_eq!(prepared.groups[0].relative_path, "A");
    assert_eq!(prepared.groups[0].candidate_count, 1);
    assert_eq!(prepared.groups[0].image_count, 1);
    assert!(prepared.groups[0]
        .issues
        .iter()
        .all(|i| i.code != "source_error"));
    assert!(prepared.index.is_none());
    let scanned = batch::scan(request, None).unwrap();
    assert!(scanned.groups[0].files[0].error.is_some());
    let metrics = scanned.scan_metrics.unwrap();
    assert_eq!(metrics.counts.hash_calls, 1);
    assert_eq!(metrics.counts.source_errors, 1);
    assert_eq!(metrics.counts.bytes_read, 12);
}

#[test]
fn naming_preview_defaults_to_folder_prefix_and_keeps_manual_names_when_fully_scanned() {
    let f = Fixture::new();
    f.pdf("230501-04067/1.pdf", 1);
    f.pdf("230501-04068/1.pdf", 1);
    let path = f.root.parent().unwrap().join("rename.xlsx");
    workbook(&path);
    let config: Config = serde_json::from_value(
        json!({"path":path,"sheet":"selected","index_column":2,"name_column":3,"header_row":3}),
    )
    .unwrap();
    assert_eq!(config.naming, "folder_index");
    assert_eq!(config.duplicates, "first");
    let mut req = f.request(1);
    req.index = Some(config);
    let prepared = batch::preview(req.clone(), None).unwrap();
    assert_eq!(prepared.groups[0].output_name, "230501-04067-第一项.pdf");
    assert_eq!(prepared.groups[0].rename.rows, vec![4, 5]);
    req.output_names = vec![batch::OutputName {
        relative_path: "230501-04067".into(),
        output_name: "人工覆盖.pdf".into(),
    }];
    let revised = batch::preview(req.clone(), None).unwrap();
    assert_eq!(revised.groups[0].output_name, "人工覆盖.pdf");
    let scanned = batch::scan(req, None).unwrap();
    assert_eq!(scanned.groups[0].output_name, "人工覆盖.pdf");
    assert_eq!(scanned.index.unwrap().config.duplicates, "first");
}

#[test]
fn removing_bad_sources_is_reversible_and_recomputes_pages_errors_and_name_conflicts() {
    let f = Fixture::new();
    f.pdf("A/Same/1.pdf", 2);
    f.pdf("B/Same/1.pdf", 3);
    fs::write(f.root.join("A/Same/2.png"), b"bad image").unwrap();
    let original = f.scan(2);
    let bad = original.groups[0]
        .files
        .iter()
        .find(|f| f.error.is_some())
        .unwrap()
        .id
        .clone();
    let other = original.groups[1].files[0].id.clone();
    let reduced = batch::edit(
        &original,
        EditRequest {
            remove_file_ids: vec![bad.clone(), other.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(reduced.removed_sources.len(), 2);
    assert_eq!(reduced.groups[0].pages, 2);
    assert!(reduced.groups[0]
        .issues
        .iter()
        .all(|i| i.severity != "error"));
    assert!(reduced.groups[1].empty);
    assert_eq!(original.groups[0].files.len(), 2);
    assert!(f.root.join("A/Same/2.png").exists());
    assert!(f.root.join("B/Same/1.pdf").exists());
    let state = f.state();
    let job = generated(&state, reduced.clone(), 4);
    assert_eq!(job.status, "completed");
    let report = batch::report(&state, &job).unwrap();
    assert_eq!(report["removed_sources"].as_array().unwrap().len(), 2);
    assert_eq!(report["successful_pages"], 2);
    let restored = batch::edit(
        &reduced,
        EditRequest {
            restore_file_ids: vec![bad, other],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(restored.removed_sources.is_empty());
    assert_eq!(restored.groups[0].pages, 2);
    assert_eq!(restored.groups[1].pages, 3);
    assert!(restored.groups[0]
        .issues
        .iter()
        .any(|i| i.code == "source_error"));
    assert!(restored
        .groups
        .iter()
        .all(|g| g.issues.iter().any(|i| i.code == "output_collision")));
}

#[tokio::test]
async fn asynchronous_scan_reports_progress_preserves_full_fingerprints_and_has_light_history() {
    let f = Fixture::new();
    f.image("1.png", 30, 50);
    f.pdf("2.pdf", 2);
    fs::write(f.root.join("3.pdf"), b"bad PDF").unwrap();
    let state = f.state();
    let app = batch::router().with_state(state.clone());
    let (status, started) = call(&app, "/scans", json!({"root":f.root})).await;
    assert_eq!(status, StatusCode::OK);
    let id = started["id"].as_str().unwrap();
    let done = wait(&state, id);
    assert_eq!(done.kind, "batch_scan");
    assert_eq!(done.status, "completed");
    assert_eq!((done.done, done.total), (3, 3));
    let progress = batch::scan_status(&state, id).unwrap();
    assert_eq!(progress.plan_id.as_deref(), Some(done.plan_id.as_str()));
    assert_eq!(progress.counts.hash_calls, 3);
    assert_eq!(progress.counts.source_errors, 1);
    assert!(progress.logs.iter().any(|l| l.code == "source_error"));
    let plan = state.batch_plan(&done.plan_id).unwrap();
    assert!(plan.groups[0]
        .files
        .iter()
        .all(|file| !file.sha256.is_empty()));
    assert!(plan.groups[0].files.iter().any(|file| file.error.is_some()));
    let report = batch::report(&state, &done).unwrap();
    assert_eq!(report["counts"]["source_errors"], 1);
    let response = app
        .clone()
        .oneshot(
            Request::get("/plans?summary=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let summaries: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(summaries[0]["file_count"], 3);
    assert_eq!(summaries[0]["pages"], 3);
    assert!(summaries[0].get("groups").is_none());
    assert!(bytes.len() < 3000);
    let retry = state.retry(id).unwrap();
    assert_ne!(retry.plan_id, done.plan_id);
    assert_eq!(wait(&state, &retry.id).status, "completed");
    let reloaded = f.state();
    assert_eq!(
        batch::scan_status(&reloaded, id)
            .unwrap()
            .plan_id
            .as_deref(),
        Some(done.plan_id.as_str())
    );
}

#[test]
fn preview_name_collisions_clear_immediately_after_a_single_row_override() {
    let f = Fixture::new();
    f.pdf("230501-04067/1.pdf", 1);
    f.pdf("240501-04067/1.pdf", 1);
    let path = f.root.parent().unwrap().join("rename.xlsx");
    workbook(&path);
    let mut req = f.request(1);
    req.index = Some(config(&path));
    let conflicted = batch::preview(req.clone(), None).unwrap();
    assert!(conflicted
        .groups
        .iter()
        .all(|g| g.issues.iter().any(|i| i.code == "output_collision")));
    req.output_names = vec![batch::OutputName {
        relative_path: "240501-04067".into(),
        output_name: "人工独立名称".into(),
    }];
    let fixed = batch::preview(req.clone(), None).unwrap();
    assert!(fixed
        .groups
        .iter()
        .all(|g| g.issues.iter().all(|i| i.code != "output_collision")));
    assert_eq!(fixed.groups[1].output_name, "人工独立名称.pdf");
    let scan = batch::scan(req, None).unwrap();
    assert_eq!(scan.groups[1].output_name, "人工独立名称.pdf");
}

#[test]
fn restored_manual_removals_still_respect_the_selected_output_subtree() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    f.pdf("delivery/source.pdf", 2);
    let original = f.scan(0);
    let id = original.groups[0]
        .files
        .iter()
        .find(|f| f.relative_path.contains("delivery/"))
        .unwrap()
        .id
        .clone();
    let removed = batch::edit(
        &original,
        EditRequest {
            remove_file_ids: vec![id.clone()],
            output_dir: Some(f.root.join("delivery").to_string_lossy().into()),
            ..Default::default()
        },
    )
    .unwrap();
    let restored = batch::edit(
        &removed,
        EditRequest {
            restore_file_ids: vec![id],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(restored.removed_sources.is_empty());
    assert_eq!(restored.omitted_sources.len(), 1);
    assert_eq!(restored.groups[0].pages, 1);
    let moved = batch::edit(
        &restored,
        EditRequest {
            output_dir: Some(original.output_dir),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(moved.omitted_sources.is_empty());
    assert_eq!(moved.groups[0].pages, 3);
}

#[test]
fn manual_restore_order_survives_separate_removals_and_reverse_partial_restores() {
    let f = Fixture::new();
    for name in ["A.pdf", "B.pdf", "C.pdf", "D.pdf"] {
        f.pdf(name, 1);
    }
    let scanned = f.scan(0);
    let ids: Vec<_> = scanned.groups[0]
        .files
        .iter()
        .map(|f| f.id.clone())
        .collect();
    let manual = batch::edit(
        &scanned,
        EditRequest {
            groups: vec![GroupEdit {
                group_id: scanned.groups[0].id.clone(),
                file_ids: Some(ids.clone()),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let names = |plan: &batch::Plan| {
        plan.groups[0]
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let b_removed = batch::edit(
        &manual,
        EditRequest {
            remove_file_ids: vec![ids[1].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    let d_removed = batch::edit(
        &b_removed,
        EditRequest {
            remove_file_ids: vec![ids[3].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        d_removed
            .removed_sources
            .iter()
            .find(|s| s.file.id == ids[3])
            .unwrap()
            .position,
        3
    );
    let b_restored = batch::edit(
        &d_removed,
        EditRequest {
            restore_file_ids: vec![ids[1].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(names(&b_restored), "A.pdf,B.pdf,C.pdf");
    let all_restored = batch::edit(
        &b_restored,
        EditRequest {
            restore_file_ids: vec![ids[3].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(names(&all_restored), "A.pdf,B.pdf,C.pdf,D.pdf");
    let bc_removed = batch::edit(
        &manual,
        EditRequest {
            remove_file_ids: vec![ids[1].clone(), ids[2].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    let c_restored = batch::edit(
        &bc_removed,
        EditRequest {
            restore_file_ids: vec![ids[2].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(names(&c_restored), "A.pdf,C.pdf,D.pdf");
    let all = batch::edit(
        &c_restored,
        EditRequest {
            restore_file_ids: vec![ids[1].clone()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(names(&all), "A.pdf,B.pdf,C.pdf,D.pdf");
    let mut empty = manual;
    for index in [1, 0, 3, 2] {
        empty = batch::edit(
            &empty,
            EditRequest {
                remove_file_ids: vec![ids[index].clone()],
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert!(empty.groups[0].empty);
    for index in [1, 3, 0, 2] {
        empty = batch::edit(
            &empty,
            EditRequest {
                restore_file_ids: vec![ids[index].clone()],
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert_eq!(names(&empty), "A.pdf,B.pdf,C.pdf,D.pdf");
}

fn start_interactive(state: &Arc<AppState>, plan: batch::Plan, threads: usize) -> Job {
    let plan = state.insert_batch_plan(plan).unwrap();
    state
        .start_batch(
            &plan.id,
            "batch_generate",
            GenerateRequest::default(),
            threads,
        )
        .unwrap()
}
fn pending_conflict(state: &AppState, id: &str) -> batch::OutputConflict {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let job = state.job(id).unwrap();
        if let Some(conflict) = job.output_conflict.clone() {
            assert!(job.active());
            return conflict;
        }
        assert!(job.active(), "job ended without conflict: {:?}", job.error);
        assert!(Instant::now() < deadline, "conflict wait stuck");
        thread::sleep(Duration::from_millis(2));
    }
}
fn history_files(directory: &Path) -> Vec<PathBuf> {
    if !directory.exists() {
        return vec![];
    }
    let mut result = vec![];
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(history_files(&path));
        } else {
            result.push(path);
        }
    }
    result
}

#[test]
fn ordinary_existing_output_is_not_a_source_error_and_user_skip_never_serves_it() {
    let f = Fixture::new();
    f.pdf("source.pdf", 2);
    let state = f.state();
    let mut plan = f.scan(0);
    fs::create_dir_all(&plan.output_dir).unwrap();
    let target = PathBuf::from(&plan.groups[0].output_path);
    let path = target.as_path();
    static_pdf(path, 3, "foreign");
    let old = fs::read(path).unwrap();
    batch::inspect_existing_outputs(&state.dir, &mut plan).unwrap();
    assert!(plan.groups[0]
        .issues
        .iter()
        .all(|issue| issue.severity != "error"));
    assert!(plan.groups[0].files[0].error.is_none());
    let group_id = plan.groups[0].id.clone();
    let job = start_interactive(&state, plan.clone(), 4);
    let conflict = pending_conflict(&state, &job.id);
    assert_eq!(conflict.group_id, group_id);
    assert!(conflict.reason.contains("同名文件"));
    assert!(!conflict.reason.contains("manifest"));
    assert!(!conflict.reason.contains("os error"));
    assert!(batch::resolve_output_conflict(&state, &job.id, "stale", "overwrite", false).is_err());
    assert!(
        batch::resolve_output_conflict(&state, "wrong-job", &conflict.id, "skip", false).is_err()
    );
    assert!(
        batch::resolve_output_conflict(&state, &job.id, &conflict.id, "unsupported", false)
            .is_err()
    );
    let answered =
        batch::resolve_output_conflict(&state, &job.id, &conflict.id, "skip", false).unwrap();
    assert!(answered.output_conflict.is_none());
    let finished = wait(&state, &job.id);
    assert_eq!(finished.status, "completed");
    assert!(finished.output_conflict.is_none());
    assert_eq!((finished.done, finished.total), (2, 2));
    assert_eq!(finished.results[0].status, "skipped_existing");
    assert_eq!(finished.results[0].pages, 0);
    assert!(finished.results[0].output_path.is_none());
    assert!(finished.results[0].error.is_none());
    assert_eq!(fs::read(path).unwrap(), old);
    let report = batch::report(&state, &finished).unwrap();
    assert_eq!(report["successful_pages"], 0);
    assert!(report["manifests"].as_array().unwrap().is_empty());
    assert!(batch::read_output(&state, &finished, &group_id).is_err());
    assert!(
        batch::resolve_output_conflict(&state, &job.id, &conflict.id, "overwrite", false).is_err()
    );
}

#[test]
fn overwrite_requires_confirmation_for_all_records_and_keeps_no_old_copies() {
    for kind in ["unknown", "owned", "modified", "broken", "other-mode"] {
        let f = Fixture::new();
        f.pdf("source.pdf", 2);
        let state = f.state();
        let mut plan = f.scan(0);
        fs::create_dir_all(&plan.output_dir).unwrap();
        let target = PathBuf::from(&plan.groups[0].output_path);
        let path = target.as_path();
        if matches!(kind, "owned" | "modified" | "other-mode") {
            batch::build_group(
                &state.dir,
                &plan,
                &plan.groups[0],
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
        } else {
            static_pdf(path, 3, "foreign");
        }
        let record = pdf::manifest_path(&state.dir, path).unwrap();
        if kind == "modified" {
            OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(b"\n% externally modified\n")
                .unwrap();
        }
        if kind == "broken" {
            fs::create_dir_all(record.parent().unwrap()).unwrap();
            fs::write(&record, b"not JSON").unwrap();
        }
        if kind == "other-mode" {
            let mut old: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
            old["mode"] = "generic_pdf".into();
            pdf::atomic_json(&record, &old).unwrap();
        }
        let old = fs::read(path).unwrap();
        let old_hash = pdf::sha256_file(path).unwrap();
        let old_record = fs::read(&record).ok();
        let source_hash = pdf::sha256_file(&f.root.join("source.pdf")).unwrap();
        batch::inspect_existing_outputs(&state.dir, &mut plan).unwrap();
        assert!(
            plan.groups[0]
                .issues
                .iter()
                .all(|issue| issue.severity != "error"),
            "{kind}"
        );
        let job = start_interactive(&state, plan.clone(), 4);
        let conflict = pending_conflict(&state, &job.id);
        batch::resolve_output_conflict(&state, &job.id, &conflict.id, "overwrite", false).unwrap();
        let finished = wait(&state, &job.id);
        assert_eq!(finished.status, "completed", "{kind}: {:?}", finished.error);
        assert_eq!(finished.results[0].status, "completed");
        assert_eq!(finished.results[0].pages, 2);
        assert_eq!(Document::load(path).unwrap().get_pages().len(), 2);
        assert!(
            history_files(&Path::new(&plan.output_dir).join(".history")).is_empty(),
            "{kind}"
        );
        assert!(
            !record.parent().unwrap().join(".history").exists(),
            "{kind}"
        );
        assert!(!record.with_extension("json.previous").exists(), "{kind}");
        if let Some(old_record) = old_record {
            assert_ne!(fs::read(&record).unwrap(), old_record, "{kind}");
        }
        assert_ne!(Document::load_mem(&old).unwrap().get_pages().len(), 0);
        let report = batch::report(&state, &finished).unwrap();
        let approval = &report["manifests"][0]["overwrite_authorization"];
        assert_eq!(approval["previous_sha256"], old_hash);
        assert_eq!(approval["job_id"], job.id);
        assert_eq!(approval["policy_applied"], false);
        assert_eq!(
            pdf::sha256_file(&f.root.join("source.pdf")).unwrap(),
            source_hash
        );
        assert!(batch::read_output(&state, &finished, &plan.groups[0].id).is_ok());
    }
}

#[test]
fn apply_decision_is_per_job_parallel_and_retry_must_ask_again() {
    let f = Fixture::new();
    for i in 0..4 {
        f.pdf(&format!("group-{i}/source.pdf"), i + 1);
    }
    let state = f.state();
    let plan = f.scan(1);
    fs::create_dir_all(&plan.output_dir).unwrap();
    let mut previous = std::collections::HashMap::new();
    for group in &plan.groups {
        let path = Path::new(&group.output_path);
        static_pdf(path, 1, "existing");
        previous.insert(group.id.clone(), fs::read(path).unwrap());
    }
    let job = start_interactive(&state, plan.clone(), 4);
    let first = pending_conflict(&state, &job.id);
    batch::resolve_output_conflict(&state, &job.id, &first.id, "skip", true).unwrap();
    let skipped = wait(&state, &job.id);
    assert_eq!(skipped.status, "completed");
    assert_eq!(skipped.results.len(), 4);
    assert!(skipped
        .results
        .iter()
        .all(|r| r.status == "skipped_existing" && r.pages == 0));
    assert_eq!((skipped.done, skipped.total), (10, 10));
    for group in &plan.groups {
        assert_eq!(fs::read(&group.output_path).unwrap(), previous[&group.id]);
    }
    let retried = state.retry(&job.id).unwrap();
    let again = pending_conflict(&state, &retried.id);
    assert_ne!(again.id, first.id);
    batch::resolve_output_conflict(&state, &retried.id, &again.id, "overwrite", true).unwrap();
    let completed = wait(&state, &retried.id);
    assert_eq!(completed.status, "completed");
    assert!(completed.results.iter().all(|r| r.status == "completed"));
    let report = batch::report(&state, &completed).unwrap();
    let records = report["manifests"].as_array().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|m| m["overwrite_authorization"]["policy_applied"] == false)
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|m| m["overwrite_authorization"]["policy_applied"] == true)
            .count(),
        3
    );
    assert!(records
        .iter()
        .all(|m| m["overwrite_authorization"]["apply_to_batch"] == true));
}

#[test]
fn workers_publish_one_conflict_at_a_time_and_honor_individual_answers() {
    let f = Fixture::new();
    for i in 0..3 {
        f.pdf(&format!("group-{i}/source.pdf"), 1);
    }
    let state = f.state();
    let plan = f.scan(1);
    fs::create_dir_all(&plan.output_dir).unwrap();
    for group in &plan.groups {
        static_pdf(Path::new(&group.output_path), 2, "existing");
    }
    let job = start_interactive(&state, plan, 3);
    let mut ids = std::collections::HashSet::new();
    let mut decisions = std::collections::HashMap::new();
    for action in ["skip", "overwrite", "skip"] {
        let pending = pending_conflict(&state, &job.id);
        assert!(ids.insert(pending.id.clone()));
        decisions.insert(pending.group_id.clone(), action);
        batch::resolve_output_conflict(&state, &job.id, &pending.id, action, false).unwrap();
    }
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "completed");
    for result in &done.results {
        assert_eq!(
            result.status,
            if decisions[&result.voucher_id] == "skip" {
                "skipped_existing"
            } else {
                "completed"
            }
        );
    }
    assert!(done.output_conflict.is_none());
}

#[test]
fn cancel_or_shutdown_while_workers_wait_clears_pending_and_keeps_all_existing_bytes() {
    for shutdown in [false, true] {
        let f = Fixture::new();
        for i in 0..4 {
            f.pdf(&format!("group-{i}/source.pdf"), 1);
        }
        let state = f.state();
        let plan = f.scan(1);
        fs::create_dir_all(&plan.output_dir).unwrap();
        let mut previous = vec![];
        for group in &plan.groups {
            static_pdf(Path::new(&group.output_path), 2, "existing");
            previous.push(fs::read(&group.output_path).unwrap());
        }
        let job = start_interactive(&state, plan.clone(), 4);
        let pending = pending_conflict(&state, &job.id);
        if shutdown {
            state.shutting_down.store(true, Ordering::Release);
            state.data.lock().unwrap().cancels[&job.id].store(true, Ordering::Release);
        } else {
            assert!(state.cancel(&job.id).unwrap().output_conflict.is_none());
        }
        let done = wait(&state, &job.id);
        assert_eq!(done.status, "cancelled");
        assert!(done.output_conflict.is_none());
        assert!(
            batch::resolve_output_conflict(&state, &job.id, &pending.id, "overwrite", true)
                .is_err()
        );
        for (group, bytes) in plan.groups.iter().zip(previous) {
            assert_eq!(fs::read(&group.output_path).unwrap(), bytes);
        }
        assert!(history_files(&Path::new(&plan.output_dir).join(".history")).is_empty());
    }
}

#[test]
fn failed_decision_persistence_leaves_the_same_prompt_and_never_executes_overwrite() {
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let state = f.state();
    let plan = f.scan(0);
    fs::create_dir_all(&plan.output_dir).unwrap();
    let path = Path::new(&plan.groups[0].output_path);
    static_pdf(path, 2, "existing");
    let bytes = fs::read(path).unwrap();
    let job = start_interactive(&state, plan.clone(), 1);
    let pending = pending_conflict(&state, &job.id);
    let jobs = state.dir.join("jobs");
    let backup = state.dir.join("jobs-backup");
    fs::rename(&jobs, &backup).unwrap();
    fs::write(&jobs, b"block job persistence").unwrap();
    let error = batch::resolve_output_conflict(&state, &job.id, &pending.id, "overwrite", true)
        .unwrap_err();
    assert!(error.to_string().contains("尚未执行"));
    thread::sleep(Duration::from_millis(150));
    let still_waiting = state.job(&job.id).unwrap();
    assert!(still_waiting.active());
    assert_eq!(still_waiting.output_conflict.unwrap().id, pending.id);
    assert_eq!(fs::read(path).unwrap(), bytes);
    fs::remove_file(jobs).unwrap();
    fs::rename(backup, state.dir.join("jobs")).unwrap();
    batch::resolve_output_conflict(&state, &job.id, &pending.id, "skip", false).unwrap();
    assert_eq!(wait(&state, &job.id).results[0].status, "skipped_existing");
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn externally_changed_target_during_confirmation_is_not_overwritten() {
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let state = f.state();
    let plan = f.scan(0);
    fs::create_dir_all(&plan.output_dir).unwrap();
    let path = Path::new(&plan.groups[0].output_path);
    static_pdf(path, 2, "existing");
    let job = start_interactive(&state, plan.clone(), 1);
    let pending = pending_conflict(&state, &job.id);
    static_pdf(path, 5, "external update");
    let changed = fs::read(path).unwrap();
    batch::resolve_output_conflict(&state, &job.id, &pending.id, "overwrite", false).unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "completed_with_errors");
    assert_eq!(done.results[0].status, "failed");
    assert!(done.results[0]
        .error
        .as_deref()
        .unwrap()
        .contains("等待确认期间"));
    assert_eq!(fs::read(path).unwrap(), changed);
    assert!(history_files(&Path::new(&plan.output_dir).join(".history")).is_empty());
}

#[test]
fn restart_clears_interrupted_prompt_and_retry_asks_for_new_permission() {
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let state = f.state();
    let plan = state.insert_batch_plan(f.scan(0)).unwrap();
    fs::create_dir_all(&plan.output_dir).unwrap();
    let path = Path::new(&plan.groups[0].output_path);
    static_pdf(path, 2, "existing");
    let before = fs::read(path).unwrap();
    let stale = batch::OutputConflict {
        id: uuid::Uuid::new_v4().to_string(),
        group_id: plan.groups[0].id.clone(),
        group_name: plan.groups[0].name.clone(),
        output_path: plan.groups[0].output_path.clone(),
        reason: "synthetic interrupted prompt".into(),
    };
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: plan.id.clone(),
        kind: "batch_generate".into(),
        status: "running".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        total: 1,
        done: 0,
        current: String::new(),
        error: None,
        results: vec![],
        request: GenerateRequest::default(),
        threads: 7,
        output_conflict: Some(stale.clone()),
        orientation_summary: None,
    };
    pdf::atomic_json(
        &state.dir.join("jobs").join(format!("{}.json", job.id)),
        &job,
    )
    .unwrap();
    let loaded = f.state();
    let interrupted = loaded.job(&job.id).unwrap();
    assert_eq!(interrupted.status, "interrupted");
    assert!(interrupted.output_conflict.is_none());
    assert!(
        batch::resolve_output_conflict(&loaded, &job.id, &stale.id, "overwrite", true).is_err()
    );
    let retry = loaded.retry(&job.id).unwrap();
    assert_eq!(retry.threads, 7);
    let prompt = pending_conflict(&loaded, &retry.id);
    assert_ne!(prompt.id, stale.id);
    batch::resolve_output_conflict(&loaded, &retry.id, &prompt.id, "skip", false).unwrap();
    assert_eq!(
        wait(&loaded, &retry.id).results[0].status,
        "skipped_existing"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn unique_case_variant_is_confirmed_and_history_download_uses_the_physical_file() {
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let state = f.state();
    let plan = f.scan(0);
    fs::create_dir_all(&plan.output_dir).unwrap();
    let actual = Path::new(&plan.output_dir).join(plan.groups[0].output_name.to_uppercase());
    static_pdf(&actual, 2, "existing uppercase extension");
    let old = fs::read(&actual).unwrap();
    let job = start_interactive(&state, plan.clone(), 1);
    let prompt = pending_conflict(&state, &job.id);
    assert_eq!(Path::new(&prompt.output_path), actual);
    batch::resolve_output_conflict(&state, &job.id, &prompt.id, "overwrite", false).unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "completed", "{:?}", done.error);
    assert_eq!(
        batch::output(&state, &done, &plan.groups[0].id).unwrap(),
        actual
    );
    let report = batch::report(&state, &done).unwrap();
    assert_eq!(
        report["manifests"][0]["output_name"],
        plan.groups[0].output_name
    );
    assert_eq!(
        Path::new(report["manifests"][0]["output_path"].as_str().unwrap()),
        actual
    );
    assert!(batch::read_output(&state, &done, &plan.groups[0].id).is_ok());
    assert!(history_files(&Path::new(&plan.output_dir).join(".history")).is_empty());
    assert_ne!(fs::read(&actual).unwrap(), old);
    let desired = Path::new(&plan.groups[0].output_path);
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(desired)
    {
        Ok(mut other) => {
            other.write_all(b"distinct case-sensitive target").unwrap();
            drop(other);
            let current = fs::read(&actual).unwrap();
            let mut inspected = plan;
            batch::inspect_existing_outputs(&state.dir, &mut inspected).unwrap();
            assert!(inspected.groups[0]
                .issues
                .iter()
                .any(|issue| issue.code == "output_unsafe"));
            assert_eq!(fs::read(actual).unwrap(), current);
        }
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists),
    }
}

#[test]
fn source_as_output_and_nonordinary_targets_never_offer_an_overwrite() {
    let f = Fixture::new();
    f.pdf("delivery/source.pdf", 1);
    let state = f.state();
    let mut plan = f.scan(0);
    let source = f.root.join("delivery/source.pdf");
    let before = fs::read(&source).unwrap();
    plan.output_dir = f.root.join("delivery").to_string_lossy().into();
    plan.groups[0].output_name = "source.pdf".into();
    plan.groups[0].output_path = source.to_string_lossy().into();
    let job = start_interactive(&state, plan, 1);
    let failed = wait(&state, &job.id);
    assert_eq!(failed.status, "completed_with_errors");
    assert!(failed.output_conflict.is_none());
    assert!(failed.results[0]
        .error
        .as_deref()
        .unwrap()
        .contains("源文件相同"));
    assert_eq!(fs::read(source).unwrap(), before);
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let mut plan = f.scan(0);
    fs::create_dir_all(&plan.groups[0].output_path).unwrap();
    batch::inspect_existing_outputs(&f.state_dir, &mut plan).unwrap();
    assert!(plan.groups[0]
        .issues
        .iter()
        .any(|issue| issue.code == "output_unsafe"));
    assert!(plan.groups[0].files[0].error.is_none());
}

#[cfg(unix)]
#[test]
fn symbolic_link_output_cannot_be_approved_as_an_ordinary_existing_file() {
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let mut plan = f.scan(0);
    fs::create_dir_all(&plan.output_dir).unwrap();
    let source = f.root.join("source.pdf");
    let before = fs::read(&source).unwrap();
    std::os::unix::fs::symlink(&source, &plan.groups[0].output_path).unwrap();
    batch::inspect_existing_outputs(&f.state_dir, &mut plan).unwrap();
    assert!(plan.groups[0]
        .issues
        .iter()
        .any(|issue| issue.code == "output_unsafe"));
    assert_eq!(fs::read(source).unwrap(), before);
}

#[tokio::test]
async fn output_conflict_http_route_rejects_stale_answers_and_returns_a_cleared_prompt() {
    let f = Fixture::new();
    f.pdf("source.pdf", 1);
    let state = f.state();
    let plan = f.scan(0);
    fs::create_dir_all(&plan.output_dir).unwrap();
    static_pdf(Path::new(&plan.groups[0].output_path), 2, "existing");
    let job = start_interactive(&state, plan, 1);
    let prompt = pending_conflict(&state, &job.id);
    let app = batch::router().with_state(state.clone());
    let endpoint = format!("/jobs/{}/output-conflict", job.id);
    assert_eq!(
        call(
            &app,
            &endpoint,
            json!({"conflict_id":"stale","action":"overwrite","apply_to_batch":false})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, response) = call(
        &app,
        &endpoint,
        json!({"conflict_id":prompt.id,"action":"skip","apply_to_batch":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["output_conflict"], Value::Null);
    assert_eq!(wait(&state, &job.id).results[0].status, "skipped_existing");
}

#[test]
fn analysis_summary_counts_all_outcomes_but_only_changed_files_have_details_or_markers() {
    let f = Fixture::new();
    for i in 0..15 {
        f.image(&format!("{i:02}.png"), 20, 30);
    }
    let mut plan = f.scan(0);
    cached(&mut plan, 90);
    for (index, file) in plan.groups[0].files.iter_mut().enumerate() {
        file.auto_rotation_change = Some(batch::RotationChange {
            from_cw: 0,
            to_cw: 90,
        });
        match index % 7 {
            0 => file.rotation_cw = 0,
            1 => {
                file.rotation_cw = 90;
                file.orientation.as_mut().unwrap().rotation_cw = 0;
            }
            2 => {
                file.rotation_cw = 0;
                file.orientation.as_mut().unwrap().rotation_cw = 0;
            }
            3 => file.rotation_cw = 90,
            4 => {
                file.rotation_cw = 90;
                let orientation = file.orientation.as_mut().unwrap();
                orientation.rotation_cw = 0;
                orientation.needs_review = true;
            }
            5 => {
                file.rotation_cw = 90;
                file.manual_rotation = true;
            }
            _ => {
                file.rotation_cw = 90;
                file.reviewed = true;
            }
        }
    }
    plan.groups[0].files[14].rotation_cw = 90;
    fs::write(f.root.join("14.png"), b"changed source").unwrap();
    let state = f.state();
    let plan = state
        .insert_batch_plan(batch::analysis_revision(&plan, 4).unwrap())
        .unwrap();
    let job = state
        .start_batch(&plan.id, "batch_analyze", GenerateRequest::default(), 4)
        .unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "failed");
    assert_eq!((done.done, done.total), (15, 15));
    assert_eq!(
        done.orientation_summary,
        Some(batch::AnalysisSummary {
            changed: 4,
            unchanged: 6,
            skipped: 4,
            failed: 1
        })
    );
    assert_eq!(done.results.len(), 4);
    assert!(done
        .results
        .iter()
        .all(|result| result.rotation_change.is_some() && result.error.is_none()));
    let after = state.batch_plan(&plan.id).unwrap();
    assert_eq!(after.groups[0].review_count, 4);
    assert_eq!(
        after.groups[0]
            .files
            .iter()
            .filter(|file| file.auto_rotation_change.is_some())
            .count(),
        4
    );
    assert_eq!(after.groups[0].files[4].rotation_cw, 90);
    assert!(after.groups[0].files[4].auto_rotation_change.is_none());
    assert_eq!(after.groups[0].files[14].rotation_cw, 90);
    assert!(after.groups[0].files[14].error.is_some());
    assert!(after.groups[0].files[14].auto_rotation_change.is_none());
    assert!(done.results.iter().any(|result| result.rotation_change
        == Some(batch::RotationChange {
            from_cw: 90,
            to_cw: 0
        })));
    let report = batch::report(&state, &done).unwrap();
    assert_eq!(report["analyzed_files"], 15);
    assert_eq!(report["unreviewed_files"], 4);
    assert_eq!(report["unreviewed_pages"], 4);
    let retried = state.retry(&done.id).unwrap();
    let retried = wait(&state, &retried.id);
    assert_eq!(retried.status, "completed");
    assert_eq!(
        retried.orientation_summary,
        Some(batch::AnalysisSummary {
            changed: 0,
            unchanged: 10,
            skipped: 4,
            failed: 0
        })
    );
    assert_eq!(retried.total, 14);
    assert!(retried.results.is_empty());
}

#[test]
fn selected_analysis_clears_old_markers_without_touching_other_groups_and_manual_edits_clear_them()
{
    let f = Fixture::new();
    f.image("A/1.png", 20, 30);
    f.image("B/1.png", 20, 30);
    let mut plan = f.scan(1);
    cached(&mut plan, 90);
    for file in plan.groups.iter_mut().flat_map(|group| &mut group.files) {
        file.rotation_cw = 90;
        file.auto_rotation_change = Some(batch::RotationChange {
            from_cw: 0,
            to_cw: 90,
        });
    }
    let state = f.state();
    let plan = state
        .insert_batch_plan(batch::analysis_revision(&plan, 4).unwrap())
        .unwrap();
    let job = state
        .start_batch(
            &plan.id,
            "batch_analyze",
            GenerateRequest {
                voucher_ids: Some(vec![plan.groups[0].id.clone()]),
                allow_unreviewed: false,
            },
            4,
        )
        .unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(
        done.orientation_summary,
        Some(batch::AnalysisSummary {
            changed: 0,
            unchanged: 1,
            skipped: 0,
            failed: 0
        })
    );
    assert!(done.results.is_empty());
    let after = state.batch_plan(&plan.id).unwrap();
    assert!(after.groups[0].files[0].auto_rotation_change.is_none());
    assert_eq!(after.groups[0].review_count, 0);
    assert!(after.groups[1].files[0].auto_rotation_change.is_some());
    assert_eq!(after.groups[1].review_count, 1);
    let edited = batch::edit(
        &after,
        EditRequest {
            files: vec![FileEdit {
                file_id: after.groups[1].files[0].id.clone(),
                rotation_cw: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(edited.groups[1].files[0].auto_rotation_change.is_none());
    assert!(edited.groups[1].files[0].manual_rotation);
    assert_eq!(edited.groups[1].review_count, 0);
}

#[test]
fn cancelling_cached_parallel_analysis_keeps_summary_and_change_details_consistent() {
    let f = Fixture::new();
    for index in 0..100 {
        f.image(&format!("{index:03}.png"), 20, 30);
    }
    let mut plan = f.scan(0);
    cached(&mut plan, 90);
    for file in &mut plan.groups[0].files {
        file.auto_rotation_change = Some(batch::RotationChange {
            from_cw: 90,
            to_cw: 0,
        });
    }
    let state = f.state();
    let plan = state
        .insert_batch_plan(batch::analysis_revision(&plan, 10).unwrap())
        .unwrap();
    let job = state
        .start_batch(&plan.id, "batch_analyze", GenerateRequest::default(), 10)
        .unwrap();
    state.cancel(&job.id).unwrap();
    let done = wait(&state, &job.id);
    assert_eq!(done.status, "cancelled");
    let summary = done.orientation_summary.unwrap();
    assert_eq!(
        summary.changed + summary.unchanged + summary.skipped + summary.failed,
        done.done
    );
    assert_eq!(summary.changed as usize, done.results.len());
    assert!(done
        .results
        .iter()
        .all(|result| result.rotation_change.is_some()));
    let after = state.batch_plan(&plan.id).unwrap();
    assert_eq!(
        after.groups[0]
            .files
            .iter()
            .filter(|file| file.auto_rotation_change.is_some())
            .count(),
        done.results.len()
    );
    assert!(!after.orientation_workflow.analysis_completed);
}

#[test]
fn no_backup_replacement_recovers_every_commit_boundary_and_rejects_damaged_pending_bytes() {
    for stage in 0..4 {
        let f = Fixture::new();
        f.pdf("source.pdf", 1);
        let plan = f.scan(0);
        let built = batch::build_group(
            &f.state_dir,
            &plan,
            &plan.groups[0],
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let new_pdf = fs::read(&built.path).unwrap();
        let record = pdf::manifest_path(&f.state_dir, &built.path).unwrap();
        let pending = Path::new(&plan.output_dir).join(".pending.pdf");
        fs::write(&built.path, b"old existing file").unwrap();
        let previous_hash = pdf::sha256_file(&built.path).unwrap();
        fs::write(&record, b"old record bytes").unwrap();
        fs::write(
            &pending,
            if stage == 3 {
                b"damaged".as_slice()
            } else {
                new_pdf.as_slice()
            },
        )
        .unwrap();
        let journal = record.with_file_name("transaction.json");
        pdf::atomic_json(&journal,&json!({"replace_existing":true,"temporary":".pending.pdf","output":plan.groups[0].output_name,"backup":null,"previous_hash":previous_hash,"manifest":built.manifest})).unwrap();
        if stage == 1 || stage == 2 {
            fs::write(&built.path, &new_pdf).unwrap();
            fs::remove_file(&pending).unwrap();
        }
        if stage == 2 {
            pdf::atomic_json(&record, &built.manifest).unwrap();
        }
        let recovered = batch::build_group(
            &f.state_dir,
            &plan,
            &plan.groups[0],
            &AtomicBool::new(false),
            &mut |_| {},
        );
        if stage == 3 {
            assert!(recovered.is_err());
            assert_eq!(fs::read(&built.path).unwrap(), b"old existing file");
            assert_eq!(fs::read(&record).unwrap(), b"old record bytes");
            assert!(journal.exists());
        } else {
            assert!(recovered.unwrap().skipped);
            assert_eq!(fs::read(&built.path).unwrap(), new_pdf);
            assert!(!journal.exists());
            assert!(!pending.exists());
            assert!(serde_json::from_slice::<batch::Manifest>(&fs::read(&record).unwrap()).is_ok());
        }
        assert!(!Path::new(&plan.output_dir).join(".history").exists());
        assert!(!record.parent().unwrap().join(".history").exists());
        assert!(!record.with_extension("json.previous").exists());
    }
}

#[test]
fn cancelled_owned_rebuild_preserves_old_target_and_later_overwrite_preserves_preexisting_history_only(
) {
    let f = Fixture::new();
    f.image("source.png", 20, 30);
    let plan = f.scan(0);
    let old = batch::build_group(
        &f.state_dir,
        &plan,
        &plan.groups[0],
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let bytes = fs::read(&old.path).unwrap();
    let record = pdf::manifest_path(&f.state_dir, &old.path).unwrap();
    let record_bytes = fs::read(&record).unwrap();
    let historical = Path::new(&plan.output_dir).join(".history/preexisting/keep.pdf");
    fs::create_dir_all(historical.parent().unwrap()).unwrap();
    fs::write(&historical, b"preexisting historical bytes").unwrap();
    let changed = batch::edit(
        &plan,
        EditRequest {
            files: vec![FileEdit {
                file_id: plan.groups[0].files[0].id.clone(),
                rotation_cw: Some(90),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    assert!(batch::build_group(
        &f.state_dir,
        &changed,
        &changed.groups[0],
        &cancel,
        &mut |_| cancel.store(true, Ordering::Release)
    )
    .is_err());
    assert_eq!(fs::read(&old.path).unwrap(), bytes);
    assert_eq!(fs::read(&record).unwrap(), record_bytes);
    let state = f.state();
    let done = generated(&state, changed, 4);
    assert_eq!(done.status, "completed");
    assert_eq!(
        history_files(&Path::new(&plan.output_dir).join(".history")),
        vec![historical.clone()]
    );
    assert_eq!(
        fs::read(historical).unwrap(),
        b"preexisting historical bytes"
    );
    assert!(!record.parent().unwrap().join(".history").exists());
}

#[test]
fn old_records_without_change_fields_keep_none_and_derived_counts_do_not_rewrite_evidence() {
    let f = Fixture::new();
    f.image("source.png", 20, 30);
    let mut old_plan = f.scan(0);
    cached(&mut old_plan, 90);
    old_plan.groups[0].files[0].rotation_cw = 90;
    old_plan.groups[0].review_count = 99;
    let state = f.state();
    let built = batch::build_group(
        &state.dir,
        &old_plan,
        &old_plan.groups[0],
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let mut mapping = serde_json::to_value(&built.manifest.sources[0]).unwrap();
    mapping
        .as_object_mut()
        .unwrap()
        .remove("auto_rotation_change");
    let decoded: batch::Mapping = serde_json::from_value(mapping).unwrap();
    assert!(decoded.auto_rotation_change.is_none());
    let mut legacy_plan = serde_json::to_value(&old_plan).unwrap();
    legacy_plan["groups"][0]["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("auto_rotation_change");
    let plan_path = state
        .dir
        .join("batch-plans")
        .join(format!("{}.json", old_plan.id));
    pdf::atomic_json(&plan_path, &legacy_plan).unwrap();
    let old_plan_bytes = fs::read(&plan_path).unwrap();
    let job_id = "legacy-analysis";
    let legacy_job = json!({"id":job_id,"plan_id":old_plan.id,"kind":"batch_analyze","status":"completed","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","total":1,"done":1,"current":"legacy evidence","error":null,"results":[{"voucher_id":old_plan.groups[0].files[0].id,"folder":"source.png","status":"completed","output_path":null,"pages":1,"error":null,"needs_review":true}],"request":{"voucher_ids":null,"allow_unreviewed":false}});
    let job_path = state.dir.join("jobs").join(format!("{job_id}.json"));
    pdf::atomic_json(&job_path, &legacy_job).unwrap();
    let old_job_bytes = fs::read(&job_path).unwrap();
    let directory = state.dir.join("reports").join(job_id);
    fs::create_dir_all(&directory).unwrap();
    let snapshot = directory.join("batch-plan.json");
    pdf::atomic_json(&snapshot, &legacy_plan).unwrap();
    let old_snapshot_bytes = fs::read(&snapshot).unwrap();
    let loaded = f.state();
    let plan = loaded.batch_plan(&old_plan.id).unwrap();
    assert!(plan.groups[0].files[0].auto_rotation_change.is_none());
    assert_eq!(plan.groups[0].files[0].rotation_cw, 90);
    assert_eq!(plan.groups[0].review_count, 0);
    assert!(!batch::needs_review(&plan.groups[0].files[0]));
    let job = loaded.job(job_id).unwrap();
    assert!(job.orientation_summary.is_none());
    assert!(job.results[0].rotation_change.is_none());
    let report = batch::report(&loaded, &job).unwrap();
    assert_eq!(report["analyzed_files"], 1);
    assert_eq!(report["unreviewed_files"], 0);
    assert_eq!(report["groups"][0]["review_count"], 0);
    assert_eq!(fs::read(plan_path).unwrap(), old_plan_bytes);
    assert_eq!(fs::read(job_path).unwrap(), old_job_bytes);
    assert_eq!(fs::read(snapshot).unwrap(), old_snapshot_bytes);
}
