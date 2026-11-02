//! Anonymous phase benchmark: pass a private fixture directory; no paths or
//! document contents are written to stdout.
use sha2::Digest;
use std::time::Instant;
use voucher_workbench::batch::{self, ScanRequest, Sort};
fn main() -> anyhow::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("provide fixture directory"))?;
    let started = Instant::now();
    let flags: Vec<_> = std::env::args().skip(2).collect();
    let request = ScanRequest {
        root,
        level: 0,
        exclude_rules: if flags.iter().any(|flag| flag == "--default-rules") {
            batch::DEFAULT_RULES.into()
        } else {
            String::new()
        },
        index: None,
        sort: Sort::default(),
        output_names: vec![],
    };
    if flags.iter().any(|flag| flag == "--metadata") {
        let prepared = batch::preview(request, None)?;
        println!(
            "{}",
            serde_json::json!({"profile":if cfg!(debug_assertions){"debug"}else{"release"},"mode":"metadata_only","files":prepared.file_count,"groups":prepared.summary.group_count,"bytes":prepared.summary.bytes,"wall_ms":started.elapsed().as_millis(),"contents_checked":false})
        );
        return Ok(());
    }
    let plan = batch::scan(request, None)?;
    let files: Vec<_> = plan.groups.iter().flat_map(|g| &g.files).collect();
    // Anonymous comparison with a prior plan: lexicographically sort the
    // [relative_path, sha256] pairs, encode compact UTF-8 JSON, then SHA256.
    let mut fingerprints: Vec<_> = files
        .iter()
        .map(|file| (&file.relative_path, &file.sha256))
        .collect();
    fingerprints.sort_by(|a, b| a.0.cmp(b.0));
    let source_digest = hex::encode(sha2::Sha256::digest(serde_json::to_vec(&fingerprints)?));
    println!(
        "{}",
        serde_json::json!({"profile":if cfg!(debug_assertions){"debug"}else{"release"},"files":files.len(),"bytes":files.iter().map(|f|f.bytes).sum::<u64>(),"pages":files.iter().map(|f|f.pages as u64).sum::<u64>(),"errors":files.iter().filter(|f|f.error.is_some()).count(),"wall_ms":started.elapsed().as_millis(),"metrics":plan.scan_metrics,"source_digest":source_digest})
    );
    Ok(())
}
