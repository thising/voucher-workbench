//! Synthetic signature structures only; no real certificates or customer data.
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::{fs, path::Path, sync::atomic::AtomicBool};
use voucher_workbench::{batch, generic, pdf};

struct Signed {
    doc: Document,
    widget: ObjectId,
    field: ObjectId,
    appearance: ObjectId,
}

fn signed(inherited: bool, visible: bool, states: bool) -> Signed {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let content = doc.add_object(Stream::new(
        Dictionary::new(),
        b"0 0 1 rg 5 5 10 10 re f".to_vec(),
    ));
    let page = doc.add_object(dictionary! {"Type"=>"Page","Parent"=>pages,"Contents"=>content});
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,
            "MediaBox"=>vec![0.into(),0.into(),200.into(),240.into()],"Rotate"=>90,
            "Resources"=>dictionary! {}
        }),
    );
    let appearance = doc.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),80.into(),40.into()],
            "Matrix"=>vec![0.into(),1.into(),(-1).into(),0.into(),40.into(),0.into()],
            "Resources"=>dictionary! {}
        },
        b"1 0 0 rg 0 0 80 40 re f 0 1 0 rg 8 8 12 20 re f".to_vec(),
    ));
    let other_appearance = if states {
        Some(
            doc.add_object(Stream::new(
                doc.get_object(appearance)
                    .unwrap()
                    .as_stream()
                    .unwrap()
                    .dict
                    .clone(),
                b"0 0 1 rg 0 0 80 40 re f".to_vec(),
            )),
        )
    } else {
        None
    };
    let signature = doc.add_object(dictionary! {"Type"=>"Sig","ByteRange"=>vec![0.into(),10.into(),20.into(),30.into()],"Contents"=>Object::String(vec![48,3,1,2],lopdf::StringFormat::Hexadecimal)});
    let mut widget = dictionary! {"Type"=>"Annot","Subtype"=>"Widget","Rect"=>if visible {vec![20.into(),30.into(),100.into(),70.into()]} else {vec![0.into(),0.into(),0.into(),0.into()]},"F"=>4,"P"=>page};
    widget.set(
        "AP",
        if states {
            dictionary! {"N"=>dictionary! {"Signed"=>appearance,"Other"=>other_appearance.unwrap()}}
        } else {
            dictionary! {"N"=>appearance}
        },
    );
    if states {
        widget.set("AS", Object::Name(b"Signed".to_vec()));
    }
    let widget = doc.add_object(widget);
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(widget)]);
    let field = if inherited {
        let parent = doc.add_object(
            dictionary! {"FT"=>"Sig","V"=>signature,"Kids"=>vec![Object::Reference(widget)]},
        );
        doc.get_object_mut(widget)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Parent", parent);
        parent
    } else {
        let w = doc.get_object_mut(widget).unwrap().as_dict_mut().unwrap();
        w.set("FT", "Sig");
        w.set("V", signature);
        widget
    };
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages,"AcroForm"=>dictionary! {"Fields"=>vec![Object::Reference(field)],"SigFlags"=>3},"Perms"=>dictionary! {"DocMDP"=>signature},"DSS"=>dictionary! {"VRI"=>dictionary! {}}});
    doc.trailer.set("Root", root);
    Signed {
        doc,
        widget,
        field,
        appearance,
    }
}

fn encoded(doc: &mut Document) -> Vec<u8> {
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn merge(source: &Path, base: &Path, extra_rotation: bool) -> Document {
    fs::create_dir_all(base.join("state")).unwrap();
    let mut plan = batch::scan(
        batch::ScanRequest {
            root: source.parent().unwrap().to_str().unwrap().into(),
            level: 0,
            exclude_rules: String::new(),
            index: None,
            sort: Default::default(),
            output_names: vec![],
        },
        Some(&base.join("state")),
    )
    .unwrap();
    assert!(
        plan.groups[0].files[0].error.is_none(),
        "{:?}",
        plan.groups[0].files[0].error
    );
    assert!(plan.groups[0].files[0].pdf_has_signature);
    assert!(plan.groups[0]
        .issues
        .iter()
        .any(|issue| issue.code == "pdf_signature" && issue.severity == "warning"));
    if extra_rotation {
        let file_id = plan.groups[0].files[0].id.clone();
        plan = batch::edit(
            &plan,
            batch::EditRequest {
                files: vec![batch::FileEdit {
                    file_id,
                    rotation_cw: Some(90),
                    page_rotations: Some(vec![90]),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
    }
    let built = batch::build_group(
        &base.join("state"),
        &plan,
        &plan.groups[0],
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert!(built
        .manifest
        .sources
        .iter()
        .all(|source| source.pdf_has_signature));
    Document::load(built.path).unwrap()
}

fn assert_no_validation(doc: &Document) {
    assert!(!doc.catalog().unwrap().has(b"AcroForm"));
    assert!(!doc.catalog().unwrap().has(b"Perms"));
    assert!(!doc.catalog().unwrap().has(b"DSS"));
    for obj in doc.objects.values() {
        if let Object::Dictionary(d) = obj {
            assert!(!d.has(b"FT") && !d.has(b"ByteRange"));
            assert_ne!(
                d.get(b"Type").ok().and_then(|v| v.as_name().ok()),
                Some(b"Sig".as_slice())
            );
            assert_ne!(
                d.get(b"Subtype").ok().and_then(|v| v.as_name().ok()),
                Some(b"Widget".as_slice())
            );
        }
    }
}

#[test]
fn visible_signature_keeps_appearance_geometry_and_source_bytes() {
    for inherited in [false, true] {
        for states in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let dir = base.join("input");
            fs::create_dir(&dir).unwrap();
            let path = dir.join("synthetic.pdf");
            let mut f = signed(inherited, true, states);
            let bytes = encoded(&mut f.doc);
            fs::write(&path, &bytes).unwrap();
            let before = pdf::sha256_file(&path).unwrap();
            let info = generic::inspect_batch_pdf_bytes(&bytes).unwrap();
            assert_eq!(info.pages, 1);
            assert!(info.signature_notice);
            assert_eq!(generic::inspect_batch_pdf(&path).unwrap(), info);
            let original = Document::load_mem(&bytes).unwrap();
            let original_ap = original
                .get_object(f.appearance)
                .unwrap()
                .as_stream()
                .unwrap();
            let output = merge(&path, &base, true);
            assert_no_validation(&output);
            let page = output
                .get_object(*output.get_pages().values().next().unwrap())
                .unwrap()
                .as_dict()
                .unwrap();
            assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), 270);
            let annots = output
                .dereference(page.get(b"Annots").unwrap())
                .unwrap()
                .1
                .as_array()
                .unwrap();
            assert_eq!(annots.len(), 1);
            let stamp = output.dereference(&annots[0]).unwrap().1.as_dict().unwrap();
            assert_eq!(stamp.get(b"Subtype").unwrap().as_name().unwrap(), b"Stamp");
            assert_eq!(stamp.get(b"F").unwrap().as_i64().unwrap(), 4);
            assert_eq!(
                stamp.get(b"Rect").unwrap(),
                original
                    .get_object(f.widget)
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"Rect")
                    .unwrap()
            );
            let ap = output
                .dereference(stamp.get(b"AP").unwrap())
                .unwrap()
                .1
                .as_dict()
                .unwrap();
            let stream = output
                .dereference(ap.get(b"N").unwrap())
                .unwrap()
                .1
                .as_stream()
                .unwrap();
            assert_eq!(stream.content, original_ap.content);
            assert_eq!(
                stream.dict.get(b"Matrix").unwrap(),
                original_ap.dict.get(b"Matrix").unwrap()
            );
            assert_eq!(
                stream.dict.get(b"BBox").unwrap(),
                original_ap.dict.get(b"BBox").unwrap()
            );
            assert_eq!(pdf::sha256_file(&path).unwrap(), before);
        }
    }
}

#[test]
fn invisible_or_hidden_signature_never_invents_a_stamp() {
    for hidden in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let dir = base.join("input");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("synthetic.pdf");
        let mut f = signed(true, hidden, false);
        let w = f
            .doc
            .get_object_mut(f.widget)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        w.remove(b"AP");
        if hidden {
            w.set("F", 2);
        }
        fs::write(&path, encoded(&mut f.doc)).unwrap();
        let before = pdf::sha256_file(&path).unwrap();
        assert!(generic::inspect_batch_pdf(&path).unwrap().signature_notice);
        let output = merge(&path, &base, false);
        assert_no_validation(&output);
        let page = output
            .get_object(*output.get_pages().values().next().unwrap())
            .unwrap()
            .as_dict()
            .unwrap();
        assert!(output
            .dereference(page.get(b"Annots").unwrap())
            .unwrap()
            .1
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(pdf::sha256_file(&path).unwrap(), before);
    }
}

#[test]
fn visible_signature_requires_unambiguous_nondegenerate_cached_appearance() {
    for broken in ["missing", "state", "bbox", "matrix", "needs_appearance"] {
        let mut f = signed(true, true, broken == "state");
        match broken {
            "missing" => {
                f.doc
                    .get_object_mut(f.widget)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .remove(b"AP");
            }
            "state" => {
                f.doc
                    .get_object_mut(f.widget)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .remove(b"AS");
            }
            "bbox" => f
                .doc
                .get_object_mut(f.appearance)
                .unwrap()
                .as_stream_mut()
                .unwrap()
                .dict
                .set("BBox", vec![0.into(), 0.into(), 0.into(), 0.into()]),
            "matrix" => f
                .doc
                .get_object_mut(f.appearance)
                .unwrap()
                .as_stream_mut()
                .unwrap()
                .dict
                .set(
                    "Matrix",
                    vec![1.into(), 1.into(), 1.into(), 1.into(), 0.into(), 0.into()],
                ),
            "needs_appearance" => {
                let root = f.doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
                f.doc
                    .get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .get_mut(b"AcroForm")
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("NeedAppearances", true);
            }
            _ => unreachable!(),
        }
        assert!(
            generic::inspect_batch_pdf_bytes(&encoded(&mut f.doc)).is_err(),
            "{broken}"
        );
    }
}

#[test]
fn non_signature_forms_scripts_encryption_and_malformed_trees_stay_blocked() {
    for unsupported in [
        "text_field",
        "field_action",
        "js",
        "encrypted",
        "cycle",
        "orphan_widget",
    ] {
        let mut f = signed(true, true, false);
        match unsupported {
            "text_field" => f
                .doc
                .get_object_mut(f.field)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("FT", "Tx"),
            "field_action" => f
                .doc
                .get_object_mut(f.field)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("AA", dictionary! {}),
            "js" => {
                f.doc.add_object(
                    dictionary! {"S"=>"JavaScript","JS"=>Object::string_literal("synthetic")},
                );
            }
            "encrypted" => f
                .doc
                .trailer
                .set("Encrypt", dictionary! {"Filter"=>"Standard","V"=>1,"R"=>2}),
            "cycle" => f
                .doc
                .get_object_mut(f.field)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Kids", vec![Object::Reference(f.field)]),
            "orphan_widget" => {
                let root = f.doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
                f.doc
                    .get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .remove(b"AcroForm");
            }
            _ => unreachable!(),
        }
        assert!(
            generic::inspect_batch_pdf_bytes(&encoded(&mut f.doc)).is_err(),
            "{unsupported}"
        );
    }
}

#[test]
fn legacy_generic_does_not_adopt_the_batch_signature_policy() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().canonicalize().unwrap().join("input");
    fs::create_dir(&dir).unwrap();
    let mut f = signed(false, true, false);
    fs::write(dir.join("synthetic.pdf"), encoded(&mut f.doc)).unwrap();
    let plan = generic::scan(generic::ScanRequest {
        root: dir.to_str().unwrap().into(),
        output_dir: None,
        level: 0,
        exclude_paths: vec![],
        exclude_globs: vec![],
        sort: "natural_asc".into(),
    })
    .unwrap();
    assert!(plan.groups[0].files[0]
        .error
        .as_ref()
        .unwrap()
        .contains("数字签名"));
}

#[test]
fn old_signature_error_plan_requires_rescan_and_is_not_silently_rewritten() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let input = base.join("input");
    let state = base.join("state");
    fs::create_dir(&input).unwrap();
    fs::create_dir(&state).unwrap();
    let mut fixture = signed(true, true, false);
    fs::write(input.join("synthetic.pdf"), encoded(&mut fixture.doc)).unwrap();
    let request = batch::ScanRequest {
        root: input.to_str().unwrap().into(),
        level: 0,
        exclude_rules: String::new(),
        index: None,
        sort: Default::default(),
        output_names: vec![],
    };
    let fresh = batch::scan(request.clone(), Some(&state)).unwrap();
    let mut legacy = fresh.clone();
    let error = "legacy signature support error; rescan required".to_string();
    legacy.groups[0].files[0].error = Some(error.clone());
    legacy.groups[0].files[0].pdf_has_signature = false;
    let edited = batch::edit(&legacy, batch::EditRequest::default()).unwrap();
    assert_eq!(
        legacy.groups[0].files[0].error.as_deref(),
        Some(error.as_str())
    );
    assert_eq!(
        edited.groups[0].files[0].error,
        legacy.groups[0].files[0].error
    );
    assert!(!edited.groups[0].files[0].pdf_has_signature);
    assert!(batch::build_group(
        &state,
        &edited,
        &edited.groups[0],
        &AtomicBool::new(false),
        &mut |_| {}
    )
    .is_err());
    let rescanned = batch::scan(request, Some(&state)).unwrap();
    assert!(rescanned.groups[0].files[0].error.is_none());
    assert!(rescanned.groups[0].files[0].pdf_has_signature);
    // An old successful snapshot without the new notice must not bypass the
    // explicit rescan requirement by simply clearing an old error.
    let mut missing_notice = fresh;
    missing_notice.groups[0].files[0].pdf_has_signature = false;
    let error = batch::build_group(
        &state,
        &missing_notice,
        &missing_notice.groups[0],
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("重新扫描"));
}
