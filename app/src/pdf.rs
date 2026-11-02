//! Page-only PDF assembly. No source is ever modified.
use crate::model::{Orientation, OrientationWorkflow, Plan, SourceFile, Voucher};
use anyhow::{bail, Context, Result};
use image::ImageDecoder;
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, ObjectId, Stream,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
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

pub(crate) trait RecordHash {
    fn output_hash(&self) -> &str;
}
impl RecordHash for Manifest {
    fn output_hash(&self) -> &str {
        &self.output_sha256
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Transaction<M = Manifest> {
    #[serde(default)]
    pub(crate) replace_existing: bool,
    pub(crate) temporary: String,
    pub(crate) output: String,
    pub(crate) backup: Option<String>,
    pub(crate) previous_hash: Option<String>,
    pub(crate) manifest: M,
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

pub(crate) struct OutputRecords {
    output: PathBuf,
    // None is only used to finish an interrupted pre-0.4.1 transaction before
    // its verified sidecar is migrated. New transactions always use state_dir.
    directory: Option<PathBuf>,
}

impl OutputRecords {
    pub(crate) fn new(state_dir: &Path, output: &Path) -> Result<Self> {
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

    pub(crate) fn manifest(&self) -> PathBuf {
        self.directory.as_ref().map_or_else(
            || self.output.with_extension("manifest.json"),
            |directory| directory.join("manifest.json"),
        )
    }

    pub(crate) fn journal(&self) -> PathBuf {
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

pub(crate) fn checked_output_child(directory: &Path, relative: &str) -> Result<PathBuf> {
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
    recover_record_transaction::<Manifest>(directory, output_name, records)
}

/// Same-volume replacement never deletes the old destination before the new
/// fully-synced file is ready. New batch overwrites deliberately keep no backup.
fn replace_file_atomic(source: &Path, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let mut source: Vec<u16> = source.as_os_str().encode_wide().collect();
        let mut target: Vec<u16> = target.as_os_str().encode_wide().collect();
        if source.contains(&0) || target.contains(&0) {
            bail!("替换路径包含无效字符");
        }
        source.push(0);
        target.push(0);
        // Both paths are children of the same validated directory; no copy/delete fallback.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("原子替换失败，旧目标未预先删除");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target).context("原子替换失败，旧目标未预先删除")
    }
}

fn replace_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        reject_symlink(path)?;
        if path.exists() && !fs::metadata(path)?.is_file() {
            bail!("记录目标不是普通文件");
        }
        let bytes = serde_json::to_vec_pretty(value)?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file_atomic(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn finish_replacement<M: DeserializeOwned + Serialize + RecordHash>(
    directory: &Path,
    records: &OutputRecords,
    transaction: &Transaction<M>,
) -> Result<()> {
    if transaction.backup.is_some() || transaction.previous_hash.is_none() {
        bail!("无备份替换事务缺少有效的旧目标指纹或包含不允许的备份路径");
    }
    let output = checked_output_child(directory, &transaction.output)?;
    let temporary = checked_output_child(directory, &transaction.temporary)?;
    let report = records.manifest();
    reject_symlink(&report)?;
    if report.exists() && !fs::metadata(&report)?.is_file() {
        bail!("处理记录目标不是普通文件，无法安全提交");
    }
    let expected = transaction.manifest.output_hash();
    if temporary.exists() {
        if !fs::metadata(&temporary)?.is_file() || sha256_file(&temporary)? != expected {
            bail!("新PDF临时文件缺失或损坏，旧目标未替换");
        }
        // Check the authorized old content immediately before the replacement.
        reject_symlink(&output)?;
        if !output.exists()
            || !fs::metadata(&output)?.is_file()
            || sha256_file(&output)? != transaction.previous_hash.as_deref().unwrap_or_default()
        {
            bail!("替换前发现目标已被外部修改，未覆盖；请核对后重新确认");
        }
        replace_file_atomic(&temporary, &output)?;
    } else {
        reject_symlink(&output)?;
        if !output.exists()
            || !fs::metadata(&output)?.is_file()
            || sha256_file(&output)? != expected
        {
            bail!("新PDF临时文件缺失且目标不匹配，未尝试删除或改写目标");
        }
    }
    // The PDF is committed. The journal contains the complete new record and
    // repairs it after a crash, without saving the old PDF or old manifest.
    replace_json_atomic(&report, &transaction.manifest)?;
    let journal = records.journal();
    let previous = journal.with_extension("json.previous");
    if previous.exists() {
        fs::remove_file(previous)?;
    }
    fs::remove_file(journal)?;
    Ok(())
}

pub(crate) fn recover_record_transaction<M: DeserializeOwned + Serialize + RecordHash>(
    directory: &Path,
    output_name: &str,
    records: &OutputRecords,
) -> Result<()> {
    let journal = records.journal();
    recover_atomic_record(&journal)?;
    if !journal.exists() {
        return Ok(());
    }
    let transaction: Transaction<M> = serde_json::from_slice(&fs::read(&journal)?)?;
    if transaction.output != output_name {
        bail!("输出事务与目标文件不符");
    }
    if transaction.replace_existing {
        return finish_replacement(directory, records, &transaction);
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
    let target_hash = transaction.manifest.output_hash();
    let current_hash = if output.exists() {
        Some(sha256_file(&output)?)
    } else {
        None
    };
    // For a first commit, rename consumes the pending file. Equal bytes alone
    // are not evidence that this journal owns a newly appeared destination.
    // Another state directory/process may have produced the same PDF bytes.
    if current_hash.is_some() && current_hash != transaction.previous_hash && temporary.exists() {
        bail!("提交发现新出现的输出且待提交文件仍在，无法证明归属；保留双方文件及事务，请改名或核对处理记录");
    }
    // A workflow/consent-only change can produce identical PDF bytes. The old
    // PDF + manifest must still be archived before replacing the manifest; a
    // matching target hash alone does not mean this transaction committed.
    let archive_pending = current_hash.is_some()
        && current_hash == transaction.previous_hash
        && backup.as_ref().is_some_and(|path| !path.exists());
    if current_hash.as_deref() != Some(target_hash) || archive_pending {
        if current_hash.is_some() && current_hash != transaction.previous_hash {
            bail!("事务恢复发现输出被外部修改，拒绝覆盖");
        }
        if !temporary.exists() || sha256_file(&temporary)? != target_hash {
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

fn object_dictionary(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn named(dictionary: &Dictionary, key: &[u8], value: &[u8]) -> bool {
    dictionary.get(key).ok().and_then(|o| o.as_name().ok()) == Some(value)
}

#[derive(Default)]
struct SignatureFields {
    nodes: HashSet<ObjectId>,
    widgets: HashSet<ObjectId>,
    has_signature: bool,
    needs_appearances: bool,
}

fn signature_fields(doc: &Document) -> Result<SignatureFields> {
    fn visit(
        doc: &Document,
        id: ObjectId,
        parent: Option<ObjectId>,
        inherited_type: Option<Vec<u8>>,
        depth: usize,
        fields: &mut SignatureFields,
    ) -> Result<()> {
        if depth >= 256 || !fields.nodes.insert(id) {
            bail!("PDF签名字段树循环、重复或过深，无法可靠保留页面外观");
        }
        let dictionary = doc.get_object(id)?.as_dict()?;
        if [b"A".as_slice(), b"AA", b"JS"]
            .iter()
            .any(|key| dictionary.has(key))
        {
            bail!("PDF表单字段含动作或脚本，不能作为静态签名页面合并");
        }
        if let (Some(parent), Ok(actual)) = (parent, dictionary.get(b"Parent")) {
            if actual.as_reference().ok() != Some(parent) {
                bail!("PDF签名字段Parent关系不一致，无法可靠保留外观");
            }
        }
        let field_type = match dictionary.get(b"FT") {
            Ok(value) => Some(doc.dereference(value)?.1.as_name()?.to_vec()),
            Err(_) => inherited_type,
        };
        if field_type.as_deref().is_some_and(|kind| kind != b"Sig") {
            bail!("PDF含非签名交互表单；当前仅能静态化签名外观，不能合并其他表单");
        }
        if field_type.as_deref() == Some(b"Sig") {
            fields.has_signature = true;
        }
        let children = match dictionary.get(b"Kids") {
            Ok(value) => doc.dereference(value)?.1.as_array()?.clone(),
            Err(_) => vec![],
        };
        if named(dictionary, b"Subtype", b"Widget") {
            if field_type.as_deref() != Some(b"Sig") {
                bail!("PDF控件没有可确认的签名字段类型，不能可靠静态化");
            }
            fields.widgets.insert(id);
        }
        if children.is_empty() && field_type.as_deref() != Some(b"Sig") {
            bail!("PDF含无法确认类型的表单字段，不能可靠静态化");
        }
        for child in children {
            let child = child
                .as_reference()
                .context("PDF签名字段Kids必须是间接引用")?;
            visit(doc, child, Some(id), field_type.clone(), depth + 1, fields)?;
        }
        Ok(())
    }
    let mut fields = SignatureFields::default();
    let Ok(form) = doc.catalog()?.get(b"AcroForm") else {
        return Ok(fields);
    };
    let form = doc.dereference(form)?.1.as_dict()?;
    if form.has(b"XFA") || form.has(b"CO") {
        bail!("PDF含动态或计算表单，不能仅静态化签名后合并");
    }
    if let Ok(value) = form.get(b"NeedAppearances") {
        fields.needs_appearances = doc.dereference(value)?.1.as_bool()?;
    }
    let roots = doc
        .dereference(form.get(b"Fields").context("PDF表单字段树缺失")?)?
        .1
        .as_array()?;
    for root in roots {
        visit(
            doc,
            root.as_reference().context("PDF表单Fields必须是间接引用")?,
            None,
            None,
            0,
            &mut fields,
        )?;
    }
    Ok(fields)
}

fn pdf_numbers<const N: usize>(doc: &Document, object: &Object, label: &str) -> Result<[f64; N]> {
    let array = doc.dereference(object)?.1.as_array()?;
    if array.len() != N {
        bail!("PDF签名{label}数量不正确，不能可靠保留外观");
    }
    let mut values = [0.0; N];
    for (index, object) in array.iter().enumerate() {
        let object = doc.dereference(object)?.1;
        let value = match object {
            Object::Integer(value) => *value as f64,
            Object::Real(value) => *value as f64,
            _ => bail!("PDF签名{label}不是数值，不能可靠保留外观"),
        };
        if !value.is_finite() {
            bail!("PDF签名{label}不是有限值，不能可靠保留外观");
        }
        values[index] = value;
    }
    Ok(values)
}

/// A cached appearance uses the same BBox/Matrix-to-Rect placement for Widget
/// and Stamp annotations. Keep the existing stream and geometry; never invent
/// an appearance or expand a zero-area signing widget into a visible mark.
fn signature_stamp(
    doc: &Document,
    widget: &Dictionary,
    needs_appearances: bool,
) -> Result<Option<Dictionary>> {
    let rect_object = widget
        .get(b"Rect")
        .context("PDF签名控件缺少Rect，无法确定可见外观")?;
    let rect = pdf_numbers::<4>(doc, rect_object, "控件矩形")?;
    let flags = match widget.get(b"F") {
        Ok(value) => doc.dereference(value)?.1.as_i64()?,
        Err(_) => 0,
    };
    if flags < 0 {
        bail!("PDF签名控件标志无效，不能可靠保留外观");
    }
    if rect[0] == rect[2] || rect[1] == rect[3] || flags & 2 != 0 {
        return Ok(None);
    }
    if needs_appearances {
        bail!("可见签名控件要求重建外观，无法可靠保留现有签章；请提供明确的静态副本");
    }
    let appearance = doc
        .dereference(
            widget
                .get(b"AP")
                .context("可见签名控件缺少AP外观，不能猜测或重画签章")?,
        )?
        .1
        .as_dict()?;
    let normal = appearance
        .get(b"N")
        .context("可见签名控件缺少AP.N外观，无法保留签章")?;
    let (_, normal_object) = doc.dereference(normal)?;
    let selected = if let Object::Dictionary(states) = normal_object {
        let state = doc
            .dereference(
                widget
                    .get(b"AS")
                    .context("签名外观有状态但缺少AS，无法确定应保留的签章")?,
            )?
            .1
            .as_name()?;
        states
            .get(state)
            .context("签名AS没有对应AP.N外观，无法确定应保留的签章")?
    } else {
        normal
    };
    let stream = doc
        .dereference(selected)?
        .1
        .as_stream()
        .context("签名AP.N不是外观流，无法可靠保留签章")?;
    selected
        .as_reference()
        .context("签名外观流必须是间接引用，无法可靠保留签章")?;
    if !named(&stream.dict, b"Subtype", b"Form") {
        bail!("签名外观不是Form XObject，无法可靠保留签章");
    }
    let bbox = pdf_numbers::<4>(
        doc,
        stream.dict.get(b"BBox").context("签名外观缺少BBox")?,
        "外观边界",
    )?;
    let matrix = match stream.dict.get(b"Matrix") {
        Ok(value) => pdf_numbers::<6>(doc, value, "外观变换")?,
        Err(_) => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    let corners = [
        (bbox[0], bbox[1]),
        (bbox[0], bbox[3]),
        (bbox[2], bbox[1]),
        (bbox[2], bbox[3]),
    ];
    let transformed: Vec<_> = corners
        .iter()
        .map(|(x, y)| {
            (
                matrix[0] * x + matrix[2] * y + matrix[4],
                matrix[1] * x + matrix[3] * y + matrix[5],
            )
        })
        .collect();
    let bounds = transformed.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |mut b, (x, y)| {
            b[0] = b[0].min(*x);
            b[1] = b[1].min(*y);
            b[2] = b[2].max(*x);
            b[3] = b[3].max(*y);
            b
        },
    );
    if bounds.iter().any(|n| !n.is_finite())
        || bounds[0] == bounds[2]
        || bounds[1] == bounds[3]
        || matrix[0] * matrix[3] - matrix[1] * matrix[2] == 0.0
    {
        bail!("签名外观BBox/Matrix退化，无法可靠保留签章");
    }
    let mut stamp = dictionary! {"Type"=>"Annot","Subtype"=>"Stamp","Rect"=>rect_object.clone(),"AP"=>dictionary! {"N"=>selected.clone()}};
    // AS is not needed after selecting N, but retaining it and the annotation
    // flags keeps read/print, no-zoom and no-rotate behavior explicit.
    for key in [b"AS".as_slice(), b"F", b"P", b"CA", b"C", b"Border", b"BS"] {
        if let Ok(value) = widget.get(key) {
            stamp.set(key, value.clone());
        }
    }
    Ok(Some(stamp))
}

pub(crate) fn normalize_batch_signatures(doc: &mut Document) -> Result<bool> {
    let fields = signature_fields(doc)?;
    let has_signature_object = doc.objects.values().filter_map(object_dictionary).any(|d| {
        named(d, b"Type", b"Sig") || named(d, b"Type", b"DocTimeStamp") || named(d, b"FT", b"Sig")
    });
    if !fields.has_signature && !has_signature_object {
        return Ok(false);
    }
    // Validate the original field objects before pruning. Otherwise unrelated
    // active/non-signature form data could disappear with the field tree.
    for (id, object) in &doc.objects {
        if let Some(dictionary) = object_dictionary(object) {
            if let Ok(field_type) = dictionary.get(b"FT") {
                if doc.dereference(field_type)?.1.as_name()? != b"Sig" {
                    bail!("PDF含非签名表单字段，不能仅静态化签名后合并");
                }
            }
            if named(dictionary, b"Subtype", b"Widget") && !fields.widgets.contains(id) {
                bail!("PDF控件不属于可验证的签名字段树，不能可靠静态化");
            }
        }
    }
    let mut converted = HashMap::new();
    let mut seen = HashSet::new();
    for page_id in doc.get_pages().values() {
        let annotations = {
            let page = doc.get_object(*page_id)?.as_dict()?;
            match page.get(b"Annots") {
                Ok(value) => doc.dereference(value)?.1.as_array()?.clone(),
                Err(_) => continue,
            }
        };
        let mut keep = vec![];
        for annotation in annotations {
            let dictionary = doc.dereference(&annotation)?.1.as_dict()?;
            if !named(dictionary, b"Subtype", b"Widget") {
                keep.push(annotation);
                continue;
            }
            let id = annotation
                .as_reference()
                .context("签名控件必须是间接引用，无法可靠静态化")?;
            if !fields.widgets.contains(&id) || !seen.insert(id) {
                bail!("签名控件归属不明或重复出现在页面，无法可靠保留外观");
            }
            let stamp = signature_stamp(doc, dictionary, fields.needs_appearances)?;
            if let Some(stamp) = stamp {
                let new_id = doc.add_object(stamp);
                converted.insert(id, new_id);
                keep.push(Object::Reference(new_id));
            }
        }
        doc.get_object_mut(*page_id)?
            .as_dict_mut()?
            .set("Annots", keep);
    }
    // Popup comments can refer back to their annotation. Redirect this specific
    // relationship, not arbitrary object references, to the new static stamp.
    for object in doc.objects.values_mut() {
        if let Object::Dictionary(dictionary) = object {
            if named(dictionary, b"Subtype", b"Popup") {
                if let Ok(parent) = dictionary.get(b"Parent").and_then(Object::as_reference) {
                    if let Some(new) = converted.get(&parent) {
                        dictionary.set("Parent", *new);
                    }
                }
            }
        }
    }
    let root = doc.trailer.get(b"Root")?.as_reference()?;
    let catalog = doc.get_object_mut(root)?.as_dict_mut()?;
    for key in [b"AcroForm".as_slice(), b"Perms", b"DSS"] {
        catalog.remove(key);
    }
    doc.prune_objects();
    // A reference from page content/resources to a removed validation object is
    // not a normal signature widget. Refuse it instead of leaving dangling data.
    if doc.objects.values().filter_map(object_dictionary).any(|d| {
        named(d, b"Type", b"Sig")
            || named(d, b"Type", b"DocTimeStamp")
            || named(d, b"FT", b"Sig")
            || named(d, b"Subtype", b"Widget")
    }) {
        bail!("PDF签名数据仍被页面引用，无法安全移除验证结构并保留外观");
    }
    Ok(true)
}

pub(crate) fn append_pdf(
    output: &mut Document,
    page_root: ObjectId,
    kids: &mut Vec<Object>,
    path: &Path,
    rotation: u16,
    cancel: &AtomicBool,
) -> Result<u32> {
    append_pdf_rotations(output, page_root, kids, path, rotation, &[], cancel)
}

pub(crate) fn append_pdf_rotations(
    output: &mut Document,
    page_root: ObjectId,
    kids: &mut Vec<Object>,
    path: &Path,
    rotation: u16,
    page_rotations: &[u16],
    cancel: &AtomicBool,
) -> Result<u32> {
    let source = Document::load(path).context("无法读取 PDF")?;
    append_document_rotations(
        output,
        page_root,
        kids,
        source,
        rotation,
        page_rotations,
        cancel,
    )
}

pub(crate) fn append_batch_pdf_rotations(
    output: &mut Document,
    page_root: ObjectId,
    kids: &mut Vec<Object>,
    path: &Path,
    rotation: u16,
    page_rotations: &[u16],
    cancel: &AtomicBool,
) -> Result<u32> {
    cancelled(cancel)?;
    let mut source = Document::load(path).context("无法读取 PDF")?;
    crate::generic::inspect_batch_pdf_document(&mut source)?;
    append_document_rotations(
        output,
        page_root,
        kids,
        source,
        rotation,
        page_rotations,
        cancel,
    )
}

fn append_document_rotations(
    output: &mut Document,
    page_root: ObjectId,
    kids: &mut Vec<Object>,
    mut source: Document,
    rotation: u16,
    page_rotations: &[u16],
    cancel: &AtomicBool,
) -> Result<u32> {
    if source.is_encrypted() {
        bail!("加密 PDF 不支持直接合并");
    }
    source.renumber_objects_with(output.max_id + 1);
    let pages = source.get_pages();
    if pages.is_empty() {
        bail!("PDF 没有页面");
    }
    if !page_rotations.is_empty() && page_rotations.len() != pages.len() {
        bail!("PDF逐页角度数量与源页数不符");
    }
    for (index, id) in pages.values().enumerate() {
        cancelled(cancel)?;
        let mut page = source.get_object(*id)?.as_dict()?.clone();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if !page.has(key) {
                if let Some(value) = inherited(&source, *id, key) {
                    page.set(key, value);
                }
            }
        }
        let rotation = (rotation + page_rotations.get(index).copied().unwrap_or(0)) % 360;
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

pub(crate) fn append_image(
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
            replace_existing: false,
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
                replace_existing: false,
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
            replace_existing: false,
        };
        atomic_json(&records.journal(), &txn).unwrap();
        recover_transaction(dir, "result.pdf", &records).unwrap();
        let saved: Manifest =
            serde_json::from_slice(&fs::read(records.manifest()).unwrap()).unwrap();
        assert_eq!(saved.output_sha256, hash);
    }
    #[test]
    fn first_commit_never_adopts_equal_bytes_while_pending_still_exists() {
        for central in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path();
            let output = dir.join("result.pdf");
            let records = if central {
                OutputRecords::new(&dir.join("state"), &output).unwrap()
            } else {
                OutputRecords::legacy(&output)
            };
            let pending = dir.join(".pending.pdf");
            fs::write(&pending, b"identical foreign PDF bytes").unwrap();
            let txn = Transaction {
                temporary: ".pending.pdf".into(),
                output: "result.pdf".into(),
                backup: None,
                previous_hash: None,
                manifest: manifest(sha256_file(&pending).unwrap()),
                replace_existing: false,
            };
            atomic_json(&records.journal(), &txn).unwrap();
            // A separate state/owner produces byte-for-byte the same result
            // after our journal is durable but before our rename starts.
            fs::write(&output, b"identical foreign PDF bytes").unwrap();
            fs::write(records.manifest(), b"foreign owner record").unwrap();
            assert!(recover_transaction(dir, "result.pdf", &records).is_err());
            assert_eq!(fs::read(&output).unwrap(), b"identical foreign PDF bytes");
            assert_eq!(fs::read(&pending).unwrap(), b"identical foreign PDF bytes");
            assert_eq!(
                fs::read(records.manifest()).unwrap(),
                b"foreign owner record"
            );
            assert!(records.journal().exists());
        }
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
                replace_existing: false,
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
            replace_existing: false,
        };
        atomic_json(&records.journal(), &txn).unwrap();
        assert!(recover_transaction(dir, "result.pdf", &records).is_err());
        assert_eq!(fs::read(output).unwrap(), b"external changes");
    }
}
