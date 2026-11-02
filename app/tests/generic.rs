//! Generated PDF fixtures only. These tests never read customer inputs or the
//! default state directory, and do not require Excel or an OCR installation.
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
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tower::ServiceExt;
use voucher_workbench::{
    generic::{self, GroupOrder, OrderRequest, ScanRequest},
    jobs::{AppState, GenerateRequest, Job, ProcessingSettings},
    pdf,
};

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    state_dir: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("合成 资料");
        fs::create_dir_all(&root).unwrap();
        Self {
            _temp: temp,
            root,
            state_dir: base.join("state"),
            output: base.join("PDF 交付"),
        }
    }
    fn state(&self) -> Arc<AppState> {
        AppState::load(
            self.state_dir.clone(),
            &self.state_dir.join("no-model"),
            None,
        )
        .unwrap()
    }
    fn request(&self, level: u8) -> ScanRequest {
        ScanRequest {
            root: self.root.to_str().unwrap().into(),
            output_dir: Some(self.output.to_str().unwrap().into()),
            level,
            exclude_paths: vec![],
            exclude_globs: vec!["**/indexImg/**".into()],
            sort: "natural_asc".into(),
        }
    }
    fn pdf(&self, relative: &str, pages: u32) {
        static_pdf(&self.root.join(relative), pages, relative);
    }
    fn nine_pages(&self) {
        self.pdf("总说明.pdf", 2);
        self.pdf("A/01.pdf", 1);
        self.pdf("A/02.pdf", 3);
        self.pdf("A/X/10.pdf", 2);
        self.pdf("B/03.pdf", 1);
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
fn generate(state: &Arc<AppState>, plan: generic::Plan, threads: usize) -> Job {
    state.set_settings(ProcessingSettings { threads }).unwrap();
    let plan = state.insert_generic_plan(plan).unwrap();
    let ids = plan
        .groups
        .iter()
        .filter(|g| !g.empty && g.issues.iter().all(|i| i.severity != "error"))
        .map(|g| g.id.clone())
        .collect();
    let job = state
        .start(
            &plan.id,
            "generic_generate",
            GenerateRequest {
                voucher_ids: Some(ids),
                allow_unreviewed: false,
            },
        )
        .unwrap();
    wait(state, &job.id)
}

#[test]
fn depth_groups_preserve_shallow_files_and_nine_pages() {
    let f = Fixture::new();
    f.nine_pages();
    for (level, groups) in [(0, 1), (1, 3), (2, 4), (32, 4)] {
        let plan = generic::scan(f.request(level)).unwrap();
        assert_eq!(plan.groups.len(), groups);
        assert_eq!(plan.groups.iter().map(|g| g.pages).sum::<u32>(), 9);
        let files: Vec<_> = plan
            .groups
            .iter()
            .flat_map(|g| g.files.iter().map(|f| &f.relative_path))
            .collect();
        let unique: std::collections::HashSet<_> = files.iter().collect();
        assert_eq!(files.len(), 5);
        assert_eq!(unique.len(), 5);
        if level == 2 {
            assert_eq!(plan.groups.iter().filter(|g| g.shallow).count(), 3);
        }
    }
    fs::create_dir_all(f.root.join("Empty/Deep")).unwrap();
    let plan = generic::scan(f.request(1)).unwrap();
    let empty = plan.groups.iter().find(|g| g.name == "Empty").unwrap();
    assert!(empty.empty);
    assert!(plan.empty_directories.contains(&"Empty/Deep".into()));
    assert!(generic::validate_generation(&plan, &GenerateRequest::default()).is_err());
}

#[test]
fn exclusions_preview_recursion_and_nested_output_do_not_read_back() {
    let f = Fixture::new();
    f.nine_pages();
    f.pdf("indexImg/hidden.pdf", 1);
    f.pdf("A/indexImg/hidden.pdf", 1);
    f.pdf("A/draft.pdf", 1);
    f.pdf("delivery/old.pdf", 1);
    fs::write(f.root.join("index.html"), b"ignored").unwrap();
    let mut req = f.request(1);
    req.output_dir = Some(f.root.join("delivery").to_str().unwrap().into());
    req.exclude_paths = vec!["B".into()];
    req.exclude_globs.push("**/draft?.pdf".into());
    req.exclude_globs.push("A/draft.pdf".into());
    let plan = generic::scan(req.clone()).unwrap();
    let files: Vec<_> = plan.groups.iter().flat_map(|g| g.files.iter()).collect();
    assert_eq!(files.len(), 4);
    assert!(!files.iter().any(|p| p.relative_path.contains("hidden")
        || p.relative_path.contains("delivery")
        || p.relative_path.contains("draft")));
    assert!(plan
        .exclusions
        .iter()
        .any(|e| e.kind == "output" && e.matches == vec!["delivery/"]));
    assert!(plan
        .exclusions
        .iter()
        .any(|e| e.rule == "B" && e.matches == vec!["B/"]));
    assert!(plan
        .exclusions
        .iter()
        .any(|e| e.rule == "**/draft?.pdf" && e.matches.is_empty()));
    req.exclude_globs.clear();
    let plan = generic::scan(req).unwrap();
    assert!(plan
        .groups
        .iter()
        .flat_map(|g| &g.files)
        .any(|p| p.relative_path.contains("indexImg")));
    for invalid in [
        "../*.pdf",
        "/a/*",
        "C:\\a\\*",
        "**x/*.pdf",
        "[a].pdf",
        "A//*.pdf",
    ] {
        let mut req = f.request(0);
        req.exclude_globs = vec![invalid.into()];
        assert!(generic::scan(req).is_err(), "{invalid}");
    }
    let mut req = f.request(0);
    req.output_dir = Some(f.root.to_str().unwrap().into());
    assert!(generic::scan(req).is_err());
    let mut req = f.request(0);
    req.output_dir = Some(f.root.parent().unwrap().to_str().unwrap().into());
    assert_eq!(generic::scan(req).unwrap().groups[0].files.len(), 7);
}

#[test]
fn automatic_order_is_stable_and_manual_order_is_an_exact_immutable_permutation() {
    let f = Fixture::new();
    for name in ["10.pdf", "2.pdf", "1.pdf", "0001.pdf"] {
        f.pdf(name, 1);
    }
    let mut req = f.request(0);
    let plan = generic::scan(req.clone()).unwrap();
    assert_eq!(
        plan.groups[0]
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["0001.pdf", "1.pdf", "2.pdf", "10.pdf"]
    );
    req.sort = "natural_desc".into();
    assert_eq!(
        generic::scan(req.clone()).unwrap().groups[0].files[0].name,
        "10.pdf"
    );
    let time = fs::metadata(f.root.join("1.pdf"))
        .unwrap()
        .modified()
        .unwrap();
    for file in &plan.groups[0].files {
        OpenOptions::new()
            .write(true)
            .open(f.root.join(&file.relative_path))
            .unwrap()
            .set_times(FileTimes::new().set_modified(time))
            .unwrap();
    }
    req.sort = "modified_desc".into();
    let timed = generic::scan(req).unwrap();
    assert_eq!(
        timed.groups[0]
            .files
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>(),
        plan.groups[0]
            .files
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>()
    );
    let group = &plan.groups[0];
    let reversed: Vec<_> = group.files.iter().rev().map(|f| f.id.clone()).collect();
    let changed = generic::reorder(
        &plan,
        OrderRequest {
            groups: vec![GroupOrder {
                group_id: group.id.clone(),
                file_ids: reversed.clone(),
                output_name: Some("CON.pdf".into()),
            }],
        },
    )
    .unwrap();
    assert_ne!(changed.id, plan.id);
    assert_eq!(changed.parent_plan_id.as_deref(), Some(plan.id.as_str()));
    assert_eq!(changed.groups[0].id, group.id);
    assert_eq!(changed.groups[0].output_name, "_CON.pdf");
    assert_eq!(plan.groups[0].files[0].name, "0001.pdf");
    assert_eq!(changed.groups[0].files[0].id, reversed[0]);
    for ids in [
        vec![],
        vec![group.files[0].id.clone(); group.files.len()],
        vec!["unregistered-path".into(); group.files.len()],
    ] {
        assert!(generic::reorder(
            &plan,
            OrderRequest {
                groups: vec![GroupOrder {
                    group_id: group.id.clone(),
                    file_ids: ids,
                    output_name: None
                }]
            }
        )
        .is_err());
    }
    assert!(generic::reorder(
        &plan,
        OrderRequest {
            groups: vec![GroupOrder {
                group_id: group.id.clone(),
                file_ids: reversed,
                output_name: Some("../escape.pdf".into())
            }]
        }
    )
    .is_err());
}

#[test]
fn output_collisions_are_blocked_and_editable() {
    let f = Fixture::new();
    f.pdf("A/B/a.pdf", 1);
    f.pdf("A__B/b.pdf", 1);
    let plan = generic::scan(f.request(2)).unwrap();
    assert_eq!(plan.groups.len(), 2);
    assert!(plan
        .groups
        .iter()
        .all(|g| g.issues.iter().any(|i| i.code == "output_collision")));
    let group = &plan.groups[1];
    let changed = generic::reorder(
        &plan,
        OrderRequest {
            groups: vec![GroupOrder {
                group_id: group.id.clone(),
                file_ids: group.files.iter().map(|f| f.id.clone()).collect(),
                output_name: Some("renamed.pdf".into()),
            }],
        },
    )
    .unwrap();
    assert!(changed.groups.iter().all(|g| g.issues.is_empty()));
    let mut req = f.request(1);
    req.level = 33;
    assert!(generic::scan(req).is_err());
}

#[test]
fn parallel_generation_preserves_pdf_pages_rotation_sources_and_snapshot_download() {
    for threads in [1, 10] {
        let f = Fixture::new();
        f.nine_pages();
        let state = f.state();
        let plan = generic::scan(f.request(2)).unwrap();
        let job = generate(&state, plan.clone(), threads);
        assert_eq!(job.status, "completed", "{:?}", job.results);
        assert_eq!((job.total, job.done), (9, 9));
        assert_eq!(job.threads, threads);
        assert_eq!(
            job.results
                .iter()
                .map(|r| &r.voucher_id)
                .collect::<Vec<_>>(),
            plan.groups.iter().map(|g| &g.id).collect::<Vec<_>>()
        );
        let report = generic::report(&state, &job).unwrap();
        assert_eq!(report["mode"], "generic_pdf");
        assert_eq!(report["successful_pages"], 9);
        for group in &plan.groups {
            let path = generic::output(&state, &job, &group.id).unwrap();
            let doc = Document::load(&path).unwrap();
            assert_eq!(doc.get_pages().len() as u32, group.pages);
            assert!(!path.with_extension("manifest.json").exists());
            let manifest: generic::Manifest = serde_json::from_slice(
                &fs::read(pdf::manifest_path(&state.dir, &path).unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest.output_sha256, pdf::sha256_file(&path).unwrap());
            assert_eq!(
                manifest
                    .sources
                    .iter()
                    .map(|m| &m.source)
                    .collect::<Vec<_>>(),
                group
                    .files
                    .iter()
                    .map(|f| &f.relative_path)
                    .collect::<Vec<_>>()
            );
            let pages = doc.get_pages();
            let mut index = 1;
            for file in &group.files {
                for page in 1..=file.pages {
                    let d = doc.get_object(pages[&index]).unwrap().as_dict().unwrap();
                    assert_eq!(
                        d.get(b"Rotate").unwrap().as_i64().unwrap(),
                        if page % 2 == 0 { 90 } else { 0 }
                    );
                    assert_eq!(
                        d.get(b"MediaBox").unwrap().as_array().unwrap()[2]
                            .as_i64()
                            .unwrap(),
                        300 + page as i64
                    );
                    index += 1;
                }
            }
        }
        let retry = state.retry(&job.id).unwrap();
        let retried = wait(&state, &retry.id);
        assert!(retried.results.iter().all(|r| r.status == "skipped"));
        assert_eq!(retried.threads, threads);
        // Changing live plan storage must never change a historical download.
        state
            .data
            .lock()
            .unwrap()
            .generic_plans
            .get_mut(&plan.id)
            .unwrap()
            .output_dir = f.root.to_string_lossy().into_owned();
        assert!(generic::output(&state, &job, &plan.groups[0].id).is_ok());
        fs::write(&plan.groups[0].output_path, b"outside edit").unwrap();
        assert!(generic::output(&state, &job, &plan.groups[0].id).is_err());
    }
}

#[test]
fn changed_order_archives_owned_outputs_and_old_tasks_do_not_serve_new_bytes() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    f.pdf("2.pdf", 2);
    let state = f.state();
    let plan = generic::scan(f.request(0)).unwrap();
    let old = generate(&state, plan.clone(), 2);
    let original_hash = pdf::sha256_file(Path::new(&plan.groups[0].output_path)).unwrap();
    let group = &plan.groups[0];
    let revised = generic::reorder(
        &plan,
        OrderRequest {
            groups: vec![GroupOrder {
                group_id: group.id.clone(),
                file_ids: group.files.iter().rev().map(|f| f.id.clone()).collect(),
                output_name: None,
            }],
        },
    )
    .unwrap();
    let new = generate(&state, revised.clone(), 2);
    assert_eq!(new.status, "completed");
    assert_eq!(new.results[0].status, "completed");
    assert!(generic::output(&state, &old, &group.id).is_err());
    assert!(generic::output(&state, &new, &group.id).is_ok());
    let histories: Vec<_> = fs::read_dir(f.output.join(".history"))
        .unwrap()
        .map(|e| e.unwrap().path().join(&group.output_name))
        .collect();
    assert_eq!(histories.len(), 1);
    assert_eq!(pdf::sha256_file(&histories[0]).unwrap(), original_hash);
    let state2 = f.state();
    assert_eq!(
        state2.generic_plan(&revised.id).unwrap().groups[0].files[0].name,
        "2.pdf"
    );
    assert!(generic::report(&state2, &old).is_ok());
}

#[test]
fn foreign_modified_or_other_mode_output_is_never_overwritten() {
    for other_mode in [false, true] {
        let f = Fixture::new();
        f.pdf("1.pdf", 1);
        let state = f.state();
        let mut plan = generic::scan(f.request(0)).unwrap();
        fs::create_dir_all(&f.output).unwrap();
        let target = PathBuf::from(&plan.groups[0].output_path);
        fs::write(&target, b"foreign content").unwrap();
        if other_mode {
            let record = pdf::manifest_path(&state.dir, &target).unwrap();
            fs::create_dir_all(record.parent().unwrap()).unwrap();
            pdf::atomic_json(&record,&json!({"producer":"voucher-workbench","root":plan.root,"folder":"not-generic","output_sha256":pdf::sha256_file(&target).unwrap()})).unwrap();
        }
        generic::inspect_existing_outputs(&state.dir, &mut plan).unwrap();
        assert!(plan.groups[0]
            .issues
            .iter()
            .any(|i| i.code == "output_existing"));
        assert!(generic::build_group(
            &state.dir,
            &plan,
            &plan.groups[0],
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"foreign content");
        let group = &plan.groups[0];
        let renamed = generic::reorder(
            &plan,
            OrderRequest {
                groups: vec![GroupOrder {
                    group_id: group.id.clone(),
                    file_ids: group.files.iter().map(|f| f.id.clone()).collect(),
                    output_name: Some("new.pdf".into()),
                }],
            },
        )
        .unwrap();
        assert!(renamed.groups[0]
            .issues
            .iter()
            .all(|i| i.code != "output_existing"));
        assert_eq!(generate(&state, renamed, 1).status, "completed");
    }
}

#[test]
fn source_change_with_same_size_and_timestamp_is_rejected_and_failure_is_group_local() {
    let f = Fixture::new();
    f.pdf("A/1.pdf", 1);
    f.pdf("B/1.pdf", 1);
    let state = f.state();
    let plan = generic::scan(f.request(1)).unwrap();
    let source = f.root.join("A/1.pdf");
    let meta = fs::metadata(&source).unwrap();
    let mut bytes = fs::read(&source).unwrap();
    let position = bytes
        .windows(b"PAGE1".len())
        .position(|s| s == b"PAGE1")
        .unwrap();
    bytes[position] = b'X';
    fs::write(&source, bytes).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(FileTimes::new().set_modified(meta.modified().unwrap()))
        .unwrap();
    let job = generate(&state, plan, 2);
    assert_eq!(job.status, "completed_with_errors");
    let failure = job.results.iter().find(|r| r.status == "failed").unwrap();
    let error = failure.error.as_deref().unwrap();
    assert!(error.contains("请检查该文件并重新扫描") && error.contains("A/1.pdf"));
    assert_eq!(
        job.results.iter().filter(|r| r.status == "failed").count(),
        1
    );
    assert_eq!(
        job.results
            .iter()
            .filter(|r| r.status == "completed")
            .count(),
        1
    );
}

#[test]
fn bad_empty_encrypted_and_interactive_pdf_are_visible_and_block_their_group() {
    let f = Fixture::new();
    f.pdf("good/1.pdf", 1);
    fs::create_dir_all(f.root.join("bad")).unwrap();
    fs::write(f.root.join("bad/broken.pdf"), b"not pdf").unwrap();
    f.pdf("zero/zero.pdf", 0);
    f.pdf("forms/form.pdf", 1);
    f.pdf("encrypted/encrypted.pdf", 1);
    f.pdf("link/link.pdf", 1);
    let path = f.root.join("forms/form.pdf");
    let mut doc = Document::load(&path).unwrap();
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AcroForm", dictionary! {"Fields"=>vec![Object::Null]});
    doc.save(&path).unwrap();
    let path = f.root.join("encrypted/encrypted.pdf");
    let mut doc = Document::load(&path).unwrap();
    doc.trailer
        .set("Encrypt", dictionary! {"Filter"=>"Standard","V"=>1,"R"=>2});
    doc.save(&path).unwrap();
    let path = f.root.join("link/link.pdf");
    let mut doc = Document::load(&path).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let annot=doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Link","Dest"=>vec![Object::Reference(page),Object::Name(b"Fit".to_vec())]});
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(annot)]);
    doc.save(&path).unwrap();
    let plan = generic::scan(f.request(1)).unwrap();
    assert_eq!(plan.groups.len(), 6);
    assert_eq!(
        plan.groups
            .iter()
            .filter(|g| g.issues.iter().any(|i| i.severity == "error"))
            .count(),
        5
    );
    let state = f.state();
    let job = generate(&state, plan, 3);
    assert_eq!(job.status, "completed");
    assert_eq!(job.results.len(), 1);
}

#[test]
fn cancellation_cleans_partial_pdf_and_preserves_previous_owned_pdf() {
    let f = Fixture::new();
    f.pdf("1.pdf", 2);
    f.pdf("2.pdf", 2);
    let state = f.state();
    let plan = generic::scan(f.request(0)).unwrap();
    let job = generate(&state, plan.clone(), 1);
    assert_eq!(job.status, "completed");
    let original = pdf::sha256_file(Path::new(&plan.groups[0].output_path)).unwrap();
    let group = &plan.groups[0];
    let changed = generic::reorder(
        &plan,
        OrderRequest {
            groups: vec![GroupOrder {
                group_id: group.id.clone(),
                file_ids: group.files.iter().rev().map(|f| f.id.clone()).collect(),
                output_name: None,
            }],
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let result = generic::build_group(
        &state.dir,
        &changed,
        &changed.groups[0],
        &cancel,
        &mut |_| {
            cancel.store(true, Ordering::Release);
        },
    );
    assert!(result.is_err());
    assert_eq!(
        pdf::sha256_file(Path::new(&group.output_path)).unwrap(),
        original
    );
    assert!(!fs::read_dir(&f.output).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with("partial.pdf")));
}

#[cfg(unix)]
#[test]
fn symlink_inputs_and_outputs_are_never_followed() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.pdf("real/1.pdf", 1);
    symlink(f.root.join("real"), f.root.join("linked")).unwrap();
    symlink(f.root.join("real/1.pdf"), f.root.join("linked.pdf")).unwrap();
    let plan = generic::scan(f.request(0)).unwrap();
    assert_eq!(plan.groups[0].files.len(), 1);
    assert_eq!(
        plan.issues
            .iter()
            .filter(|i| i.code == "unsupported_path")
            .count(),
        2
    );
    let mut req = f.request(0);
    req.root = f.root.join("linked").to_str().unwrap().into();
    assert!(generic::scan(req).is_err());
    symlink(f.root.join("real"), &f.output).unwrap();
    assert!(generic::scan(f.request(0)).is_err());
}

#[tokio::test]
async fn api_stores_immutable_plan_and_rejects_unregistered_ids() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    let state = f.state();
    let router = generic::router().with_state(state.clone());
    let response = router
        .clone()
        .oneshot(
            Request::post("/scan")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&f.request(0)).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let plan: generic::Plan =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::post(format!("/plans/{}/generate", plan.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"group_ids":["../unregistered"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = router
        .clone()
        .oneshot(
            Request::get(format!("/plans/{}/files/not-a-file", plan.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/plans/{}/files/{}",
                plan.id, plan.groups[0].files[0].id
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/pdf");
    let response = router
        .oneshot(Request::get("/plans").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

#[test]
fn layers_output_intents_and_malformed_page_trees_block_only_their_groups() {
    let f = Fixture::new();
    f.pdf("good/1.pdf", 1);
    for kind in [
        "oc_catalog",
        "oc_object",
        "oc_membership",
        "output_intent",
        "uri_base",
        "af_catalog",
        "ef_object",
        "relative_uri",
        "missing_kid",
        "bad_count",
        "cycle",
        "duplicate",
        "direct_kid",
        "wrong_type",
    ] {
        let path = f.root.join(format!("{kind}/1.pdf"));
        static_pdf(&path, 1, kind);
        let mut doc = Document::load(&path).unwrap();
        let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let pages = doc
            .catalog()
            .unwrap()
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        match kind {
            "oc_catalog" => doc
                .get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("OCProperties", dictionary! {}),
            "oc_object" => {
                doc.add_object(
                    dictionary! {"Type"=>"OCG","Name"=>Object::string_literal("hidden")},
                );
            }
            "oc_membership" => {
                doc.add_object(dictionary! {"Type"=>"OCMD"});
            }
            "output_intent" => doc
                .get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("OutputIntents", vec![Object::Dictionary(dictionary! {})]),
            "uri_base" => doc
                .get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set(
                    "URI",
                    dictionary! {"Base"=>Object::string_literal("https://example.invalid/")},
                ),
            "af_catalog" => doc
                .get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("AF", vec![Object::Dictionary(dictionary! {})]),
            "ef_object" => {
                doc.add_object(
                    dictionary! {"Type"=>"Filespec","EF"=>dictionary! {"F"=>Object::Null}},
                );
            }
            "relative_uri" => {
                let annotation=doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Link","A"=>dictionary! {"S"=>"URI","URI"=>Object::string_literal("relative.pdf")}});
                doc.get_object_mut(page)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Annots", vec![Object::Reference(annotation)]);
            }
            "missing_kid" => {
                let missing = doc.new_object_id();
                let d = doc.get_object_mut(pages).unwrap().as_dict_mut().unwrap();
                d.set(
                    "Kids",
                    vec![Object::Reference(page), Object::Reference(missing)],
                );
                d.set("Count", 2);
            }
            "bad_count" => doc
                .get_object_mut(pages)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Count", 2),
            "cycle" => {
                let d = doc.get_object_mut(pages).unwrap().as_dict_mut().unwrap();
                d.set("Kids", vec![Object::Reference(pages)]);
            }
            "duplicate" => {
                let d = doc.get_object_mut(pages).unwrap().as_dict_mut().unwrap();
                d.set(
                    "Kids",
                    vec![Object::Reference(page), Object::Reference(page)],
                );
                d.set("Count", 2);
            }
            "direct_kid" => doc
                .get_object_mut(pages)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set(
                    "Kids",
                    vec![Object::Dictionary(
                        dictionary! {"Type"=>"Page","Parent"=>pages},
                    )],
                ),
            "wrong_type" => doc
                .get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Type", "SomethingElse"),
            _ => unreachable!(),
        }
        doc.save(&path).unwrap();
    }
    let plan = generic::scan(f.request(1)).unwrap();
    assert_eq!(plan.groups.len(), 15);
    assert!(plan
        .groups
        .iter()
        .filter(|g| g.name != "good")
        .all(|g| g.files[0].error.is_some() && g.issues.iter().any(|i| i.severity == "error")));
    assert!(plan
        .groups
        .iter()
        .find(|g| g.name == "missing_kid")
        .unwrap()
        .files[0]
        .error
        .as_deref()
        .unwrap()
        .contains("不存在"));
    let job = generate(&f.state(), plan, 3);
    assert_eq!(job.status, "completed");
    assert_eq!(job.results.len(), 1);
}

#[test]
fn static_page_annotations_survive_import() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    let source = f.root.join("1.pdf");
    let mut doc = Document::load(&source).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let note=doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Text","Rect"=>vec![0.into(),0.into(),10.into(),10.into()],"Contents"=>Object::string_literal("generated annotation")});
    let link=doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Link","Rect"=>vec![20.into(),0.into(),40.into(),10.into()],"A"=>dictionary! {"S"=>"URI","URI"=>Object::string_literal("https://example.invalid/")}});
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Annots",
            vec![Object::Reference(note), Object::Reference(link)],
        );
    doc.save(&source).unwrap();
    let state = f.state();
    let plan = generic::scan(f.request(0)).unwrap();
    let job = generate(&state, plan.clone(), 1);
    assert_eq!(job.status, "completed");
    let output =
        Document::load(generic::output(&state, &job, &plan.groups[0].id).unwrap()).unwrap();
    let page = *output.get_pages().values().next().unwrap();
    let annots = output
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(annots.len(), 2);
    let note = output
        .get_object(annots[0].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        note.get(b"Contents").unwrap().as_str().unwrap(),
        b"generated annotation"
    );
    let link = output
        .get_object(annots[1].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        link.get(b"A")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"URI")
            .unwrap()
            .as_str()
            .unwrap(),
        b"https://example.invalid/"
    );
}

#[test]
fn shared_job_gate_restart_and_retry_preserve_original_thread_snapshot() {
    let f = Fixture::new();
    f.pdf("A/1.pdf", 1);
    f.pdf("B/1.pdf", 1);
    let state = f.state();
    let plan = state
        .insert_generic_plan(generic::scan(f.request(1)).unwrap())
        .unwrap();
    let interrupted = Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: plan.id.clone(),
        kind: "generic_generate".into(),
        status: "running".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        total: 2,
        done: 0,
        current: "synthetic interrupted job".into(),
        error: None,
        results: vec![],
        request: GenerateRequest::default(),
        threads: 7,
    };
    state
        .data
        .lock()
        .unwrap()
        .jobs
        .insert(interrupted.id.clone(), interrupted.clone());
    pdf::atomic_json(
        &state
            .dir
            .join("jobs")
            .join(format!("{}.json", interrupted.id)),
        &interrupted,
    )
    .unwrap();
    assert!(state
        .start(&plan.id, "generic_generate", GenerateRequest::default())
        .unwrap_err()
        .to_string()
        .contains("已有任务"));
    assert!(state
        .start(
            "unneeded-voucher-plan",
            "generate",
            GenerateRequest::default()
        )
        .unwrap_err()
        .to_string()
        .contains("已有任务"));
    assert!(state
        .set_settings(ProcessingSettings { threads: 1 })
        .is_err());
    let loaded = f.state();
    assert_eq!(loaded.job(&interrupted.id).unwrap().status, "interrupted");
    loaded
        .set_settings(ProcessingSettings { threads: 1 })
        .unwrap();
    let retry = loaded.retry(&interrupted.id).unwrap();
    assert_eq!(retry.threads, 7);
    let completed = wait(&loaded, &retry.id);
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.results.len(), 2);
    for path in ["\\\\server\\share", "\\\\?\\UNC\\server\\share"] {
        let mut req = f.request(0);
        req.root = path.into();
        assert!(generic::scan(req).unwrap_err().to_string().contains("本地"));
    }
}

#[cfg(windows)]
#[test]
fn windows_drive_and_verbatim_roots_are_checked_only_when_complete() {
    let f = Fixture::new();
    f.pdf("1.pdf", 1);
    let verbatim = f.request(0);
    assert!(verbatim.root.starts_with(r"\\?\"));
    assert_eq!(generic::scan(verbatim.clone()).unwrap().groups[0].pages, 1);
    let mut ordinary = verbatim.clone();
    ordinary.root = ordinary.root.strip_prefix(r"\\?\").unwrap().into();
    ordinary.output_dir = ordinary
        .output_dir
        .map(|p| p.strip_prefix(r"\\?\").unwrap().into());
    assert_eq!(generic::scan(ordinary).unwrap().groups[0].pages, 1);
    let mut drive_relative = verbatim;
    drive_relative.root = "C:relative".into();
    assert!(generic::scan(drive_relative)
        .unwrap_err()
        .to_string()
        .contains("盘符路径必须包含完整根目录"));
}

#[cfg(windows)]
#[test]
fn windows_directory_junction_is_reported_without_following_it() {
    let f = Fixture::new();
    f.pdf("real/1.pdf", 1);
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(f.root.join("junction"))
        .arg(f.root.join("real"))
        .status()
        .unwrap();
    assert!(status.success());
    let plan = generic::scan(f.request(0)).unwrap();
    assert_eq!(plan.groups[0].files.len(), 1);
    assert!(plan
        .issues
        .iter()
        .any(|i| i.code == "unsupported_path" && i.path.as_deref() == Some("junction")));
    // TempDir cleanup must remove the junction rather than its target tree.
    fs::remove_dir(f.root.join("junction")).unwrap();
}
