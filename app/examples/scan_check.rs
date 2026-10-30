//! Read-only scanner diagnostic. No business paths or fixture data are bundled.
//! cargo run --example scan_check -- /path/to/batch [ledger.xls] [sheet]
use voucher_workbench::{ledger, model::ScanRequest, scan};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or_else(|| {
        anyhow::anyhow!("用法：scan_check <批次或分项路径> [明细账路径] [工作表]")
    })?;
    let started = std::time::Instant::now();
    let plan = scan::scan(&ScanRequest {
        root,
        ledger: args.next(),
        sheet: args.next(),
    })?;
    let matched = plan.vouchers.iter().filter(|v| v.subject.is_some()).count();
    let files: Vec<_> = plan.vouchers.iter().flat_map(|v| &v.files).collect();
    let errors = plan
        .issues
        .iter()
        .chain(plan.vouchers.iter().flat_map(|v| &v.issues))
        .filter(|i| i.severity == "error")
        .count();
    println!("selected_sheet={} vouchers={} matched={} images={} pdf_files={} pages={} errors={} ignored={} elapsed_seconds={:.2}", plan.sheet, plan.vouchers.len(), matched, files.iter().filter(|f| f.kind == "image").count(), files.iter().filter(|f| f.kind == "pdf").count(), files.iter().map(|f| f.pages as u64).sum::<u64>(), errors, plan.ignored, started.elapsed().as_secs_f64());
    for sheet in &plan.sheets {
        if sheet.valid {
            let book = ledger::read(std::path::Path::new(&plan.ledger_path), Some(&sheet.name))?;
            let coverage = plan
                .vouchers
                .iter()
                .filter(|v| {
                    book.entries
                        .get(&v.code)
                        .is_some_and(|entry| entry.subject.is_some())
                })
                .count();
            println!(
                "sheet={} coverage={}/{} header_row={}",
                sheet.name,
                coverage,
                plan.vouchers.len(),
                sheet.header_row.unwrap_or(0)
            );
        } else {
            println!("sheet={} invalid_header", sheet.name);
        }
    }
    if errors > 0 {
        anyhow::bail!("扫描有 {errors} 个错误，请在工作台中查看详细问题");
    }
    Ok(())
}
