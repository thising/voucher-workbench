//! Title-only orientation acceptance. Ordinary tests are synthetic; the ignored
//! test reads an explicitly configured local five-page fixture and keeps all
//! derived rotations inside a disposable temporary directory.
use voucher_workbench::{
    model::{Orientation, SourceFile},
    orientation::{self, Engine, MODEL_ID},
    pdf,
};

fn source(orientation: Option<Orientation>) -> SourceFile {
    SourceFile {
        id: "synthetic".into(),
        name: "synthetic.png".into(),
        relative_path: "item/voucher/synthetic.png".into(),
        sequence: 1,
        kind: "image".into(),
        pages: 1,
        bytes: 0,
        modified_ms: 0,
        sha256: String::new(),
        width: Some(100),
        height: Some(200),
        orientation,
        rotation_cw: 0,
        reviewed: false,
        error: None,
    }
}

#[test]
fn old_classification_records_remain_reviewable_after_schema_upgrade() {
    let old: Orientation=serde_json::from_str(r#"{"rotation_cw":0,"confidence":0.999,"scores":[0.999,0.001,0,0],"model":"old-document-classifier"}"#).unwrap();
    assert!(old.needs_review);
    assert!(old.title.is_none());
    let old_source = source(Some(old));
    assert!(pdf::needs_review(&old_source));
    assert!(pdf::needs_analysis(&old_source));

    // A previous title algorithm can have auto-accepted a page incorrectly.
    // Version migration must invalidate that acceptance, even with a high score.
    let mut previous_title = old_source;
    let decision = previous_title.orientation.as_mut().unwrap();
    decision.model = "title-ocr-v1.0.0".into();
    decision.needs_review = false;
    assert_ne!(decision.model, MODEL_ID);
    assert!(pdf::needs_review(&previous_title));
    assert!(pdf::needs_analysis(&previous_title));
    previous_title.reviewed = true;
    assert!(!pdf::needs_review(&previous_title));
    assert!(!pdf::needs_analysis(&previous_title));
}

#[test]
fn model_uncertainty_uses_title_decision_not_the_old_point_nine_cutoff() {
    let decision = Orientation {
        rotation_cw: 0,
        confidence: 0.86,
        scores: vec![],
        model: MODEL_ID.into(),
        needs_review: false,
        title: Some("合成标题".into()),
        reason: "synthetic accepted title".into(),
        candidates: vec![],
    };
    assert!(!pdf::needs_review(&source(Some(decision.clone()))));
    let mut unresolved = decision;
    unresolved.confidence = 0.999;
    unresolved.needs_review = true;
    unresolved.reason = "标题冲突".into();
    assert!(pdf::needs_review(&source(Some(unresolved))));
    let mut reviewed = source(None);
    reviewed.reviewed = true;
    assert!(!pdf::needs_review(&reviewed));
}

#[test]
#[ignore = "requires title OCR models, native ONNX runtime, and explicitly configured local five-page fixture"]
fn five_known_pages_and_all_quarter_turns_keep_the_correct_reading_direction() {
    use std::{fs, path::PathBuf};
    let fixture = PathBuf::from(
        std::env::var_os("VOUCHER_ORIENTATION_SAMPLE_DIR")
            .expect("set VOUCHER_ORIENTATION_SAMPLE_DIR to the five-page acceptance fixture"),
    );
    let model = std::env::var_os("VOUCHER_TITLE_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/title-ocr"));
    let mut engine = Engine::new(&model, None).unwrap();
    assert!(MODEL_ID.starts_with("title-ocr-v1"));
    let mut files = fs::read_dir(fixture)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|p| {
        p.file_stem()
            .unwrap()
            .to_string_lossy()
            .rsplit('_')
            .next()
            .unwrap()
            .parse::<u32>()
            .expect("fixture filenames end with an integer sequence")
    });
    assert_eq!(
        files.len(),
        5,
        "fixture must contain exactly five original JPEG pages"
    );
    let temporary = tempfile::tempdir().unwrap();
    for (page, (source, base_correction)) in files.iter().zip([0u16, 0, 0, 0, 270]).enumerate() {
        let image = orientation::load_image(source).unwrap();
        for applied_cw in [0u16, 90, 180, 270] {
            let path = if applied_cw == 0 {
                source.clone()
            } else {
                let rotated = match applied_cw {
                    90 => image.rotate90(),
                    180 => image.rotate180(),
                    270 => image.rotate270(),
                    _ => unreachable!(),
                };
                let path =
                    temporary
                        .path()
                        .join(format!("synthetic-{}-{}.png", page + 1, applied_cw));
                rotated.save(&path).unwrap();
                path
            };
            let result = engine.predict(&path).unwrap();
            let expected = (base_correction + 360 - applied_cw) % 360;
            eprintln!(
                "page={} source_rotation={} expected_correction={} actual={} review={} title={:?}",
                page + 1,
                applied_cw,
                expected,
                result.rotation_cw,
                result.needs_review,
                result.title
            );
            assert_eq!(
                result.rotation_cw,
                expected,
                "page {} rotated {} degrees",
                page + 1,
                applied_cw
            );
            assert!(
                !result.needs_review,
                "known clear-title fixture should be resolved: {}",
                result.reason
            );
            assert!(result.title.as_ref().is_some_and(|s| !s.is_empty()));
            assert!(result
                .candidates
                .iter()
                .any(|c| c.accepted && c.rotation_cw == expected && !c.keyword.is_empty()));
        }
    }
}
