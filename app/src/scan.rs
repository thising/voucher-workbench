use crate::{
    ledger::{self, Ledger},
    model::{Issue, OrientationWorkflow, Plan, ScanRequest, SourceFile, Voucher},
};
use anyhow::{bail, Context, Result};
use chrono::{NaiveDate, Utc};
use regex::Regex;
use std::{
    collections::BTreeMap,
    fs,
    io::{BufReader, Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::UNIX_EPOCH,
};
use uuid::Uuid;

fn error(message: impl Into<String>) -> Issue {
    Issue {
        severity: "error".into(),
        message: message.into(),
    }
}
fn warning(message: impl Into<String>) -> Issue {
    Issue {
        severity: "warning".into(),
        message: message.into(),
    }
}

fn folder_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^([0-9]{6})-([0-9]+)$").unwrap())
}
fn file_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^([0-9]{4})_([0-9]+)_([0-9]+)$").unwrap())
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn ignored(path: &Path, is_dir: bool) -> bool {
    let name = name(path);
    (is_dir && name == "indexImg")
        || (!is_dir && name == "index.html")
        || matches!(name.as_str(), ".DS_Store" | "Thumbs.db" | "desktop.ini")
        || name.starts_with("._")
}

fn is_ledger(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("xls") || s.eq_ignore_ascii_case("xlsx"))
}

fn entries(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("无法读取目录 {}", path.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

/// Relative paths use '/' in persisted plans on every platform.
fn relative(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).context("文件路径超出资料目录")?;
    Ok(relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn parse_folder(folder: &str) -> Result<(String, String)> {
    let captures = folder_regex()
        .captures(folder)
        .context("凭证文件夹名称应为 YYMMDD-凭证编号，例如 230501-04067")?;
    let date = &captures[1];
    let year = 2000 + date[0..2].parse::<i32>()?;
    let month = date[2..4].parse::<u32>()?;
    let day = date[4..6].parse::<u32>()?;
    if NaiveDate::from_ymd_opt(year, month, day).is_none() {
        bail!("文件夹首段不是有效日期（YYMMDD）");
    }
    Ok((date.to_owned(), captures[2].to_owned()))
}

fn safe_subject(subject: &str) -> String {
    subject
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_end_matches([' ', '.'])
        .to_owned()
}

fn find_ledger(request: &ScanRequest, selected_root: &Path, batch_root: &Path) -> Result<PathBuf> {
    if let Some(value) = request
        .ledger
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let path = PathBuf::from(value);
        let path = if path.is_absolute() {
            path
        } else {
            selected_root.join(path)
        };
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("明细账不能是符号链接");
        }
        let path = path.canonicalize().context("指定明细账不存在")?;
        if !path.is_file() || !is_ledger(&path) {
            bail!("请选择 XLS 或 XLSX 明细账文件");
        }
        return Ok(path);
    }
    let mut candidates = Vec::new();
    for directory in [selected_root, batch_root] {
        if directory == batch_root && directory == selected_root && !candidates.is_empty() {
            break;
        }
        for entry in entries(directory)? {
            let file_type = entry.file_type()?;
            if file_type.is_file()
                && is_ledger(&entry.path())
                && !name(&entry.path()).starts_with("~$")
            {
                candidates.push(entry.path());
            }
        }
        if !candidates.is_empty() {
            break;
        }
    }
    // Prefer the explicitly named ledger, then deterministic filename order.
    candidates.sort_by_key(|path| {
        (
            path.file_stem().and_then(|s| s.to_str()) != Some("明细账"),
            name(path),
        )
    });
    candidates
        .into_iter()
        .next()
        .context("未找到 XLS/XLSX 明细账，请指定文件路径")
}

fn modified_ms(metadata: &fs::Metadata) -> Result<u64> {
    Ok(metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX))
}

fn add_file_error(file: &mut SourceFile, message: impl Into<String>) {
    let message = message.into();
    file.error = Some(match file.error.take() {
        Some(previous) => format!("{previous}；{message}"),
        None => message,
    });
}

fn tiff_has_multiple_pages(path: &Path) -> Result<bool> {
    let file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    tiff_reader_has_multiple_pages(file, length)
}

fn tiff_reader_has_multiple_pages(mut file: impl Read + Seek, length: u64) -> Result<bool> {
    let mut header = [0u8; 8];
    file.read_exact(&mut header).context("TIFF 文件头不完整")?;
    let little = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => bail!("TIFF 字节顺序无效"),
    };
    let u16_value = |bytes: [u8; 2]| {
        if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    let u32_value = |bytes: [u8; 4]| {
        if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };
    let u64_value = |bytes: [u8; 8]| {
        if little {
            u64::from_le_bytes(bytes)
        } else {
            u64::from_be_bytes(bytes)
        }
    };
    let (offset, count_width, entry_width, pointer_width) = match u16_value([header[2], header[3]])
    {
        42 => (
            u32_value(header[4..8].try_into().unwrap()) as u64,
            2u64,
            12u64,
            4u64,
        ),
        43 => {
            if u16_value([header[4], header[5]]) != 8 || header[6..8] != [0, 0] {
                bail!("BigTIFF 文件头无效");
            }
            let mut bytes = [0; 8];
            file.read_exact(&mut bytes)?;
            (u64_value(bytes), 8u64, 20u64, 8u64)
        }
        _ => bail!("TIFF 格式标识无效"),
    };
    if offset == 0
        || offset
            .checked_add(count_width)
            .is_none_or(|end| end > length)
    {
        bail!("TIFF 首个页面目录越界");
    }
    file.seek(SeekFrom::Start(offset))?;
    let count = if count_width == 2 {
        let mut bytes = [0; 2];
        file.read_exact(&mut bytes)?;
        u16_value(bytes) as u64
    } else {
        let mut bytes = [0; 8];
        file.read_exact(&mut bytes)?;
        u64_value(bytes)
    };
    let next_offset = count
        .checked_mul(entry_width)
        .and_then(|bytes| bytes.checked_add(offset + count_width))
        .context("TIFF 页面目录长度溢出")?;
    if next_offset
        .checked_add(pointer_width)
        .is_none_or(|end| end > length)
    {
        bail!("TIFF 页面目录不完整");
    }
    file.seek(SeekFrom::Start(next_offset))?;
    let next = if pointer_width == 4 {
        let mut bytes = [0; 4];
        file.read_exact(&mut bytes)?;
        u32_value(bytes) as u64
    } else {
        let mut bytes = [0; 8];
        file.read_exact(&mut bytes)?;
        u64_value(bytes)
    };
    Ok(next != 0)
}

pub(crate) fn reject_multiframe(path: &Path, format: Option<image::ImageFormat>) -> Result<()> {
    let multiple = match format {
        Some(image::ImageFormat::Png) => {
            image::codecs::png::PngDecoder::new(BufReader::new(fs::File::open(path)?))?.is_apng()?
        }
        Some(image::ImageFormat::WebP) => {
            image::codecs::webp::WebPDecoder::new(BufReader::new(fs::File::open(path)?))?
                .has_animation()
        }
        Some(image::ImageFormat::Tiff) => tiff_has_multiple_pages(path)?,
        _ => false,
    };
    if multiple {
        bail!("多帧/多页图片不能只导入首帧，请先拆为单页图片或转换为 PDF");
    }
    Ok(())
}

/// Batch validation examines the same bytes used for SHA256 and decoding.
pub(crate) fn reject_multiframe_bytes(
    bytes: &[u8],
    format: Option<image::ImageFormat>,
) -> Result<()> {
    let multiple = match format {
        Some(image::ImageFormat::Png) => {
            image::codecs::png::PngDecoder::new(Cursor::new(bytes))?.is_apng()?
        }
        Some(image::ImageFormat::WebP) => {
            image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))?.has_animation()
        }
        Some(image::ImageFormat::Tiff) => {
            tiff_reader_has_multiple_pages(Cursor::new(bytes), bytes.len() as u64)?
        }
        _ => false,
    };
    if multiple {
        bail!("多帧/多页图片不能只导入首帧，请先拆为单页图片或转换为 PDF");
    }
    Ok(())
}

fn scan_file(path: &Path, root: &Path, date: &str, code: &str) -> Result<SourceFile> {
    let metadata = fs::symlink_metadata(path)?;
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = if extension == "pdf" { "pdf" } else { "image" };
    let mut file = SourceFile {
        id: Uuid::new_v4().to_string(),
        name: name(path),
        relative_path: relative(root, path)?,
        sequence: 0,
        kind: kind.into(),
        pages: 0,
        bytes: metadata.len(),
        modified_ms: modified_ms(&metadata)?,
        sha256: crate::pdf::sha256_file(path).context("无法计算源文件校验值")?,
        width: None,
        height: None,
        orientation: None,
        rotation_cw: 0,
        reviewed: false,
        error: None,
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if let Some(captures) = file_regex().captures(stem) {
        file.sequence = captures[3].parse::<u32>().unwrap_or(0);
        if file.sequence == 0 {
            add_file_error(&mut file, "末尾序号必须为大于零的32位整数");
        }
        if !date.is_empty() && captures[1] != date[..4] {
            add_file_error(&mut file, "文件名前四位年月与凭证文件夹不一致");
        }
        if !code.is_empty() && captures[2] != *code {
            add_file_error(&mut file, "文件名中的凭证编号与文件夹不一致");
        }
    } else {
        add_file_error(&mut file, "文件名应为 YYMM_凭证编号_数字序号.扩展名");
    }
    if metadata.len() == 0 {
        add_file_error(&mut file, "文件为空（0字节）");
    } else if kind == "pdf" {
        match lopdf::Document::load(path) {
            Ok(document) => {
                if document.is_encrypted() {
                    add_file_error(&mut file, "加密 PDF 不支持合并，请先解密");
                }
                file.pages = document.get_pages().len().try_into().unwrap_or(u32::MAX);
                if file.pages == 0 {
                    add_file_error(&mut file, "PDF 没有可读取页面");
                }
            }
            Err(error) => add_file_error(&mut file, format!("PDF 无法读取：{error}")),
        }
    } else {
        let result = (|| -> Result<image::DynamicImage> {
            let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
            reject_multiframe(path, reader.format())?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(30_000);
            limits.max_image_height = Some(30_000);
            limits.max_alloc = Some(512 * 1024 * 1024);
            reader.limits(limits);
            Ok(reader.decode()?)
        })();
        match result {
            Ok(image) => {
                file.width = Some(image.width());
                file.height = Some(image.height());
                file.pages = 1;
            }
            Err(error) => add_file_error(&mut file, format!("图片无法解码：{error:#}")),
        }
    }
    let after = fs::symlink_metadata(path)?;
    if after.file_type().is_symlink()
        || after.len() != file.bytes
        || modified_ms(&after)? != file.modified_ms
    {
        add_file_error(&mut file, "扫描期间源文件发生变化，请重新扫描");
    }
    Ok(file)
}

fn scan_voucher(
    path: &Path,
    root: &Path,
    item: &str,
    ledger: &Ledger,
    ignored_count: &mut usize,
) -> Result<Voucher> {
    let folder = name(path);
    let mut voucher = Voucher {
        id: Uuid::new_v4().to_string(),
        item: item.into(),
        folder: folder.clone(),
        relative_path: relative(root, path)?,
        code: String::new(),
        subject: None,
        ledger_row: None,
        duplicate_rows: Vec::new(),
        output_relative_path: None,
        files: Vec::new(),
        issues: Vec::new(),
    };
    let mut date = String::new();
    match parse_folder(&folder) {
        Ok((parsed_date, code)) => {
            date = parsed_date;
            voucher.code = code;
        }
        Err(error) => voucher.issues.push(error_issue(error)),
    }
    if let Some(entry) = ledger.entries.get(&voucher.code) {
        voucher.subject = entry.subject.clone();
        voucher.ledger_row = Some(entry.row);
        voucher.duplicate_rows = entry.duplicate_rows.clone();
        if !entry.duplicate_rows.is_empty() {
            voucher.issues.push(warning(format!(
                "凭证编号重复，按所选工作表第 {} 行的首个科目命名，另有 {} 行重号",
                entry.row,
                entry.duplicate_rows.len()
            )));
        }
        if let Some(subject) = entry.subject.as_ref() {
            let safe = safe_subject(subject);
            if safe.is_empty() {
                voucher
                    .issues
                    .push(error("科目名称规范化后为空，无法生成文件名"));
            } else {
                let filename = format!("{folder}-{safe}.pdf");
                if filename.encode_utf16().count() > 255 {
                    voucher.issues.push(error(
                        "输出文件名超过 Windows 单文件名长度限制（255个UTF-16单元）",
                    ));
                } else {
                    voucher.output_relative_path = Some(format!("{item}-PDF/{filename}"));
                }
                if safe != *subject {
                    voucher.issues.push(warning(
                        "科目名称中的 Windows 非法文件名字符已替换为下划线，末尾空格/句点已移除",
                    ));
                }
            }
        } else {
            voucher.issues.push(error(format!(
                "第 {} 行的首匹配科目为空，不能取后续重号行代替",
                entry.row
            )));
        }
    } else if !voucher.code.is_empty() {
        voucher.issues.push(error(format!(
            "所选工作表“{}”中没有匹配的凭证编号 {}",
            ledger.sheet, voucher.code
        )));
    }
    for entry in entries(path)? {
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        if ignored(&entry_path, file_type.is_dir()) {
            *ignored_count += 1;
            continue;
        }
        if file_type.is_symlink() {
            voucher
                .issues
                .push(error(format!("不跟随符号链接：{}", name(&entry_path))));
        } else if file_type.is_dir() {
            voucher.issues.push(error(format!(
                "凭证中含非约定的嵌套目录：{}；请整理为直接文件",
                name(&entry_path)
            )));
        } else if !file_type.is_file() {
            voucher
                .issues
                .push(error(format!("不是普通文件：{}", name(&entry_path))));
        } else {
            let extension = entry_path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !matches!(
                extension.as_str(),
                "jpg" | "jpeg" | "png" | "bmp" | "tif" | "tiff" | "webp" | "pdf"
            ) {
                voucher
                    .issues
                    .push(error(format!("不支持的内容文件：{}", name(&entry_path))));
                continue;
            }
            match scan_file(&entry_path, root, &date, &voucher.code) {
                Ok(file) => {
                    if let Some(message) = &file.error {
                        voucher
                            .issues
                            .push(error(format!("{}：{message}", file.name)));
                    }
                    voucher.files.push(file);
                }
                Err(error) => voucher.issues.push(error_issue(
                    error.context(format!("读取文件 {} 失败", name(&entry_path))),
                )),
            }
        }
    }
    voucher
        .files
        .sort_by(|a, b| a.sequence.cmp(&b.sequence).then(a.name.cmp(&b.name)));
    let mut positions: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, file) in voucher
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.sequence > 0)
    {
        positions.entry(file.sequence).or_default().push(index);
    }
    let mut expected = 1;
    for (sequence, indices) in positions {
        if indices.len() > 1 {
            voucher
                .issues
                .push(error(format!("序号 {sequence} 重复，无法确定合并顺序")));
            for index in indices {
                add_file_error(&mut voucher.files[index], format!("序号 {sequence} 重复"));
            }
        }
        if sequence != expected {
            voucher.issues.push(error(format!(
                "文件序号缺失：预期 {expected}，实际下一项为 {sequence}"
            )));
        }
        expected = sequence.saturating_add(1);
    }
    if voucher.files.is_empty() {
        voucher
            .issues
            .push(error("凭证文件夹中没有可合并的图片或 PDF"));
    }
    Ok(voucher)
}

fn error_issue(source: anyhow::Error) -> Issue {
    error(format!("{source:#}"))
}

pub fn scan(request: &ScanRequest) -> Result<Plan> {
    let selected = PathBuf::from(request.root.trim());
    if selected.as_os_str().is_empty() {
        bail!("请选择资料根目录或分项文件夹");
    }
    if fs::symlink_metadata(&selected)
        .context("资料目录不存在")?
        .file_type()
        .is_symlink()
    {
        bail!("资料根目录不能是符号链接，请选择真实路径");
    }
    let selected = selected.canonicalize().context("无法定位资料目录")?;
    if !selected.is_dir() {
        bail!("资料路径必须为文件夹");
    }
    let selected_entries = entries(&selected)?;
    let is_item = selected_entries.iter().any(|entry| {
        entry.file_type().is_ok_and(|kind| kind.is_dir())
            && folder_regex().is_match(&name(&entry.path()))
    });
    let root = if is_item {
        selected
            .parent()
            .context("分项目录没有父目录")?
            .to_path_buf()
    } else {
        selected.clone()
    };
    let ledger_path = find_ledger(request, &selected, &root)?;
    let ledger = ledger::read(&ledger_path, request.sheet.as_deref())?;
    let mut plan = Plan {
        id: Uuid::new_v4().to_string(),
        root: root.to_string_lossy().into_owned(),
        ledger_path: ledger_path.to_string_lossy().into_owned(),
        requested_root: Some(selected.to_string_lossy().into_owned()),
        ledger_sha256: ledger.sha256.clone(),
        sheet: ledger.sheet.clone(),
        sheets: ledger.sheets.clone(),
        created_at: Utc::now().to_rfc3339(),
        vouchers: Vec::new(),
        issues: ledger.issues.clone(),
        ignored: 0,
        orientation_workflow: OrientationWorkflow::disabled(),
    };
    let mut item_paths = Vec::new();
    if is_item {
        item_paths.push(selected.clone());
    } else {
        for entry in selected_entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if ignored(&path, kind.is_dir()) || (kind.is_dir() && name(&path).ends_with("-PDF")) {
                plan.ignored += 1;
                continue;
            }
            if kind.is_symlink() {
                plan.issues.push(error(format!(
                    "根目录包含符号链接，不予扫描：{}",
                    name(&path)
                )));
            } else if kind.is_dir() {
                item_paths.push(path);
            } else if kind.is_file() && is_ledger(&path) { /* workbook is an index, not a content page */
            } else {
                plan.issues
                    .push(error(format!("根目录包含非约定内容：{}", name(&path))));
            }
        }
    }
    for item_path in item_paths {
        let item = name(&item_path);
        if item.encode_utf16().count() + 4 > 255 || item != safe_subject(&item) {
            plan.issues
                .push(error(format!("分项名称不适合 Windows 输出目录：{item}")));
            continue;
        }
        let item_entries = match entries(&item_path) {
            Ok(value) => value,
            Err(error) => {
                plan.issues.push(error_issue(error));
                continue;
            }
        };
        let previous_count = plan.vouchers.len();
        for entry in item_entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if ignored(&path, kind.is_dir()) {
                plan.ignored += 1;
                continue;
            }
            if kind.is_symlink() {
                plan.issues
                    .push(error(format!("分项“{item}”包含符号链接：{}", name(&path))));
            } else if kind.is_dir() {
                match scan_voucher(&path, &root, &item, &ledger, &mut plan.ignored) {
                    Ok(voucher) => plan.vouchers.push(voucher),
                    Err(error) => plan.issues.push(error_issue(
                        error.context(format!("凭证目录 {} 无法扫描", relative(&root, &path)?)),
                    )),
                }
            } else if path == ledger_path { /* item-local explicit or discovered ledger */
            } else {
                plan.issues.push(error(format!(
                    "分项“{item}”包含不在凭证文件夹中的文件：{}",
                    name(&path)
                )));
            }
        }
        if previous_count == plan.vouchers.len() {
            plan.issues
                .push(warning(format!("分项“{item}”没有凭证文件夹")));
        }
    }
    if plan.vouchers.is_empty() {
        plan.issues.push(error(
            "未找到凭证文件夹；请选择包含分项的批次目录，或包含 YYMMDD-编号 文件夹的单个分项",
        ));
    }
    let mut outputs: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, voucher) in plan.vouchers.iter().enumerate() {
        if let Some(output) = &voucher.output_relative_path {
            outputs
                .entry(output.to_lowercase())
                .or_default()
                .push(index);
        }
    }
    for indices in outputs.values().filter(|indices| indices.len() > 1) {
        for &index in indices {
            plan.vouchers[index]
                .issues
                .push(error("多个凭证的输出路径在 Windows 下冲突，不能生成"));
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::LedgerEntry;

    fn test_ledger() -> Ledger {
        Ledger {
            sha256: "test".into(),
            sheet: "第一表".into(),
            sheets: Vec::new(),
            issues: Vec::new(),
            entries: BTreeMap::from([(
                "00001".into(),
                LedgerEntry {
                    subject: Some("合成科目".into()),
                    row: 8,
                    duplicate_rows: vec![9],
                },
            )]),
        }
    }
    fn image(path: &Path) {
        image::RgbImage::new(4, 3).save(path).unwrap();
    }

    #[test]
    fn folder_dates_and_windows_subjects() {
        assert_eq!(
            parse_folder("240301-00001").unwrap(),
            ("240301".into(), "00001".into())
        );
        assert!(parse_folder("230229-00001").is_err());
        assert!(parse_folder("240301-1-extra").is_err());
        assert!(parse_folder("２４０３０１-00001").is_err());
        assert_eq!(safe_subject("研发/设备:款. "), "研发_设备_款");
    }

    #[test]
    fn ignores_export_resources_and_sorts_numerically() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("分项/240301-00001");
        fs::create_dir_all(folder.join("indexImg/nested")).unwrap();
        fs::write(folder.join("index.html"), "ignored").unwrap();
        fs::write(folder.join("indexImg/nested/broken.jpg"), "ignored").unwrap();
        for sequence in 1..=12 {
            image(&folder.join(format!("2403_00001_{sequence}.jpg")));
        }
        let mut ignored = 0;
        let voucher =
            scan_voucher(&folder, temp.path(), "分项", &test_ledger(), &mut ignored).unwrap();
        assert_eq!(ignored, 2);
        assert_eq!(
            voucher.files.iter().map(|f| f.sequence).collect::<Vec<_>>(),
            (1..=12).collect::<Vec<_>>()
        );
        assert!(voucher.issues.iter().all(|i| i.severity != "error"));
        assert_eq!(
            voucher.output_relative_path.as_deref(),
            Some("分项-PDF/240301-00001-合成科目.pdf")
        );
        assert_eq!(voucher.ledger_row, Some(8));
    }

    #[test]
    fn damaged_unknown_duplicate_and_missing_files_are_visible() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("分项/240301-00001");
        fs::create_dir_all(&folder).unwrap();
        image(&folder.join("2403_00001_1.jpg"));
        image(&folder.join("2403_00001_1.png"));
        fs::write(folder.join("2403_00001_3.jpg"), "not an image").unwrap();
        fs::write(folder.join("notes.docx"), "unsupported").unwrap();
        let voucher = scan_voucher(&folder, temp.path(), "分项", &test_ledger(), &mut 0).unwrap();
        let errors = voucher
            .issues
            .iter()
            .map(|i| i.message.as_str())
            .collect::<Vec<_>>()
            .join(";");
        for expected in ["重复", "缺失", "无法解码", "不支持"] {
            assert!(errors.contains(expected), "{errors}");
        }
        assert_eq!(voucher.files.len(), 3);
        assert!(voucher.files.iter().all(|f| f.error.is_some()));
    }

    #[test]
    fn mixed_pdf_and_image_keep_one_numeric_sequence() {
        use lopdf::{dictionary, Document, Object};
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("分项/240301-00001");
        fs::create_dir_all(&folder).unwrap();
        image(&folder.join("2403_00001_2.jpg"));
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()] });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", root);
        doc.save(folder.join("2403_00001_1.pdf")).unwrap();
        let voucher = scan_voucher(&folder, temp.path(), "分项", &test_ledger(), &mut 0).unwrap();
        assert_eq!(
            voucher
                .files
                .iter()
                .map(|f| (f.kind.as_str(), f.pages))
                .collect::<Vec<_>>(),
            vec![("pdf", 1), ("image", 1)]
        );
        assert!(voucher.issues.iter().all(|i| i.severity != "error"));
    }

    #[test]
    #[cfg(unix)]
    fn symbolic_links_are_not_followed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("分项/240301-00001");
        fs::create_dir_all(&folder).unwrap();
        let outside = temp.path().join("external.jpg");
        image(&outside);
        symlink(&outside, folder.join("2403_00001_1.jpg")).unwrap();
        let voucher = scan_voucher(&folder, temp.path(), "分项", &test_ledger(), &mut 0).unwrap();
        assert!(voucher.files.is_empty());
        assert!(voucher
            .issues
            .iter()
            .any(|i| i.message.contains("符号链接")));
    }

    #[test]
    fn ignore_is_exact_for_content_exports() {
        assert!(ignored(Path::new("indexImg"), true));
        assert!(ignored(Path::new("index.html"), false));
        assert!(!ignored(Path::new("IndexImg"), true));
        assert!(!ignored(Path::new("index.html"), true));
    }

    #[test]
    fn multipage_tiff_is_rejected_instead_of_silently_reading_first_page() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("2403_00001_1.tiff");
        image(&path);
        assert!(!tiff_has_multiple_pages(&path).unwrap());
        reject_multiframe_bytes(&fs::read(&path).unwrap(), Some(image::ImageFormat::Tiff)).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let little = &bytes[..2] == b"II";
        let first = if little {
            u32::from_le_bytes(bytes[4..8].try_into().unwrap())
        } else {
            u32::from_be_bytes(bytes[4..8].try_into().unwrap())
        } as usize;
        let count = if little {
            u16::from_le_bytes(bytes[first..first + 2].try_into().unwrap())
        } else {
            u16::from_be_bytes(bytes[first..first + 2].try_into().unwrap())
        } as usize;
        let pointer = first + 2 + count * 12;
        let second = bytes.len() as u32;
        // Append an identical second directory, as a real two-page TIFF does.
        let directory = bytes[first..pointer + 4].to_vec();
        bytes.extend(directory);
        bytes[pointer..pointer + 4].copy_from_slice(&if little {
            second.to_le_bytes()
        } else {
            second.to_be_bytes()
        });
        fs::write(&path, &bytes).unwrap();
        assert!(
            reject_multiframe_bytes(&bytes, Some(image::ImageFormat::Tiff))
                .unwrap_err()
                .to_string()
                .contains("多帧/多页")
        );
        assert!(tiff_has_multiple_pages(&path).unwrap());
        let file = scan_file(&path, temp.path(), "240301", "00001").unwrap();
        assert_eq!(file.pages, 0);
        assert!(file.error.as_deref().unwrap().contains("多帧/多页"));
    }
}
