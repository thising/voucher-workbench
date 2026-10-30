//! Reapply the documented v1.0.1 -> v1.0.2 title-rule restriction to completed
//! OCR evidence. No model is loaded and no new OCR inference is represented.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use voucher_workbench::{
    model::{Orientation, Plan},
    orientation::{select_title_direction, MODEL_ID},
    pdf,
};

const SOURCE_MODEL: &str = "title-ocr-v1.0.1";
const TARGET_MODEL: &str = "title-ocr-v1.0.2";

#[derive(Debug, Parser)]
#[command(about = "只读复用完整 v1.0.1 OCR 候选，执行 v1.0.2 标题规则；不重新推理")]
struct Args {
    plan: PathBuf,
    input: PathBuf,
    /// A new JSONL file. Existing files are never overwritten or appended.
    output: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    file_id: String,
    relative_path: String,
    sha256: String,
    orientation: Option<Orientation>,
    elapsed_ms: u128,
    error: Option<String>,
    plan_id: String,
    recorded_at: String,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

fn parse_records(bytes: &[u8]) -> Result<HashMap<String, Record>> {
    let mut records = HashMap::new();
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = serde_json::from_slice(line).with_context(|| {
            format!("输入第 {} 行不是完整 JSON；请等待原始 OCR 完成", index + 1)
        })?;
        if !value
            .get("orientation")
            .and_then(|o| o.get("candidates"))
            .is_some_and(Value::is_array)
        {
            bail!("第 {} 行没有完整 OCR candidates，不能重判", index + 1);
        }
        let record: Record = serde_json::from_value(value)
            .with_context(|| format!("输入第 {} 行记录格式不完整", index + 1))?;
        let key = record.relative_path.clone();
        if records.insert(key.clone(), record).is_some() {
            bail!("输入有重复图片记录：{key}；请先选择唯一的完整推理批次");
        }
    }
    Ok(records)
}

fn reclassify(plan: &Plan, bytes: &[u8]) -> Result<Vec<Record>> {
    if MODEL_ID != TARGET_MODEL {
        bail!("只支持 {SOURCE_MODEL} -> {TARGET_MODEL}，当前编译规则为 {MODEL_ID}");
    }
    let mut records = parse_records(bytes)?;
    let images: Vec<_> = plan
        .vouchers
        .iter()
        .flat_map(|voucher| &voucher.files)
        .filter(|source| source.kind == "image")
        .collect();
    if images.is_empty() || records.len() != images.len() {
        bail!(
            "必须有全部图片的完整 OCR 结果：计划 {} 张，输入 {} 条",
            images.len(),
            records.len()
        );
    }
    let source_results_sha256 = hex::encode(Sha256::digest(bytes));
    let reclassified_at = chrono::Utc::now().to_rfc3339();
    let mut seen_ids = HashSet::new();
    let mut output = Vec::with_capacity(images.len());
    for source in images {
        if !seen_ids.insert(&source.id) {
            bail!("计划有重复图片 ID：{}", source.id);
        }
        let mut record = records
            .remove(&source.relative_path)
            .with_context(|| format!("图片缺少 OCR 结果：{}", source.relative_path))?;
        if record.file_id != source.id
            || record.plan_id != plan.id
            || record.sha256 != source.sha256
        {
            bail!(
                "记录 ID、计划 ID 或 SHA 与当前计划不符：{}",
                source.relative_path
            );
        }
        if record.error.is_some() || record.recorded_at.is_empty() {
            bail!("原始 OCR 有错误或缺少时间戳：{}", source.relative_path);
        }
        if record.extra.contains_key("reclassification") {
            bail!(
                "输入已带重判元数据，不是原始 OCR 批次：{}",
                source.relative_path
            );
        }
        let old = record.orientation.take().context("缺少原始方向结果")?;
        if old.model != SOURCE_MODEL {
            bail!(
                "输入必须全部为 {SOURCE_MODEL}：{} 是 {}",
                source.relative_path,
                old.model
            );
        }
        if old.candidates.iter().any(|candidate| {
            ![0, 90, 180, 270].contains(&candidate.rotation_cw)
                || !candidate.confidence.is_finite()
                || !(0.0..=1.0).contains(&candidate.confidence)
        }) {
            bail!("OCR 候选角度或置信度无效：{}", source.relative_path);
        }
        // Verifies path containment, bytes, mtime and the actual source SHA.
        pdf::check_source(Path::new(&plan.root), source)?;
        record.orientation = Some(select_title_direction(old.candidates));
        record.extra.insert(
            "reclassification".into(),
            serde_json::json!({
                "source_results_sha256": source_results_sha256,
                "source_model": SOURCE_MODEL,
                "target_model": TARGET_MODEL,
                "reclassified_at": reclassified_at,
                "method": "select_title_direction on preserved OCR candidates; v1.0.2 only restricts title keywords",
                "new_ocr_inference_performed": false,
                "ocr_elapsed_ms_and_recorded_at_preserved": true,
            }),
        );
        output.push(record);
    }
    if !records.is_empty() {
        bail!("输入包含计划之外的图片记录");
    }
    Ok(output)
}

fn run(args: &Args) -> Result<usize> {
    if args.output.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        bail!("输出必须是全新的 .jsonl 文件");
    }
    if args.output.exists() {
        bail!("输出已存在，拒绝覆盖或追加：{}", args.output.display());
    }
    let plan: Plan = serde_json::from_slice(&fs::read(&args.plan)?)?;
    let bytes = fs::read(&args.input)?;
    // Validate every record and source before creating anything at output.
    let records = reclassify(&plan, &bytes)?;
    let mut output_bytes = Vec::new();
    for record in &records {
        serde_json::to_writer(&mut output_bytes, record)?;
        output_bytes.push(b'\n');
    }
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)
        .with_context(|| format!("无法创建全新输出：{}", args.output.display()))?;
    if let Err(error) = file.write_all(&output_bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&args.output);
        return Err(error).context("写入重判结果失败；未完成输出已尝试移除");
    }
    Ok(records.len())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let count = run(&args)?;
    eprintln!(
        "规则重判完成：{count} 张，{SOURCE_MODEL} -> {TARGET_MODEL}；复用原始 OCR，未重新推理；{}",
        args.output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Args) {
        let dir = TempDir::new().unwrap();
        let image = dir.path().join("source.jpg");
        fs::write(&image, b"source fingerprint fixture").unwrap();
        let meta = fs::metadata(&image).unwrap();
        let hash = hex::encode(Sha256::digest(fs::read(&image).unwrap()));
        let plan = serde_json::json!({
            "id":"plan", "root":dir.path(), "ledger_path":"ledger.xls", "ledger_sha256":"unused",
            "sheet":"first", "sheets":[], "created_at":"2026-01-01", "issues":[], "ignored":0,
            "vouchers":[{"id":"voucher", "item":"item", "folder":"folder", "relative_path":"folder",
                "code":"00001", "subject":null, "ledger_row":null, "duplicate_rows":[],
                "output_relative_path":null, "issues":[], "files":[{
                    "id":"image", "name":"source.jpg", "relative_path":"source.jpg", "sequence":1,
                    "kind":"image", "pages":1, "bytes":meta.len(),
                    "modified_ms":meta.modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                    "sha256":hash, "width":null, "height":null, "orientation":null,
                    "rotation_cw":0, "reviewed":false, "error":null
                }]}]
        });
        let record = serde_json::json!({
            "file_id":"image", "relative_path":"source.jpg", "sha256":hash,
            "orientation":{
                "rotation_cw":180, "confidence":0.95, "scores":[], "model":SOURCE_MODEL,
                "needs_review":false, "title":"方记账凭证", "reason":"old decision",
                "candidates":[{"rotation_cw":180,"text":"方记账凭证","confidence":0.95,
                    "bbox":[10,10,200,30],"keyword":"记账凭证","accepted":true}]
            }, "elapsed_ms":4321, "error":null, "plan_id":"plan",
            "recorded_at":"2026-01-01T01:02:03Z"
        });
        let args = Args {
            plan: dir.path().join("plan.json"),
            input: dir.path().join("input.jsonl"),
            output: dir.path().join("output.jsonl"),
        };
        fs::write(&args.plan, serde_json::to_vec(&plan).unwrap()).unwrap();
        fs::write(&args.input, format!("{record}\n")).unwrap();
        (dir, args)
    }

    #[test]
    fn removes_side_caption_and_preserves_original_inference_provenance() {
        let (_dir, args) = fixture();
        let input = fs::read(&args.input).unwrap();
        assert_eq!(run(&args).unwrap(), 1);
        let result = parse_records(&fs::read(&args.output).unwrap()).unwrap();
        let record = &result["source.jpg"];
        let orientation = record.orientation.as_ref().unwrap();
        assert!(orientation.needs_review);
        assert_eq!(orientation.model, TARGET_MODEL);
        assert_eq!(record.elapsed_ms, 4321);
        assert_eq!(record.recorded_at, "2026-01-01T01:02:03Z");
        assert_eq!(
            record.extra["reclassification"]["new_ocr_inference_performed"],
            false
        );
        assert_eq!(
            record.extra["reclassification"]["source_results_sha256"],
            hex::encode(Sha256::digest(&input))
        );
        assert_eq!(fs::read(&args.input).unwrap(), input);
    }

    #[test]
    fn retains_a_valid_title_and_its_geometry_acceptance() {
        let (_dir, args) = fixture();
        let mut record: Value = serde_json::from_slice(&fs::read(&args.input).unwrap()).unwrap();
        record["orientation"]["candidates"][0]["text"] = "银行转账回单".into();
        record["orientation"]["candidates"][0]["rotation_cw"] = 270.into();
        fs::write(&args.input, serde_json::to_vec(&record).unwrap()).unwrap();
        run(&args).unwrap();
        let records = parse_records(&fs::read(&args.output).unwrap()).unwrap();
        let result = records["source.jpg"].orientation.as_ref().unwrap();
        assert!(!result.needs_review);
        assert_eq!(result.rotation_cw, 270);
        assert_eq!(result.candidates[0].bbox, [10, 10, 200, 30]);
    }

    #[test]
    fn refuses_incomplete_duplicate_failed_or_inconsistent_evidence_without_output() {
        for case in [
            "missing",
            "duplicate",
            "torn",
            "error",
            "model",
            "file_id",
            "plan_id",
            "sha",
            "candidates",
            "changed_source",
        ] {
            let (dir, args) = fixture();
            let bytes = fs::read(&args.input).unwrap();
            let mut row: Value = serde_json::from_slice(&bytes).unwrap();
            match case {
                "missing" => fs::write(&args.input, []).unwrap(),
                "duplicate" => {
                    fs::write(&args.input, [bytes.as_slice(), bytes.as_slice()].concat()).unwrap()
                }
                "torn" => fs::write(&args.input, b"{\"file_id\":").unwrap(),
                "changed_source" => {
                    fs::write(dir.path().join("source.jpg"), b"changed bytes").unwrap()
                }
                _ => {
                    match case {
                        "error" => row["error"] = "OCR failure".into(),
                        "model" => row["orientation"]["model"] = "title-ocr-v1.0.0".into(),
                        "file_id" => row["file_id"] = "other".into(),
                        "plan_id" => row["plan_id"] = "other".into(),
                        "sha" => row["sha256"] = "other".into(),
                        "candidates" => {
                            row["orientation"]
                                .as_object_mut()
                                .unwrap()
                                .remove("candidates");
                        }
                        _ => unreachable!(),
                    }
                    fs::write(&args.input, serde_json::to_vec(&row).unwrap()).unwrap();
                }
            }
            assert!(run(&args).is_err(), "case {case}");
            assert!(!args.output.exists(), "case {case}");
        }
    }

    #[test]
    fn never_overwrites_existing_output() {
        let (_dir, args) = fixture();
        fs::write(&args.output, b"preserve me").unwrap();
        assert!(run(&args).is_err());
        assert_eq!(fs::read(&args.output).unwrap(), b"preserve me");
    }
}
