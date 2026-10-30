//! Local, read-only source evaluation for title-based orientation.
//! Writes resumable JSONL and an explicit review/sample manifest, never PDFs or
//! application state. No business filenames or paths are embedded in this tool.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    time::Instant,
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use voucher_workbench::{
    model::{Orientation, Plan, SourceFile},
    orientation::{Engine, MODEL_ID},
    pdf,
};

#[derive(Parser, Debug)]
#[command(about = "本地标题方向整批评估；不修改原件或工作台状态")]
struct Args {
    /// Existing plan JSON, usually a saved workbench plan.
    plan: PathBuf,
    /// Directory containing the pinned OCR models and character dictionary.
    model_dir: PathBuf,
    /// Resumable per-image result JSONL. The summary goes beside this file.
    output: PathBuf,
    #[arg(long)]
    runtime: Option<PathBuf>,
    /// Recompute all selected images; prior lines remain as a local audit trail.
    #[arg(long)]
    force: bool,
    /// Select one voucher folder for a pilot run.
    #[arg(long)]
    folder: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
    /// Independent OCR engines; keep this modest because each uses CPU threads.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=16))]
    workers: u16,
    /// Rebuild statistics from existing records, without loading models.
    #[arg(long)]
    summary_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    file_id: String,
    relative_path: String,
    sha256: String,
    orientation: Option<Orientation>,
    elapsed_ms: u128,
    error: Option<String>,
    #[serde(default)]
    plan_id: String,
    #[serde(default)]
    recorded_at: String,
}

/// Only a torn final line may be ignored. A malformed completed line is a real
/// corrupt journal, rather than a reason to silently redo an unknown subset.
fn load_records(path: &Path) -> Result<(HashMap<String, Record>, Option<u64>, bool)> {
    if !path.exists() {
        return Ok((HashMap::new(), None, false));
    }
    let bytes = fs::read(path)?;
    let mut records = HashMap::new();
    let mut offset = 0;
    let mut truncated_at = None;
    for chunk in bytes.split_inclusive(|b| *b == b'\n') {
        let complete = chunk.ends_with(b"\n");
        let content = if complete {
            &chunk[..chunk.len() - 1]
        } else {
            chunk
        };
        if content.iter().all(u8::is_ascii_whitespace) {
            offset += chunk.len();
            continue;
        }
        match serde_json::from_slice::<Record>(content) {
            Ok(record) => {
                records.insert(record.relative_path.clone(), record);
            }
            Err(_) if !complete => {
                truncated_at = Some(offset as u64);
                break;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("JSONL 在字节 {offset} 处损坏"));
            }
        }
        offset += chunk.len();
    }
    let need_newline = !bytes.is_empty() && !bytes.ends_with(b"\n") && truncated_at.is_none();
    Ok((records, truncated_at, need_newline))
}

fn reusable(record: &Record, file: &SourceFile) -> bool {
    record.sha256 == file.sha256
        && record.error.is_none()
        && record
            .orientation
            .as_ref()
            .is_some_and(|o| o.model == MODEL_ID)
}

fn sample_key(path: &str) -> String {
    // Stable pseudo-random ordering, independent of the classifier's confidence.
    hex::encode(Sha256::digest(format!("title-review-v1:{path}").as_bytes()))
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.output.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        bail!("评估输出必须使用 .jsonl 扩展名，避免误写其他文件");
    }
    let plan: Plan = serde_json::from_slice(&fs::read(&args.plan)?)?;
    if !MODEL_ID.starts_with("title-ocr-v1") {
        bail!("当前编译版本不是标题方向算法，拒绝用旧整页模型生成本次评估");
    }
    let mut selected: Vec<_> = plan
        .vouchers
        .iter()
        .filter(|v| args.folder.as_ref().is_none_or(|name| *name == v.folder))
        .flat_map(|v| {
            v.files
                .iter()
                .filter(|f| f.kind == "image")
                .map(move |f| (v, f))
        })
        .collect();
    if let Some(limit) = args.limit {
        selected.truncate(limit);
    }
    if selected.is_empty() {
        bail!("所选范围没有图片");
    }
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let (mut records, truncated_at, need_newline) = load_records(&args.output)?;
    if !args.summary_only {
        let started = Instant::now();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&args.output)?;
        if let Some(length) = truncated_at {
            file.set_len(length)?;
            eprintln!("恢复未完整写入的最后一行；已保留此前完成记录");
        }
        let mut writer = BufWriter::new(file);
        if need_newline {
            writeln!(writer)?;
        }
        let mut resumed = 0;
        let mut work = VecDeque::new();
        for (_, source) in &selected {
            // Check the real bytes before reusing a result. Plan fingerprints
            // alone cannot prove the source still has the same contents.
            let path = pdf::check_source(Path::new(&plan.root), source);
            if path.is_ok()
                && !args.force
                && records
                    .get(&source.relative_path)
                    .is_some_and(|r| reusable(r, source))
            {
                resumed += 1;
            } else {
                work.push_back(*source);
            }
        }
        let work_count = work.len();
        let worker_count = usize::from(args.workers).min(work_count);
        eprintln!(
            "准备处理 {} 张；已恢复 {} 张；OCR worker {}",
            work_count, resumed, worker_count
        );
        if work_count > 0 {
            let queue = Mutex::new(work);
            let stop = AtomicBool::new(false);
            // Workers never write to the journal. One receiver serializes each
            // complete record, so interruption cannot interleave JSON fragments.
            let (sender, receiver) = mpsc::channel::<Result<Record>>();
            std::thread::scope(|scope| -> Result<()> {
                let mut handles = Vec::new();
                for _ in 0..worker_count {
                    let sender = sender.clone();
                    let queue = &queue;
                    let stop = &stop;
                    let root = &plan.root;
                    let plan_id = &plan.id;
                    let model = &args.model_dir;
                    let runtime = args.runtime.as_deref();
                    handles.push(scope.spawn(move || {
                        let mut engine = match Engine::new(model, runtime) {
                            Ok(engine) => engine,
                            Err(error) => {
                                let _ = sender.send(Err(error.context("OCR worker 初始化失败")));
                                return;
                            }
                        };
                        loop {
                            if stop.load(Ordering::Relaxed) {
                                break;
                            }
                            let source = queue.lock().unwrap().pop_front();
                            let Some(source) = source else {
                                break;
                            };
                            let before = Instant::now();
                            let outcome =
                                pdf::check_source(Path::new(root), source).and_then(|path| {
                                    let orientation = engine.predict(&path)?;
                                    pdf::check_source(Path::new(root), source)?;
                                    Ok(orientation)
                                });
                            let (orientation, error) = match outcome {
                                Ok(value) => (Some(value), None),
                                Err(error) => (None, Some(format!("{error:#}"))),
                            };
                            let record = Record {
                                file_id: source.id.clone(),
                                relative_path: source.relative_path.clone(),
                                sha256: source.sha256.clone(),
                                orientation,
                                elapsed_ms: before.elapsed().as_millis(),
                                error,
                                plan_id: plan_id.clone(),
                                recorded_at: chrono::Utc::now().to_rfc3339(),
                            };
                            if sender.send(Ok(record)).is_err() {
                                break;
                            }
                        }
                    }));
                }
                drop(sender);
                let processing = (|| -> Result<()> {
                    let mut processed = 0;
                    for message in receiver {
                        let record = message?;
                        serde_json::to_writer(&mut writer, &record)?;
                        writeln!(writer)?;
                        writer.flush()?;
                        records.insert(record.relative_path.clone(), record);
                        processed += 1;
                        if processed % 50 == 0 || processed == work_count {
                            writer.get_ref().sync_data()?;
                            eprintln!(
                                "已处理 {}/{}；本次推理 {}；恢复 {}；worker {}；耗时 {:.1}s",
                                processed + resumed,
                                selected.len(),
                                processed,
                                resumed,
                                worker_count,
                                started.elapsed().as_secs_f32()
                            );
                        }
                    }
                    if processed != work_count {
                        bail!("OCR worker 提前退出：完成 {processed}/{work_count}；结果已保存，可续跑");
                    }
                    Ok(())
                })();
                if processing.is_err() {
                    stop.store(true, Ordering::Relaxed);
                }
                let mut panicked = false;
                for handle in handles {
                    if handle.join().is_err() {
                        panicked = true;
                    }
                }
                processing?;
                if panicked {
                    bail!("OCR worker 内部异常，结果已保存，可续跑");
                }
                Ok(())
            })?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;
    }

    let mut success = 0usize;
    let mut automatic = 0usize;
    let mut pending = 0usize;
    let mut failed = 0usize;
    let mut missing = 0usize;
    let mut old_review = 0usize;
    let mut old_unavailable = 0usize;
    let mut changed = 0usize;
    let mut released_from_old_review = 0usize;
    let mut newly_pending = 0usize;
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut rotations = BTreeMap::<u16, usize>::new();
    let mut automatic_rotations = BTreeMap::<u16, usize>::new();
    let mut keywords = BTreeMap::<String, usize>::new();
    let mut all_nonzero = Vec::new();
    let mut all_pending = Vec::new();
    let mut all_errors = Vec::new();
    let mut all_zero = Vec::new();
    let mut zero_strata = BTreeMap::<String, Vec<String>>::new();
    for (voucher, file) in &selected {
        let old_needs_review = match &file.orientation {
            Some(o) if !o.model.starts_with("title-ocr-v1") => {
                if o.confidence < 0.90 {
                    old_review += 1;
                }
                Some(o.confidence < 0.90)
            }
            _ => {
                old_unavailable += 1;
                None
            }
        };
        let Some(record) = records
            .get(&file.relative_path)
            .filter(|r| r.sha256 == file.sha256)
        else {
            missing += 1;
            continue;
        };
        if let Some(error) = &record.error {
            failed += 1;
            all_errors.push(json!({"relative_path":file.relative_path,"error":error}));
            continue;
        }
        let Some(orientation) = record.orientation.as_ref().filter(|o| o.model == MODEL_ID) else {
            missing += 1;
            continue;
        };
        success += 1;
        *rotations.entry(orientation.rotation_cw).or_default() += 1;
        let keyword = orientation
            .candidates
            .iter()
            .find(|c| c.accepted && c.rotation_cw == orientation.rotation_cw)
            .map(|c| c.keyword.clone())
            .unwrap_or_default();
        if !keyword.is_empty() {
            *keywords.entry(keyword.clone()).or_default() += 1;
        }
        let entry = json!({"file_id":file.id,"relative_path":file.relative_path,"rotation_cw":orientation.rotation_cw,"needs_review":orientation.needs_review,"title":orientation.title,"confidence":orientation.confidence,"reason":orientation.reason});
        if orientation.rotation_cw != 0 {
            all_nonzero.push(entry.clone());
        }
        if orientation.needs_review {
            pending += 1;
            *reasons.entry(orientation.reason.clone()).or_default() += 1;
            all_pending.push(entry);
            if old_needs_review == Some(false) {
                newly_pending += 1;
            }
        } else {
            automatic += 1;
            *automatic_rotations
                .entry(orientation.rotation_cw)
                .or_default() += 1;
            if old_needs_review == Some(true) {
                released_from_old_review += 1;
            }
            if orientation.rotation_cw == 0 {
                all_zero.push(file.relative_path.clone());
                let band = if orientation.confidence < 0.85 {
                    "<.85"
                } else if orientation.confidence < 0.95 {
                    ".85-.95"
                } else {
                    ">=.95"
                };
                let stratum = format!("{}|{}|{}", voucher.folder, keyword, band);
                zero_strata
                    .entry(stratum)
                    .or_default()
                    .push(file.relative_path.clone());
            }
        }
        if let Some(old) = &file.orientation {
            if !old.model.starts_with("title-ocr-v1") && old.rotation_cw != orientation.rotation_cw
            {
                changed += 1;
            }
        }
    }
    let mut zero_sample = BTreeSet::new();
    for values in zero_strata.values_mut() {
        values.sort_by_cached_key(|p| sample_key(p));
        zero_sample.extend(values.iter().take(2).cloned());
    }
    all_zero.sort_by_cached_key(|p| sample_key(p));
    zero_sample.extend(all_zero.iter().take(100).cloned());
    let denominator = selected.len() as f64;
    let summary = json!({
        "plan_id":plan.id, "algorithm":MODEL_ID, "created_at":chrono::Utc::now().to_rfc3339(),
        "counts":{"selected":selected.len(),"processed":success,"automatic":automatic,"pending_review":pending,"errors":failed,"missing":missing},
        "review_rate_all_selected": (pending+failed+missing) as f64 / denominator,
        "review_rate_processed": if success>0 {Some(pending as f64/success as f64)} else {None},
        "review_reasons":reasons,"rotation_counts":rotations,"automatic_rotation_counts":automatic_rotations,"title_keywords":keywords,
        "old_vs_new":{"old_review_count":old_review,"old_unavailable":old_unavailable,"rotation_changed":changed,"released_from_old_review":released_from_old_review,"newly_pending":newly_pending},
        "visual_review":{"all_nonzero":all_nonzero,"all_pending":all_pending,"errors":all_errors,"automatic_zero_stratified_sample":zero_sample,"automatic_zero_sample_rule":"2 per voucher|title keyword|OCR confidence band, plus first 100 stable SHA256-ordered zero-degree pages; union, no duplicates"},
        "accuracy_note":"这些统计仅描述算法结果，尚未标注正确率；visual_review是待独立查看的清单，不能将自动通过率当作准确率。"
    });
    let summary_path = args.output.with_extension("summary.json");
    pdf::atomic_json(&summary_path, &summary)?;
    println!(
        "{}",
        serde_json::to_string(
            &json!({"summary":summary_path,"counts":summary["counts"],"review_rate_all_selected":summary["review_rate_all_selected"],"rotation_counts":summary["rotation_counts"]})
        )?
    );
    if failed > 0 || missing > 0 {
        bail!("有 {failed} 个错误、{missing} 个缺失结果；可修复后续跑，统计已保存");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        Record {
            file_id: "synthetic".into(),
            relative_path: "item/voucher/1.png".into(),
            sha256: "abc".into(),
            orientation: None,
            elapsed_ms: 1,
            error: Some("synthetic error".into()),
            plan_id: "synthetic-plan".into(),
            recorded_at: "2026-01-01".into(),
        }
    }

    #[test]
    fn torn_last_line_keeps_prior_records_and_reports_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.jsonl");
        let first = format!("{}\n", serde_json::to_string(&record()).unwrap());
        fs::write(&path, format!("{first}{{\"file_id\":")).unwrap();
        let (records, truncate, newline) = load_records(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(truncate, Some(first.len() as u64));
        assert!(!newline);
        fs::write(&path, format!("bad complete line\n{first}")).unwrap();
        assert!(load_records(&path).is_err());
    }

    #[test]
    fn complete_last_record_without_newline_is_not_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.jsonl");
        fs::write(&path, serde_json::to_vec(&record()).unwrap()).unwrap();
        let (records, truncate, newline) = load_records(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(truncate, None);
        assert!(newline);
    }
}
