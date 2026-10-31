//! Page-only PDF assembly. No source is ever modified.
use crate::model::{Orientation, OrientationWorkflow, Plan, SourceFile, Voucher};
use anyhow::{bail, Context, Result};
use image::ImageDecoder;
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, ObjectId, Stream,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageMapping {
    pub source: String,
    pub sha256: String,
    pub first_page: u32,
    pub last_page: u32,
    pub rotation_cw: u16,
    pub reviewed: bool,
    pub confidence: Option<f32>,
    #[serde(default)]
    pub orientation: Option<Orientation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub producer: String,
    pub version: String,
    pub created_at: String,
    pub root: String,
    pub folder: String,
    pub ledger_path: String,
    pub ledger_sha256: String,
    pub sheet: String,
    pub ledger_row: Option<u32>,
    pub duplicate_rows: Vec<u32>,
    pub subject: Option<String>,
    pub pages: u32,
    pub output_sha256: String,
    pub needs_review: bool,
    pub sources: Vec<PageMapping>,
    #[serde(default)]
    pub orientation_workflow: OrientationWorkflow,
}
pub struct BuildResult {
    pub path: PathBuf,
    pub manifest: Manifest,
    pub skipped: bool,
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    temporary: String,
    output: String,
    backup: Option<String>,
    previous_hash: Option<String>,
    manifest: Manifest,
}

fn transaction_path(directory: &Path, output: &str) -> PathBuf {
    directory.join(format!(
        ".voucher-{}.transaction.json",
        hex::encode(Sha256::digest(output.as_bytes()))
    ))
}

/// Stable across rescans and jobs, but distinct for identically named PDFs in
/// different directories. Resolving the existing parent also normalizes the
/// Windows extended-path prefix used by the scanner.
pub fn manifest_path(state_dir: &Path, output: &Path) -> Result<PathBuf> {
    let output = output
        .parent()
        .context("输出目录无效")?
        .canonicalize()?
        .join(output.file_name().context("输出文件名无效")?);
    let identity = output.to_string_lossy();
    #[cfg(windows)]
    let identity = identity.to_lowercase();
    let key = hex::encode(Sha256::digest(identity.as_bytes()));
    Ok(state_dir.join("outputs").join(key).join("manifest.json"))
}

struct OutputRecords {
    output: PathBuf,
    // None is only used to finish an interrupted pre-0.4.1 transaction before
    // its verified sidecar is migrated. New transactions always use state_dir.
    directory: Option<PathBuf>,
}

impl OutputRecords {
    fn new(state_dir: &Path, output: &Path) -> Result<Self> {
        fs::create_dir_all(state_dir)?;
        let state = state_dir.canonicalize()?;
        let manifest = manifest_path(&state, output)?;
        let directory = manifest.parent().context("输出记录目录无效")?;
        for path in [state.join("outputs"), directory.to_owned()] {
            reject_symlink(&path)?;
            fs::create_dir_all(path)?;
        }
        let records = Self {
            output: output.to_owned(),
            directory: Some(directory.to_owned()),
        };
        recover_atomic_record(&records.manifest())?;
        recover_atomic_record(&records.journal())?;
        Ok(records)
    }

    fn legacy(output: &Path) -> Self {
        Self {
            output: output.to_owned(),
            directory: None,
        }
    }

    fn manifest(&self) -> PathBuf {
        self.directory.as_ref().map_or_else(
            || self.output.with_extension("manifest.json"),
            |directory| directory.join("manifest.json"),
        )
    }

    fn journal(&self) -> PathBuf {
        self.directory.as_ref().map_or_else(
            || {
                transaction_path(
                    self.output.parent().unwrap(),
                    self.output.file_name().unwrap().to_str().unwrap(),
                )
            },
            |directory| directory.join("transaction.json"),
        )
    }

    fn archive(&self, backup: &Path) -> Result<PathBuf> {
        let path = if let Some(directory) = &self.directory {
            let relative = backup
                .strip_prefix(self.output.parent().context("输出目录无效")?)?
                .with_extension("manifest.json");
            checked_output_child(directory, relative.to_str().context("历史记录路径无效")?)?
        } else {
            backup.with_extension("manifest.json")
        };
        reject_symlink(&path)?;
        Ok(path)
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    // symlink_metadata also detects dangling links, unlike Path::exists().
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => bail!("输出及记录不能是符号链接"),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn recover_atomic_record(path: &Path) -> Result<()> {
    let previous = path.with_extension("json.previous");
    reject_symlink(path)?;
    reject_symlink(&previous)?;
    if !path.exists() && previous.exists() {
        fs::rename(previous, path)?;
    }
    Ok(())
}

fn validate_owned_manifest(
    plan: &Plan,
    voucher: &Voucher,
    output: &Path,
    manifest: &Manifest,
) -> Result<()> {
    if manifest.producer != "voucher-workbench"
        || manifest.root != plan.root
        || manifest.folder != voucher.folder
        || sha256_file(output)? != manifest.output_sha256
    {
        bail!("已有输出被修改或不属于此任务，拒绝覆盖");
    }
    Ok(())
}

fn migrate_legacy_manifest(records: &OutputRecords, plan: &Plan, voucher: &Voucher) -> Result<()> {
    let legacy = records.output.with_extension("manifest.json");
    recover_atomic_record(&legacy)?;
    if !legacy.exists() {
        return Ok(());
    }
    let bytes = fs::read(&legacy)?;
    let old: Manifest = serde_json::from_slice(&bytes).context("旧输出记录无法验证，保留原文件")?;
    validate_owned_manifest(plan, voucher, &records.output, &old)?;
    let previous = legacy.with_extension("json.previous");
    let previous_bytes = if previous.exists() {
        let bytes = fs::read(&previous)?;
        let saved: Manifest =
            serde_json::from_slice(&bytes).context("旧输出残留记录无法验证，保留原文件")?;
        if serde_json::to_value(&saved)? != serde_json::to_value(&old)? {
            bail!("旧输出记录与替换前副本冲突，保留双方记录，请核对后再生成");
        }
        Some(bytes)
    } else {
        None
    };
    let target = records.manifest();
    if target.exists() {
        let current: Manifest = serde_json::from_slice(&fs::read(&target)?)?;
        if serde_json::to_value(&current)? != serde_json::to_value(&old)? {
            bail!("本地记录与旧输出记录冲突，保留双方记录，请核对后再生成");
        }
    } else {
        atomic_json(&target, &old)?;
    }
    // The old sidecar is removed only after a durable, verified copy exists.
    // Copying (rather than renaming) also works when state and PDFs are on
    // different drives. If interrupted, the next attempt repeats this check.
    let saved: Manifest = serde_json::from_slice(&fs::read(&target)?)?;
    if serde_json::to_value(&saved)? != serde_json::to_value(&old)? || fs::read(&legacy)? != bytes {
        bail!("旧输出记录迁移校验失败，保留原文件");
    }
    validate_owned_manifest(plan, voucher, &records.output, &saved)?;
    if let Some(bytes) = previous_bytes {
        if fs::read(&previous)? != bytes {
            bail!("旧输出残留记录在迁移期间变化，保留原文件");
        }
        // Remove the verified duplicate first: a crash can then leave only
        // the valid sidecar, never a stale .previous that revives next time.
        fs::remove_file(previous)?;
    }
    fs::remove_file(legacy)?;
    Ok(())
}

fn checked_output_child(directory: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if rel.is_absolute() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("输出事务路径无效");
    }
    let mut path = directory.to_owned();
    for part in rel.components() {
        path.push(part.as_os_str());
        reject_symlink(&path)?;
    }
    Ok(path)
}

// A tiny write-ahead journal makes the PDF + manifest pair recoverable on both
// Windows and Unix. Once commit starts, cancellation only stops the next voucher.
fn recover_transaction(directory: &Path, output_name: &str, records: &OutputRecords) -> Result<()> {
    let journal = records.journal();
    recover_atomic_record(&journal)?;
    if !journal.exists() {
        return Ok(());
    }
    let transaction: Transaction = serde_json::from_slice(&fs::read(&journal)?)?;
    if transaction.output != output_name {
        bail!("输出事务与目标文件不符");
    }
    let output = checked_output_child(directory, &transaction.output)?;
    let temporary = checked_output_child(directory, &transaction.temporary)?;
    let report = records.manifest();
    recover_atomic_record(&report)?;
    let backup = transaction
        .backup
        .as_deref()
        .map(|s| checked_output_child(directory, s))
        .transpose()?;
    let target_hash = &transaction.manifest.output_sha256;
    let current_hash = if output.exists() {
        Some(sha256_file(&output)?)
    } else {
        None
    };
    // A workflow/consent-only change can produce identical PDF bytes. The old
    // PDF + manifest must still be archived before replacing the manifest; a
    // matching target hash alone does not mean this transaction committed.
    let archive_pending = current_hash.is_some()
        && current_hash == transaction.previous_hash
        && backup.as_ref().is_some_and(|path| !path.exists());
    if current_hash.as_deref() != Some(target_hash.as_str()) || archive_pending {
        if current_hash.is_some() && current_hash != transaction.previous_hash {
            bail!("事务恢复发现输出被外部修改，拒绝覆盖");
        }
        if !temporary.exists() || sha256_file(&temporary)? != *target_hash {
            // If the pending artifact disappeared, restore the previous owned output.
            if !output.exists() {
                if let Some(backup) = &backup {
                    if backup.exists() {
                        fs::rename(backup, &output)?;
                    }
                }
            }
            if !report.exists() {
                if let Some(backup) = &backup {
                    let previous = records.archive(backup)?;
                    if previous.exists() {
                        fs::rename(previous, &report)?;
                    }
                }
            }
            bail!("待提交文件缺失或损坏，已尽可能恢复旧输出；请保留事务记录排查");
        }
        if let Some(backup) = &backup {
            fs::create_dir_all(backup.parent().context("备份目录无效")?)?;
            if output.exists() {
                if backup.exists() {
                    bail!("事务备份已存在，拒绝覆盖");
                }
                fs::rename(&output, backup)?;
            }
            if report.exists() {
                let saved = records.archive(backup)?;
                if !saved.exists() {
                    fs::create_dir_all(saved.parent().context("历史记录目录无效")?)?;
                    fs::rename(&report, saved)?;
                }
            }
        } else if output.exists() {
            bail!("未知输出阻止事务提交");
        }
        fs::rename(&temporary, &output)?;
    }
    // PDF is already committed, so repair/finalize its matching report even if
    // the previous process died between the two filesystem operations.
    atomic_json(&report, &transaction.manifest)?;
    if temporary.exists() {
        fs::remove_file(temporary)?;
    }
    let previous_journal = journal.with_extension("json.previous");
    if previous_journal.exists() {
        fs::remove_file(previous_journal)?;
    }
    fs::remove_file(journal)?;
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub fn safe_source(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if rel.is_absolute() || rel.components().any(|p| !matches!(p, Component::Normal(_))) {
        bail!("非法相对路径");
    }
    let base = root.canonicalize()?;
    let mut current = base.clone();
    for c in rel.components() {
        current.push(c.as_os_str());
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            bail!("不允许访问符号链接");
        }
    }
    let p = current.canonicalize()?;
    if !p.starts_with(&base) {
        bail!("文件超出资料目录");
    }
    Ok(p)
}

pub fn check_source(root: &Path, source: &SourceFile) -> Result<PathBuf> {
    let path = safe_source(root, &source.relative_path)?;
    let meta = fs::metadata(&path)?;
    let ms = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    if meta.len() != source.bytes
        || ms != source.modified_ms
        || source.sha256.is_empty()
        || sha256_file(&path)? != source.sha256
    {
        bail!("源文件已变化或计划缺少指纹，请重新扫描：{}", source.name);
    }
    Ok(path)
}

pub fn needs_review(f: &SourceFile) -> bool {
    f.kind == "image"
        && !f.reviewed
        && f.orientation
            .as_ref()
            .map(|o| !crate::orientation::is_current(o) || o.needs_review)
            .unwrap_or(true)
}

pub fn requires_manual_review(plan: &Plan, file: &SourceFile) -> bool {
    plan.orientation_workflow.enabled && file.kind == "image" && !file.reviewed
}

pub fn effective_rotation(plan: &Plan, file: &SourceFile) -> u16 {
    if file.reviewed || file.kind != "image" {
        file.rotation_cw
    } else if plan.orientation_workflow.enabled {
        file.orientation
            .as_ref()
            .filter(|o| crate::orientation::is_current(o))
            .map(|o| o.rotation_cw)
            .unwrap_or(0)
    } else {
        0
    }
}

pub fn validate_generation_workflow(plan: &Plan) -> Result<()> {
    if plan.orientation_workflow.enabled {
        if !plan.orientation_workflow.has_current_consent() {
            bail!("启用方向识别前须同意当前免责声明");
        }
        if !plan.orientation_workflow.analysis_completed {
            bail!("请先完成本计划的方向识别，再人工核对确认后生成");
        }
    }
    Ok(())
}

pub fn is_orientation_error(error: &str) -> bool {
    error.starts_with("方向识别失败：")
}

pub fn needs_analysis(f: &SourceFile) -> bool {
    f.kind == "image"
        && !f.reviewed
        && !f
            .orientation
            .as_ref()
            .is_some_and(crate::orientation::is_current)
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("任务已取消");
    }
    Ok(())
}

fn inherited(doc: &Document, id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut current = id;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current) {
        let dict = doc.get_object(current).ok()?.as_dict().ok()?;
        if let Ok(v) = dict.get(key) {
            return Some(v.clone());
        }
        current = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn append_pdf(
    output: &mut Document,
    page_root: ObjectId,
    kids: &mut Vec<Object>,
    path: &Path,
    rotation: u16,
    cancel: &AtomicBool,
) -> Result<u32> {
    let mut source = Document::load(path).context("无法读取 PDF")?;
    if source.is_encrypted() {
        bail!("加密 PDF 不支持直接合并");
    }
    source.renumber_objects_with(output.max_id + 1);
    let pages = source.get_pages();
    if pages.is_empty() {
        bail!("PDF 没有页面");
    }
    for id in pages.values() {
        cancelled(cancel)?;
        let mut page = source.get_object(*id)?.as_dict()?.clone();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if !page.has(key) {
                if let Some(value) = inherited(&source, *id, key) {
                    page.set(key, value);
                }
            }
        }
        if rotation != 0 {
            let existing = page
                .get(b"Rotate")
                .ok()
                .and_then(|v| source.dereference(v).ok())
                .and_then(|(_, v)| v.as_i64().ok())
                .unwrap_or(0);
            page.set("Rotate", (existing + rotation as i64).rem_euclid(360));
        }
        page.set("Parent", page_root);
        source.objects.insert(*id, Object::Dictionary(page));
        kids.push(Object::Reference(*id));
    }
    output.max_id = output.max_id.max(source.max_id);
    output.objects.extend(source.objects);
    Ok(pages.len() as u32)
}

// SOF describes the original JPEG component count (CMYK must not be marked DeviceRGB).
fn jpeg_components(data: &[u8]) -> Option<u8> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut p = 2;
    while p + 4 <= data.len() {
        if data[p] != 0xff {
            return None;
        }
        while p < data.len() && data[p] == 0xff {
            p += 1;
        }
        let marker = *data.get(p)?;
        p += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let len = u16::from_be_bytes([*data.get(p)?, *data.get(p + 1)?]) as usize;
        if len < 2 || p + len > data.len() {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            return data.get(p + 7).copied();
        }
        p += len;
    }
    None
}

fn append_image(
    output: &mut Document,
    page_root: ObjectId,
    kids: &mut Vec<Object>,
    path: &Path,
    rotation: u16,
) -> Result<()> {
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let exif = decoder.orientation()?;
    drop(decoder);
    let img = crate::orientation::load_image(path)?;
    let (w, h) = (img.width(), img.height());
    let raw = fs::read(path)?;
    let components = jpeg_components(&raw);
    let (bytes, color) = if exif == image::metadata::Orientation::NoTransforms
        && matches!(components, Some(1 | 3))
    {
        (
            raw,
            if components == Some(1) {
                "DeviceGray"
            } else {
                "DeviceRGB"
            },
        )
    } else {
        // Composite transparent scans on white, not black, then normalize to RGB JPEG.
        let mut rgb = img.to_rgb8();
        if img.color().has_alpha() {
            let rgba = img.to_rgba8();
            for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
                let a = src[3] as u16;
                for c in 0..3 {
                    dst[c] = ((src[c] as u16 * a + 255 * (255 - a) + 127) / 255) as u8;
                }
            }
        }
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95).encode_image(&rgb)?;
        (bytes, "DeviceRGB")
    };
    let image_id = output.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>w, "Height"=>h,
            "ColorSpace"=>color, "BitsPerComponent"=>8, "Filter"=>"DCTDecode"
        },
        bytes,
    ));
    let (rw, rh) = if rotation % 180 == 90 {
        (h as f32, w as f32)
    } else {
        (w as f32, h as f32)
    };
    let (pw, ph) = if rw > rh {
        (841.8898_f32, 595.2756_f32)
    } else {
        (595.2756_f32, 841.8898_f32)
    };
    let scale = (pw / rw).min(ph / rh);
    let (dw, dh) = (w as f32 * scale, h as f32 * scale);
    let (x, y) = ((pw - rw * scale) / 2., (ph - rh * scale) / 2.);
    let matrix = match rotation {
        0 => [dw, 0., 0., dh, x, y],
        90 => [0., -dw, dh, 0., x, y + dw],
        180 => [-dw, 0., 0., -dh, x + dw, y + dh],
        270 => [0., dw, -dh, 0., x + dh, y],
        _ => bail!("旋转角度必须是0/90/180/270"),
    };
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new("cm", matrix.into_iter().map(Object::Real).collect()),
            Operation::new("Do", vec![Object::Name(b"Scan".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };
    let content_id = output.add_object(Stream::new(Dictionary::new(), content.encode()?));
    let page_id = output.add_object(dictionary! {
        "Type"=>"Page", "Parent"=>page_root, "MediaBox"=>vec![0.into(),0.into(),pw.into(),ph.into()],
        "Resources"=>dictionary! {"XObject"=>dictionary! {"Scan"=>image_id}}, "Contents"=>content_id
    });
    kids.push(page_id.into());
    Ok(())
}

pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let data = serde_json::to_vec_pretty(value)?;
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    f.write_all(&data)?;
    f.sync_all()?;
    drop(f);
    // Windows rename cannot replace an existing file. Keep a recoverable previous copy.
    let previous = path.with_extension("json.previous");
    if path.exists() {
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(path, &previous)?;
    }
    if let Err(err) = fs::rename(&temp, path) {
        if previous.exists() {
            let _ = fs::rename(&previous, path);
        }
        return Err(err.into());
    }
    if previous.exists() {
        fs::remove_file(previous)?;
    }
    Ok(())
}

pub fn build_voucher(
    state_dir: &Path,
    plan: &Plan,
    voucher: &Voucher,
    allow_unreviewed: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u32),
) -> Result<BuildResult> {
    cancelled(cancel)?;
    if allow_unreviewed {
        bail!("不再支持跳过人工确认生成，请先核对方向并确认");
    }
    validate_generation_workflow(plan)?;
    if voucher.issues.iter().any(|i| i.severity == "error") {
        bail!("凭证存在扫描错误，请修正后重新扫描");
    }
    if voucher.files.is_empty() {
        bail!("凭证没有可合并文件");
    }
    if voucher.files.iter().any(|f| {
        f.error
            .as_deref()
            .is_some_and(|error| plan.orientation_workflow.enabled || !is_orientation_error(error))
    }) {
        bail!("源文件存在错误，请修正后重新扫描");
    }
    let review = voucher
        .files
        .iter()
        .any(|f| requires_manual_review(plan, f));
    if review {
        bail!("所选凭证的所有图片均须在识别完成后人工核对修正并确认");
    }
    let root = Path::new(&plan.root);
    if sha256_file(Path::new(&plan.ledger_path))? != plan.ledger_sha256 {
        bail!("明细账已变化，请重新扫描");
    }
    let relative = voucher
        .output_relative_path
        .as_ref()
        .context("没有有效输出文件名")?;
    let parts: Vec<_> = Path::new(relative).components().collect();
    if parts.len() != 2
        || parts.iter().any(|p| !matches!(p, Component::Normal(_)))
        || !parts[0].as_os_str().to_string_lossy().ends_with("-PDF")
    {
        bail!("输出路径非法");
    }
    let directory = root.join(parts[0].as_os_str());
    if directory.exists() && fs::symlink_metadata(&directory)?.file_type().is_symlink() {
        bail!("输出目录不能是符号链接");
    }
    fs::create_dir_all(&directory)?;
    let path = root.join(relative);
    reject_symlink(&path)?;
    let output_name = path
        .file_name()
        .context("输出文件名无效")?
        .to_str()
        .context("输出文件名非UTF8")?;
    let records = OutputRecords::new(state_dir, &path)?;
    let legacy = OutputRecords::legacy(&path);
    recover_atomic_record(&legacy.journal())?;
    if legacy.journal().exists() {
        if records.journal().exists() || records.manifest().exists() {
            bail!("发现新旧输出事务记录冲突，保留记录，请核对后再生成");
        }
        recover_transaction(&directory, output_name, &legacy)?;
    }
    recover_transaction(&directory, output_name, &records)?;
    migrate_legacy_manifest(&records, plan, voucher)?;
    let report_path = records.manifest();
    let mut sources = Vec::new();
    let mut next_page = 1u32;
    for file in &voucher.files {
        cancelled(cancel)?;
        let source = check_source(root, file)?;
        sources.push(PageMapping {
            source: file.relative_path.clone(),
            sha256: sha256_file(&source)?,
            first_page: next_page,
            last_page: next_page + file.pages - 1,
            rotation_cw: effective_rotation(plan, file),
            reviewed: file.reviewed,
            confidence: file.orientation.as_ref().map(|o| o.confidence),
            orientation: file.orientation.clone(),
        });
        next_page += file.pages;
    }
    let manifest = Manifest {
        producer: "voucher-workbench".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        root: plan.root.clone(),
        folder: voucher.folder.clone(),
        ledger_path: plan.ledger_path.clone(),
        ledger_sha256: plan.ledger_sha256.clone(),
        sheet: plan.sheet.clone(),
        ledger_row: voucher.ledger_row,
        duplicate_rows: voucher.duplicate_rows.clone(),
        subject: voucher.subject.clone(),
        pages: next_page - 1,
        output_sha256: String::new(),
        needs_review: review,
        sources,
        orientation_workflow: plan.orientation_workflow.clone(),
    };
    if path.exists() {
        let old: Manifest = serde_json::from_slice(
            &fs::read(&report_path).context("输出已存在且无工具生成记录，拒绝覆盖")?,
        )
        .context("输出记录无法验证，拒绝覆盖")?;
        validate_owned_manifest(plan, voucher, &path, &old)?;
        if old.sources == manifest.sources
            && old.orientation_workflow == manifest.orientation_workflow
            && old.ledger_sha256 == manifest.ledger_sha256
            && old.sheet == manifest.sheet
            && old.subject == manifest.subject
        {
            progress(manifest.pages);
            return Ok(BuildResult {
                path,
                manifest: old,
                skipped: true,
            });
        }
        // Same owned output, but changed review/rotation: retain the old deliverable before replacement.
    }
    let temporary = directory.join(format!(".{}.partial.pdf", uuid::Uuid::new_v4()));
    let result = (|| -> Result<BuildResult> {
        let mut document = Document::with_version("1.7");
        let page_root = document.new_object_id();
        let mut kids = Vec::new();
        for (file, mapping) in voucher.files.iter().zip(&manifest.sources) {
            cancelled(cancel)?;
            let source = check_source(root, file)?;
            let count = if file.kind == "pdf" {
                append_pdf(
                    &mut document,
                    page_root,
                    &mut kids,
                    &source,
                    mapping.rotation_cw,
                    cancel,
                )?
            } else {
                append_image(
                    &mut document,
                    page_root,
                    &mut kids,
                    &source,
                    mapping.rotation_cw,
                )?;
                1
            };
            if count != file.pages || sha256_file(&source)? != mapping.sha256 {
                bail!("源文件在生成期间发生变化：{}", file.name);
            }
            progress(count);
        }
        document.objects.insert(
            page_root,
            Object::Dictionary(
                dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>manifest.pages},
            ),
        );
        let catalog = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>page_root});
        document.trailer.set("Root", catalog);
        document.prune_objects();
        // Windows FlushFileBuffers requires write access. Keep the writer's
        // handle instead of reopening the saved PDF as read-only for sync.
        document
            .save(&temporary)
            .context("临时PDF写入失败")?
            .sync_all()
            .context("临时PDF同步到磁盘失败")?;
        cancelled(cancel)?;
        let verify = Document::load(&temporary)?;
        if verify.get_pages().len() as u32 != manifest.pages {
            bail!("输出页数校验不一致");
        }
        drop(verify);
        let mut manifest = manifest;
        manifest.output_sha256 = sha256_file(&temporary)?;
        cancelled(cancel)?;
        let output_name = path.file_name().unwrap().to_string_lossy().to_string();
        let previous_hash = if path.exists() {
            Some(sha256_file(&path)?)
        } else {
            None
        };
        let transaction = Transaction {
            temporary: temporary.file_name().unwrap().to_string_lossy().into(),
            output: output_name.clone(),
            backup: previous_hash
                .as_ref()
                .map(|_| format!(".history/{}/{}", uuid::Uuid::new_v4(), output_name)),
            previous_hash,
            manifest: manifest.clone(),
        };
        if let Some(backup) = &transaction.backup {
            checked_output_child(&directory, backup)?;
        }
        atomic_json(&records.journal(), &transaction)?;
        recover_transaction(&directory, &output_name, &records)?;
        Ok(BuildResult {
            path: path.clone(),
            manifest,
            skipped: false,
        })
    })();
    if result.is_err() && !records.journal().exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    fn manifest(hash: String) -> Manifest {
        Manifest {
            producer: "voucher-workbench".into(),
            version: "test".into(),
            created_at: String::new(),
            root: String::new(),
            folder: "synthetic".into(),
            ledger_path: String::new(),
            ledger_sha256: String::new(),
            sheet: "first".into(),
            ledger_row: Some(1),
            duplicate_rows: vec![],
            subject: Some("synthetic".into()),
            pages: 1,
            output_sha256: hash,
            needs_review: false,
            sources: vec![],
            orientation_workflow: OrientationWorkflow::disabled(),
        }
    }

    #[test]
    fn interrupted_pdf_report_commit_recovers_at_every_boundary() {
        for (central, stage) in [false, true]
            .into_iter()
            .flat_map(|central| (0..=3).map(move |stage| (central, stage)))
        {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path();
            let output = dir.join("result.pdf");
            let records = if central {
                OutputRecords::new(&dir.join("state"), &output).unwrap()
            } else {
                OutputRecords::legacy(&output)
            };
            let report = records.manifest();
            let pending = dir.join(".pending.pdf");
            let backup = dir.join(".history/revision/result.pdf");
            let archived_report = records.archive(&backup).unwrap();
            fs::create_dir_all(archived_report.parent().unwrap()).unwrap();
            fs::create_dir_all(backup.parent().unwrap()).unwrap();
            fs::write(&output, b"old verified output").unwrap();
            fs::write(&report, b"old report").unwrap();
            fs::write(&pending, b"new verified output").unwrap();
            let old_hash = sha256_file(&output).unwrap();
            let new_hash = sha256_file(&pending).unwrap();
            let txn = Transaction {
                temporary: ".pending.pdf".into(),
                output: "result.pdf".into(),
                backup: Some(".history/revision/result.pdf".into()),
                previous_hash: Some(old_hash),
                manifest: manifest(new_hash.clone()),
            };
            atomic_json(&records.journal(), &txn).unwrap();
            if stage >= 1 {
                fs::rename(&output, &backup).unwrap();
            }
            if stage >= 2 {
                fs::rename(&report, &archived_report).unwrap();
            }
            if stage >= 3 {
                fs::rename(&pending, &output).unwrap();
            }
            recover_transaction(dir, "result.pdf", &records).unwrap();
            assert_eq!(sha256_file(&output).unwrap(), new_hash);
            let saved: Manifest = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
            assert_eq!(saved.output_sha256, new_hash);
            assert_eq!(fs::read(&backup).unwrap(), b"old verified output");
            assert_eq!(fs::read(&archived_report).unwrap(), b"old report");
            assert!(!records.journal().exists());
        }
    }
    #[test]
    fn first_commit_can_recover_pdf_without_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let output = dir.join("result.pdf");
        let records = OutputRecords::new(&dir.join("state"), &output).unwrap();
        fs::write(&output, b"new verified output").unwrap();
        let hash = sha256_file(&output).unwrap();
        let txn = Transaction {
            temporary: ".pending.pdf".into(),
            output: "result.pdf".into(),
            backup: None,
            previous_hash: None,
            manifest: manifest(hash.clone()),
        };
        atomic_json(&records.journal(), &txn).unwrap();
        recover_transaction(dir, "result.pdf", &records).unwrap();
        let saved: Manifest =
            serde_json::from_slice(&fs::read(records.manifest()).unwrap()).unwrap();
        assert_eq!(saved.output_sha256, hash);
    }
    #[test]
    fn identical_pdf_bytes_preserve_manifest_history_across_recovery() {
        for (central, stage) in [false, true]
            .into_iter()
            .flat_map(|central| (0..=4).map(move |stage| (central, stage)))
        {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path();
            let output = dir.join("result.pdf");
            let records = if central {
                OutputRecords::new(&dir.join("state"), &output).unwrap()
            } else {
                OutputRecords::legacy(&output)
            };
            let report = records.manifest();
            let pending = dir.join(".pending.pdf");
            let backup = dir.join(".history/revision/result.pdf");
            let archived_report = records.archive(&backup).unwrap();
            fs::create_dir_all(archived_report.parent().unwrap()).unwrap();
            fs::create_dir_all(backup.parent().unwrap()).unwrap();
            fs::write(&output, b"same PDF bytes").unwrap();
            fs::write(&pending, b"same PDF bytes").unwrap();
            let hash = sha256_file(&output).unwrap();
            let old = manifest(hash.clone());
            atomic_json(&report, &old).unwrap();
            let old_report_bytes = fs::read(&report).unwrap();
            let mut new = old.clone();
            new.orientation_workflow.enabled = true;
            new.orientation_workflow.consent_version = Some("new consent".into());
            new.orientation_workflow.consented_at = Some("new time".into());
            new.orientation_workflow.analysis_completed = true;
            let txn = Transaction {
                temporary: ".pending.pdf".into(),
                output: "result.pdf".into(),
                backup: Some(".history/revision/result.pdf".into()),
                previous_hash: Some(hash.clone()),
                manifest: new.clone(),
            };
            let journal = records.journal();
            atomic_json(&journal, &txn).unwrap();
            if stage >= 1 {
                fs::rename(&output, &backup).unwrap();
            }
            if stage >= 2 {
                fs::rename(&report, &archived_report).unwrap();
            }
            if stage >= 3 {
                fs::rename(&pending, &output).unwrap();
            }
            if stage >= 4 {
                atomic_json(&report, &new).unwrap();
            }
            recover_transaction(dir, "result.pdf", &records).unwrap();
            // Replaying the same committed journal cannot overwrite the old
            // consent record with the now-current manifest.
            atomic_json(&journal, &txn).unwrap();
            recover_transaction(dir, "result.pdf", &records).unwrap();
            assert_eq!(fs::read(&backup).unwrap(), b"same PDF bytes");
            assert_eq!(fs::read(&archived_report).unwrap(), old_report_bytes);
            assert_eq!(sha256_file(&output).unwrap(), hash);
            let saved: Manifest = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
            assert_eq!(saved.orientation_workflow, new.orientation_workflow);
            assert!(!journal.exists());
        }
    }
    #[test]
    fn recovery_never_overwrites_a_foreign_file() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let output = dir.join("result.pdf");
        let records = OutputRecords::new(&dir.join("state"), &output).unwrap();
        fs::write(&output, b"external changes").unwrap();
        fs::write(dir.join(".pending.pdf"), b"new").unwrap();
        let txn = Transaction {
            temporary: ".pending.pdf".into(),
            output: "result.pdf".into(),
            backup: None,
            previous_hash: None,
            manifest: manifest(sha256_file(&dir.join(".pending.pdf")).unwrap()),
        };
        atomic_json(&records.journal(), &txn).unwrap();
        assert!(recover_transaction(dir, "result.pdf", &records).is_err());
        assert_eq!(fs::read(output).unwrap(), b"external changes");
    }
}
