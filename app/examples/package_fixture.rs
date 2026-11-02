//! Generated, non-business inputs and independent PDF checks for release smoke tests.
//! This development helper is built on the CI host and is never shipped in the ZIP.
use anyhow::{ensure, Result};
use clap::{Parser, Subcommand, ValueEnum};
use image::{Rgb, RgbImage};
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, ObjectId, Stream,
};
use serde_json::json;
use std::{fs, io::Write, path::Path};
use voucher_workbench::{model::ORIENTATION_DISCLAIMER_VERSION, pdf};

#[cfg(test)]
const OUTPUT: &str = "合成分项-PDF/260101-00001-合成科目.pdf";

#[derive(Parser)]
#[command(about = "Generate and verify synthetic release-smoke inputs; no business data")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Create {
        root: std::path::PathBuf,
    },
    Verify {
        root: std::path::PathBuf,
        #[arg(value_enum)]
        mode: Mode,
        #[arg(long)]
        state_dir: std::path::PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    Disabled,
    Enabled,
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Create { root } => create(&root),
        Command::Verify {
            root,
            mode,
            state_dir,
        } => verify(&root, mode, &state_dir),
    }
}

fn ledger(path: &Path) -> Result<()> {
    let mut archive = zip::ZipWriter::new(fs::File::create(path)?);
    let entries = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="合成首表" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>凭证编号</t></is></c><c r="B1" t="inlineStr"><is><t>科目名称</t></is></c><c r="C1" t="inlineStr"><is><t>无关字段</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>(c)00001</t></is></c><c r="B2" t="inlineStr"><is><t>合成科目</t></is></c><c r="C2" t="inlineStr"><is><t>不进入PDF</t></is></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>(c)00001</t></is></c><c r="B3" t="inlineStr"><is><t>重号后行不采用</t></is></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>(c)00002</t></is></c><c r="B4" t="inlineStr"><is><t>合成科目</t></is></c></row><row r="5"><c r="A5" t="inlineStr"><is><t>(c)00002</t></is></c><c r="B5" t="inlineStr"><is><t>第二凭证重号后行</t></is></c></row></sheetData></worksheet>"#,
        ),
    ];
    for (name, contents) in entries {
        archive.start_file(name, zip::write::SimpleFileOptions::default())?;
        archive.write_all(contents.as_bytes())?;
    }
    archive.finish()?;
    Ok(())
}

fn glyph(letter: char) -> [u8; 7] {
    match letter {
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'N' => [17, 25, 25, 21, 19, 19, 17],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        _ => [0; 7],
    }
}

fn synthetic_image(width: u32, height: u32) -> RgbImage {
    let mut image = RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    // Original bitmap lettering, deliberately no font dependency or external asset.
    let scale = 10;
    let text_width = 7 * 6 * scale;
    let offset_x = (width - text_width) / 2;
    for (letter_index, letter) in "INVOICE".chars().enumerate() {
        for (y, row) in glyph(letter).iter().enumerate() {
            for x in 0..5 {
                if row & (1 << (4 - x)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            image.put_pixel(
                                offset_x + letter_index as u32 * 6 * scale + x * scale + dx,
                                70 + y as u32 * scale + dy,
                                Rgb([15, 25, 35]),
                            );
                        }
                    }
                }
            }
        }
    }
    // A red arrow points to the heading; a blue strip marks the lower edge.
    for y in 195..315 {
        for x in width / 2 - 8..width / 2 + 8 {
            image.put_pixel(x, y, Rgb([190, 30, 35]));
        }
    }
    for y in 175..215 {
        let radius = (y - 175) / 2;
        for x in width / 2 - radius..=width / 2 + radius {
            image.put_pixel(x, y, Rgb([190, 30, 35]));
        }
    }
    for y in 360..height - 90 {
        for x in 70..width - 70 {
            if (y - 360) % 90 < 3 || (x - 70) % 180 < 3 {
                image.put_pixel(x, y, Rgb([90, 100, 110]));
            }
        }
    }
    for y in height - 45..height - 25 {
        for x in 40..width - 40 {
            image.put_pixel(x, y, Rgb([20, 70, 200]));
        }
    }
    image
}

fn native_pdf(path: &Path) -> Result<()> {
    let mut document = Document::with_version("1.7");
    let root = document.new_object_id();
    let font = document.add_object(
        dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"},
    );
    let mut pages = Vec::new();
    for (index, (width, height, rotation)) in [(321, 654, 0), (600, 400, 270)].iter().enumerate() {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 20.into()]),
                Operation::new("Td", vec![30.into(), (height - 50).into()]),
                Operation::new(
                    "Tj",
                    vec![Object::string_literal(format!(
                        "SYNTHETIC PAGE {}",
                        index + 1
                    ))],
                ),
                Operation::new("ET", vec![]),
            ],
        };
        let stream = document.add_object(Stream::new(Dictionary::new(), content.encode()?));
        let page = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => root,
            "MediaBox" => vec![0.into(), 0.into(), (*width).into(), (*height).into()],
            "Rotate" => *rotation,
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}},
            "Contents" => stream
        });
        pages.push(Object::Reference(page));
    }
    document.objects.insert(
        root,
        Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => pages, "Count" => 2}),
    );
    let catalog = document.add_object(dictionary! {"Type" => "Catalog", "Pages" => root});
    document.trailer.set("Root", catalog);
    document.save(path)?;
    Ok(())
}

fn create(root: &Path) -> Result<()> {
    ensure!(
        !root.exists(),
        "Fixture target already exists; refusing to overwrite"
    );
    fs::create_dir_all(root)?;
    ledger(&root.join("明细账.xlsx"))?;
    for code in 1..=2 {
        let directory = root.join(format!("合成分项/260101-{code:05}"));
        fs::create_dir_all(&directory)?;
        synthetic_image(1200, 900).save(directory.join(format!("2601_{code:05}_1.jpg")))?;
        image::imageops::rotate90(&synthetic_image(900, 1200))
            .save(directory.join(format!("2601_{code:05}_2.jpg")))?;
        native_pdf(&directory.join(format!("2601_{code:05}_3.pdf")))?;
        fs::write(directory.join("index.html"), "SYNTHETIC IGNORED HTML")?;
        fs::create_dir(directory.join("indexImg"))?;
        fs::write(
            directory.join("indexImg/not-an-image.jpg"),
            "SYNTHETIC IGNORED RESOURCE",
        )?;
    }
    println!(
        "{}",
        json!({"synthetic":true,"vouchers":2,"files":6,"pages":8})
    );
    Ok(())
}

fn page_box(document: &Document, id: ObjectId) -> Result<Vec<f32>> {
    let page = document.get_object(id)?.as_dict()?;
    let value = document.dereference(page.get(b"MediaBox")?)?.1;
    value
        .as_array()?
        .iter()
        .map(|value| Ok(value.as_float()?))
        .collect()
}

fn verify(root: &Path, mode: Mode, state_dir: &Path) -> Result<()> {
    for code in 1..=2 {
        verify_voucher(root, mode, state_dir, code)?;
    }
    println!(
        "{}",
        json!({"verified":true,"synthetic":true,"vouchers":2,"images":4,"native_pdf_pages":4,"pages":8,"enabled":mode == Mode::Enabled,"source_bytes_preserved":true})
    );
    Ok(())
}

fn verify_voucher(root: &Path, mode: Mode, state_dir: &Path, code: u32) -> Result<()> {
    let voucher = format!("合成分项/260101-{code:05}");
    let output = root.join(format!("合成分项-PDF/260101-{code:05}-合成科目.pdf"));
    let manifest: pdf::Manifest =
        serde_json::from_slice(&fs::read(pdf::manifest_path(state_dir, &output)?)?)?;
    fn check_output_tree(directory: &Path) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                check_output_tree(&path)?;
            } else {
                ensure!(
                    path.extension().is_some_and(|extension| extension == "pdf"),
                    "Output directory contains non-PDF bookkeeping: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }
    check_output_tree(output.parent().unwrap())?;
    ensure!(
        manifest.producer == "voucher-workbench",
        "Unexpected output producer"
    );
    ensure!(
        manifest.output_sha256 == pdf::sha256_file(&output)?,
        "Output hash mismatch"
    );
    ensure!(
        manifest.ledger_sha256 == pdf::sha256_file(&root.join("明细账.xlsx"))?,
        "Ledger changed"
    );
    ensure!(
        manifest.sheet == "合成首表" && manifest.subject.as_deref() == Some("合成科目"),
        "Ledger naming mismatch"
    );
    ensure!(
        manifest.ledger_row == Some(code * 2) && manifest.duplicate_rows == [code * 2 + 1],
        "First duplicate row not retained"
    );
    ensure!(
        manifest.sources.len() == 3 && manifest.pages == 4,
        "Source/page count mismatch"
    );
    ensure!(
        !manifest.needs_review,
        "Committed output unexpectedly needs review"
    );
    let enabled = mode == Mode::Enabled;
    ensure!(
        manifest.orientation_workflow.enabled == enabled,
        "Workflow snapshot mismatch"
    );
    if enabled {
        ensure!(
            manifest.orientation_workflow.has_current_consent()
                && manifest.orientation_workflow.analysis_completed,
            "Enabled workflow incomplete"
        );
        ensure!(
            manifest.orientation_workflow.consent_version.as_deref()
                == Some(ORIENTATION_DISCLAIMER_VERSION),
            "Wrong disclaimer version"
        );
    }
    let document = Document::load(&output)?;
    let pages = document.get_pages();
    ensure!(
        pages.len() == 4,
        "PDF has unexpected extra or missing pages"
    );
    for (index, mapping) in manifest.sources.iter().enumerate() {
        let suffix = if index < 2 { "jpg" } else { "pdf" };
        let expected = format!("{voucher}/2601_{code:05}_{}.{suffix}", index + 1);
        ensure!(mapping.source == expected, "Source order mismatch");
        ensure!(
            mapping.sha256 == pdf::sha256_file(&root.join(&expected))?,
            "Source fingerprint mismatch"
        );
        ensure!(
            mapping.first_page == index as u32 + 1
                && mapping.last_page == if index < 2 { index as u32 + 1 } else { 4 },
            "Page mapping mismatch"
        );
        let expected_angle = if enabled && index == 1 { 270 } else { 0 };
        ensure!(
            mapping.rotation_cw == expected_angle,
            "Unexpected effective rotation"
        );
        if index < 2 {
            ensure!(mapping.reviewed == enabled, "Manual confirmation mismatch");
            let page_id = pages[&(index as u32 + 1)];
            let page = document.get_object(page_id)?.as_dict()?;
            let image_ref = page
                .get(b"Resources")?
                .as_dict()?
                .get(b"XObject")?
                .as_dict()?
                .get(b"Scan")?;
            let image = document.dereference(image_ref)?.1.as_stream()?;
            ensure!(
                image.content == fs::read(root.join(&expected))?,
                "JPEG source bytes were changed"
            );
            let bounds = page_box(&document, page_id)?;
            let (width, height) = if expected_angle == 270 {
                (595.2756, 841.8898)
            } else {
                (841.8898, 595.2756)
            };
            ensure!(
                (bounds[2] - width).abs() < 0.01 && (bounds[3] - height).abs() < 0.01,
                "A4 orientation mismatch"
            );
            let content = Content::decode(&document.get_page_content(page_id)?)?;
            ensure!(
                content
                    .operations
                    .iter()
                    .map(|op| op.operator.as_str())
                    .collect::<Vec<_>>()
                    == ["q", "cm", "Do", "Q"],
                "Image page has unexpected overlay content"
            );
            let matrix = content.operations[1]
                .operands
                .iter()
                .map(|v| v.as_float())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if expected_angle == 270 {
                ensure!(
                    matrix[0] == 0. && matrix[1] > 0. && matrix[2] < 0. && matrix[3] == 0.,
                    "PDF image matrix is not 270 degrees"
                );
            } else {
                ensure!(
                    matrix[0] > 0. && matrix[1] == 0. && matrix[2] == 0. && matrix[3] > 0.,
                    "PDF image matrix is not upright"
                );
            }
        }
    }
    let original = Document::load(root.join(voucher).join(format!("2601_{code:05}_3.pdf")))?;
    for (number, id) in original.get_pages() {
        let target = pages[&(number + 2)];
        ensure!(
            page_box(&original, id)? == page_box(&document, target)?,
            "Native PDF dimensions changed"
        );
        let rotation = |doc: &Document, page: ObjectId| -> Result<i64> {
            Ok(doc
                .get_object(page)?
                .as_dict()?
                .get(b"Rotate")
                .ok()
                .map(|value| value.as_i64())
                .transpose()?
                .unwrap_or(0))
        };
        ensure!(
            rotation(&original, id)? == rotation(&document, target)?,
            "Native PDF rotation changed"
        );
        ensure!(
            original.get_page_content(id)? == document.get_page_content(target)?,
            "Native PDF content changed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use voucher_workbench::{
        model::{OrientationWorkflow, ScanRequest},
        scan,
    };

    #[test]
    fn generated_inputs_support_both_workflows_and_detect_output_tampering() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("中文 路径");
        let state_dir = temp.path().join("隔离 状态");
        create(&root)?;
        ensure!(
            create(&root).is_err(),
            "Fixture must refuse overwriting an existing directory"
        );
        let mut plan = scan::scan(&ScanRequest {
            root: root.to_string_lossy().into(),
            ledger: None,
            sheet: None,
        })?;
        ensure!(
            plan.vouchers.len() == 2 && plan.vouchers.iter().all(|v| v.files.len() == 3),
            "Unexpected scan result"
        );
        ensure!(plan.ignored == 4, "Ignored exports were not excluded");
        ensure!(
            !plan.issues.iter().any(|issue| issue.severity == "error"),
            "Synthetic batch invalid"
        );
        for voucher in &plan.vouchers {
            pdf::build_voucher(
                &state_dir,
                &plan,
                voucher,
                false,
                &AtomicBool::new(false),
                |_| {},
            )?;
        }
        verify(&root, Mode::Disabled, &state_dir)?;
        plan.orientation_workflow = OrientationWorkflow {
            enabled: true,
            consent_version: Some(ORIENTATION_DISCLAIMER_VERSION.into()),
            consented_at: Some("2026-01-01T00:00:00Z".into()),
            analysis_completed: true,
        };
        for file in plan
            .vouchers
            .iter_mut()
            .flat_map(|voucher| &mut voucher.files)
            .filter(|f| f.kind == "image")
        {
            file.reviewed = true;
            file.rotation_cw = if file.sequence == 2 { 270 } else { 0 };
        }
        for voucher in &plan.vouchers {
            pdf::build_voucher(
                &state_dir,
                &plan,
                voucher,
                false,
                &AtomicBool::new(false),
                |_| {},
            )?;
        }
        verify(&root, Mode::Enabled, &state_dir)?;
        fs::write(root.join(OUTPUT), b"tampered")?;
        ensure!(
            verify(&root, Mode::Enabled, &state_dir).is_err(),
            "Tampered PDF must fail verification"
        );
        Ok(())
    }
}
