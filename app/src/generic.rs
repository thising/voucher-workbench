//! Generic, local-only static PDF merging. Plans are immutable snapshots;
//! source files and outputs are never accepted from a generation request.
use crate::{
    jobs::{self, AppState, GenerateRequest, Job, JobResult},
    pdf::{self, OutputRecords, RecordHash, Transaction},
    workers,
};
use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{Path as ApiPath, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use lopdf::{dictionary, Document, Object, ObjectId};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    cmp::Ordering as Comparison,
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::UNIX_EPOCH,
};

fn default_globs() -> Vec<String> {
    vec!["**/indexImg/**".into()]
}
fn default_sort() -> String {
    "natural_asc".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub root: String,
    pub output_dir: Option<String>,
    #[serde(default)]
    pub level: u8,
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    #[serde(default = "default_globs")]
    pub exclude_globs: Vec<String>,
    #[serde(default = "default_sort")]
    pub sort: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}
fn issue(severity: &str, code: &str, message: impl Into<String>, path: Option<String>) -> Issue {
    Issue {
        severity: severity.into(),
        code: code.into(),
        message: message.into(),
        path,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    pub relative_path: String,
    pub bytes: u64,
    pub modified_ms: u64,
    pub sha256: String,
    pub pages: u32,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub relative_path: String,
    pub name: String,
    pub shallow: bool,
    pub output_name: String,
    pub output_path: String,
    pub files: Vec<File>,
    pub pages: u32,
    pub issues: Vec<Issue>,
    pub empty: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Exclusion {
    pub rule: String,
    pub kind: String,
    pub matches: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub schema_version: u32,
    pub mode: String,
    pub id: String,
    pub parent_plan_id: Option<String>,
    pub created_at: String,
    pub root: String,
    pub output_dir: String,
    pub level: u8,
    pub sort: String,
    pub exclude_paths: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub groups: Vec<Group>,
    pub issues: Vec<Issue>,
    pub exclusions: Vec<Exclusion>,
    pub ignored_files: usize,
    pub empty_directories: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupOrder {
    pub group_id: String,
    pub file_ids: Vec<String>,
    pub output_name: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderRequest {
    pub groups: Vec<GroupOrder>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generate {
    group_ids: Option<Vec<String>>,
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .context("路径包含无法保存的非Unicode名称，请改名后重试")
}
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Windows directory junctions and cloud placeholders must not be followed.
pub(crate) fn checked_metadata(path: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        bail!("符号链接不在支持范围，请选择真实本地目录或排除该路径");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let attrs = meta.file_attributes();
        if attrs & 0x400 != 0 {
            bail!("目录联接、重解析点或云占位文件不在支持范围，请使用已下载的真实本地副本");
        }
        if attrs & (0x1000 | 0x40000 | 0x400000) != 0 {
            bail!("离线或云占位文件不在支持范围，请先下载到本地");
        }
    }
    Ok(meta)
}

fn is_missing(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn check_component_with(
    path: &Path,
    must_exist: bool,
    directory_required: bool,
    mut observe: impl FnMut(&Path) -> Result<fs::Metadata>,
) -> Result<()> {
    let metadata = match observe(path) {
        Ok(metadata) => Some(metadata),
        Err(first) if is_missing(&first) => match observe(path) {
            // Another merge worker may have created the shared output directory.
            // Validate its current type/attributes instead of returning an old
            // NotFound after a separate exists() query has observed its creation.
            Ok(metadata) => Some(metadata),
            Err(current) if is_missing(&current) => {
                if must_exist {
                    return Err(current).context("目录不存在，请检查资料路径");
                }
                None
            }
            Err(current) => return Err(current),
        },
        Err(error) => return Err(error),
    };
    if directory_required && metadata.is_some_and(|metadata| !metadata.is_dir()) {
        bail!("路径中间组件不是目录，请选择真实本地目录");
    }
    Ok(())
}

fn canonicalize_checked_leaf(path: &Path) -> Result<PathBuf> {
    checked_metadata(path)?;
    Ok(path.canonicalize()?)
}
fn reject_network(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if (text.starts_with("\\\\") && !text.starts_with("\\\\?\\"))
        || text.starts_with("//")
        || text.to_ascii_lowercase().starts_with("\\\\?\\unc\\")
    {
        bail!("本版本仅支持本地磁盘，请将UNC或网络共享资料复制到本地后处理");
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        if let Some(Component::Prefix(p)) = path.components().next() {
            let letter = match p.kind() {
                std::path::Prefix::Disk(c) | std::path::Prefix::VerbatimDisk(c) => c,
                _ => bail!("本版本仅支持本地磁盘路径"),
            };
            let drive = format!("{}:\\", letter as char);
            let wide: Vec<u16> = OsStr::new(&drive).encode_wide().chain(Some(0)).collect();
            if unsafe { GetDriveTypeW(wide.as_ptr()) } == 4 {
                bail!("本版本不支持映射网络盘，请复制到本地磁盘后处理");
            }
        }
    }
    Ok(())
}
pub(crate) fn absolute_checked(path: &Path, must_exist: bool) -> Result<PathBuf> {
    reject_network(path)?;
    #[cfg(windows)]
    if matches!(path.components().next(), Some(Component::Prefix(_))) && !path.is_absolute() {
        bail!("盘符路径必须包含完整根目录，例如C:\\资料；不支持C:资料这样的盘符相对路径");
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        match part {
            Component::CurDir => continue,
            // A drive prefix (especially \\?\C:) is not a complete root
            // until the next RootDir component has been appended. Querying it
            // alone returns ERROR_INVALID_FUNCTION on Windows.
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::ParentDir => {
                if !current.pop() {
                    bail!("路径超出文件系统根目录");
                }
            }
            _ => current.push(part.as_os_str()),
        }
        check_component_with(
            &current,
            must_exist,
            parts.peek().is_some(),
            checked_metadata,
        )?;
    }
    reject_network(&current)?;
    if current.exists() {
        // The leaf may have appeared after a missing observation above. Do not
        // canonicalize a newly introduced link/reparse/cloud-placeholder leaf.
        return canonicalize_checked_leaf(&current);
    }
    let parent = current.parent().context("输出目录无效")?;
    let canonical_parent = absolute_checked(parent, false)?;
    Ok(canonical_parent.join(current.file_name().context("目录名无效")?))
}

fn relative_rule(text: &str) -> Result<String> {
    let text = text.replace('\\', "/");
    let text = text.trim_end_matches('/');
    if text.is_empty()
        || text.starts_with('/')
        || text.contains(':')
        || text
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        bail!("排除规则必须是资料根之下的相对路径，不能含空段、.、..、盘符或UNC路径");
    }
    Ok(text.into())
}
pub(crate) fn glob_regex(pattern: &str) -> Result<Regex> {
    let pattern = relative_rule(pattern)?;
    let segments: Vec<_> = pattern.split('/').collect();
    let mut expression = String::from("(?i)^");
    for (index, segment) in segments.iter().enumerate() {
        let last = index + 1 == segments.len();
        if *segment == "**" {
            expression.push_str(if last { ".*" } else { "(?:[^/]+/)*" });
            continue;
        }
        if segment.contains("**") || segment.contains(['[', ']', '{', '}', '!']) {
            bail!("glob仅支持*、?和作为完整路径段的**；不支持否定、字符集或正则表达式");
        }
        for c in segment.chars() {
            match c {
                '*' => expression.push_str("[^/]*"),
                '?' => expression.push_str("[^/]"),
                _ => expression.push_str(&regex::escape(&c.to_string())),
            }
        }
        if !last {
            expression.push('/');
        }
    }
    expression.push('$');
    Regex::new(&expression).context("glob规则无效")
}

/// Compare arbitrary-length ASCII number runs, with a deterministic tie break.
pub fn natural_compare(a: &str, b: &str) -> Comparison {
    let ak = a.to_lowercase();
    let bk = b.to_lowercase();
    let ac: Vec<char> = ak.chars().collect();
    let bc: Vec<char> = bk.chars().collect();
    let (mut i, mut j) = (0, 0);
    while i < ac.len() && j < bc.len() {
        if ac[i].is_ascii_digit() && bc[j].is_ascii_digit() {
            let (start_i, start_j) = (i, j);
            while i < ac.len() && ac[i].is_ascii_digit() {
                i += 1;
            }
            while j < bc.len() && bc[j].is_ascii_digit() {
                j += 1;
            }
            let an: String = ac[start_i..i].iter().collect();
            let bn: String = bc[start_j..j].iter().collect();
            let av = an.trim_start_matches('0');
            let bv = bn.trim_start_matches('0');
            let order = av.len().cmp(&bv.len()).then_with(|| av.cmp(bv));
            if order != Comparison::Equal {
                return order;
            }
            let order = an.cmp(&bn);
            if order != Comparison::Equal {
                return order;
            }
        } else {
            let order = ac[i].cmp(&bc[j]);
            if order != Comparison::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    ac.len().cmp(&bc.len()).then_with(|| a.cmp(b))
}
fn sort_files(files: &mut [File], sort: &str) {
    files.sort_by(|a, b| match sort {
        "natural_desc" => natural_compare(&b.relative_path, &a.relative_path),
        "modified_asc" => a
            .modified_ms
            .cmp(&b.modified_ms)
            .then_with(|| natural_compare(&a.relative_path, &b.relative_path)),
        "modified_desc" => b
            .modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| natural_compare(&a.relative_path, &b.relative_path)),
        _ => natural_compare(&a.relative_path, &b.relative_path),
    });
}
pub(crate) fn output_name(raw: &str) -> Result<String> {
    // User edits are names, never paths. Default names have already joined directories.
    if raw.contains(['/', '\\']) {
        bail!("输出文件名不能包含目录路径");
    }
    let stem = if raw.to_ascii_lowercase().ends_with(".pdf") {
        &raw[..raw.len() - 4]
    } else {
        raw
    };
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let stem = stem.trim_end_matches(['.', ' ']);
    if stem.is_empty() || matches!(stem, "." | "..") {
        bail!("输出文件名不能为空");
    }
    let reserved = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    let reserved = matches!(
        reserved.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || reserved
        .strip_prefix("COM")
        .or_else(|| reserved.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    let name = format!("{}{}.pdf", if reserved { "_" } else { "" }, stem);
    if name.encode_utf16().count() > 255 {
        bail!("输出文件名超过255个UTF-16单元，请缩短名称");
    }
    Ok(name)
}

/// Reject unsupported document-level features before page import. Static page
/// annotations are restricted to text/markup and external URI links.
pub(crate) fn inspect_pdf(path: &Path) -> Result<u32> {
    inspect_pdf_bytes(&fs::read(path).context("PDF损坏或无法解析")?)
}

/// Inspect the exact bytes already fingerprinted by a scanner, without reopening
/// the source. The path and byte entry points apply the same support boundary.
pub(crate) fn inspect_pdf_bytes(bytes: &[u8]) -> Result<u32> {
    let doc = Document::load_mem(bytes).context("PDF损坏或无法解析")?;
    inspect_pdf_document(&doc)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PdfImportInfo {
    pub pages: u32,
    pub signature_notice: bool,
}

/// Batch-only page import. Signature appearances become static annotations;
/// original files and legacy import policies are unchanged.
pub fn inspect_batch_pdf_bytes(bytes: &[u8]) -> Result<PdfImportInfo> {
    let mut doc = Document::load_mem(bytes).context("PDF损坏或无法解析")?;
    inspect_batch_pdf_document(&mut doc)
}

pub fn inspect_batch_pdf(path: &Path) -> Result<PdfImportInfo> {
    let mut doc = Document::load(path).context("PDF损坏或无法解析")?;
    inspect_batch_pdf_document(&mut doc)
}

pub(crate) fn inspect_batch_pdf_document(doc: &mut Document) -> Result<PdfImportInfo> {
    inspect_pdf_document_rules(doc, true)?;
    let signature_notice = pdf::normalize_batch_signatures(doc)?;
    let pages = inspect_pdf_document(doc)?;
    Ok(PdfImportInfo {
        pages,
        signature_notice,
    })
}

fn pdf_dictionary(object: &Object) -> Option<&lopdf::Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn signature_field_state(doc: &Document) -> (bool, bool) {
    let mut has_signature_field = false;
    for dictionary in doc.objects.values().filter_map(pdf_dictionary) {
        if dictionary
            .get(b"FT")
            .ok()
            .and_then(|value| value.as_name().ok())
            != Some(b"Sig")
        {
            continue;
        }
        has_signature_field = true;
        let Some(value) = dictionary
            .get(b"V")
            .ok()
            .and_then(|value| doc.dereference(value).ok())
            .and_then(|(_, value)| value.as_dict().ok())
        else {
            continue;
        };
        let byte_range = value
            .get(b"ByteRange")
            .ok()
            .and_then(|value| doc.dereference(value).ok())
            .and_then(|(_, value)| value.as_array().ok());
        let contents = value
            .get(b"Contents")
            .ok()
            .and_then(|value| doc.dereference(value).ok())
            .and_then(|(_, value)| value.as_str().ok());
        // This identifies a populated signature structure, not a cryptographic
        // verification. An empty/preallocated signing field is a separate case.
        if byte_range.is_some_and(|range| {
            range.len() >= 4
                && range.len() % 2 == 0
                && range
                    .iter()
                    .all(|value| value.as_i64().is_ok_and(|n| n >= 0))
                && range
                    .iter()
                    .any(|value| value.as_i64().is_ok_and(|n| n > 0))
        }) && contents.is_some_and(|bytes| bytes.iter().any(|byte| *byte != 0))
        {
            return (true, true);
        }
    }
    (false, has_signature_field)
}

fn inspect_pdf_document(doc: &Document) -> Result<u32> {
    inspect_pdf_document_rules(doc, false)
}

fn inspect_pdf_document_rules(doc: &Document, signature_source: bool) -> Result<u32> {
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        bail!("加密PDF不支持，请提供未加密的静态副本");
    }
    let (has_populated_signature, has_signature_field) = signature_field_state(doc);
    if has_populated_signature && !signature_source {
        bail!("PDF含已填写的数字签名字段（未验证签名有效性）；当前合并无法保持原数字签名验证，原件未被改动。请移除此文件，或使用由您另行确认的静态副本");
    }
    let catalog = doc.catalog()?;
    if catalog.has(b"AcroForm") && !signature_source {
        if has_signature_field {
            bail!("PDF含未填写或无法验证的签名字段；当前合并不支持保留此字段，原件未被改动。请移除此文件，或使用由您另行确认的静态副本");
        }
        bail!("PDF含交互表单；当前合并无法保留表单功能，原件未被改动。请移除此文件，或使用由您另行确认的静态副本");
    }
    for (key, message) in [
        (
            b"OpenAction".as_slice(),
            "PDF含打开时动作，当前仅支持不含自动动作的静态副本",
        ),
        (b"AA", "PDF含文档自动动作，当前仅支持不含自动动作的静态副本"),
        (
            b"Collection",
            "PDF为文档集合，当前不支持合并，请提供普通静态副本",
        ),
        (
            b"OCProperties",
            "PDF含可选图层，当前合并无法保留其文档级图层设置，请提供普通静态副本",
        ),
        (
            b"OutputIntents",
            "PDF含文档级色彩输出意图，当前合并无法保留该设置，请提供普通静态副本",
        ),
        (
            b"URI",
            "PDF含文档级链接基地址，当前合并可能改变相对链接目标，请提供普通静态副本",
        ),
        (b"AF", "PDF含关联附件，当前仅支持不含附件的普通静态副本"),
    ] {
        if catalog.has(key) {
            bail!(message);
        }
    }
    if let Ok(names) = catalog.get(b"Names") {
        let (_, names) = doc.dereference(names)?;
        let names = names.as_dict()?;
        if names.has(b"JavaScript") || names.has(b"EmbeddedFiles") {
            bail!("PDF含脚本或嵌入文件，请提供普通静态副本");
        }
    }
    for d in doc.objects.values().filter_map(pdf_dictionary) {
        let named = |key: &[u8], value: &[u8]| {
            d.get(key).ok().and_then(|o| o.as_name().ok()) == Some(value)
        };
        if (named(b"Type", b"Sig") || named(b"FT", b"Sig")) && !signature_source {
            bail!("PDF含签名对象或签名字段；当前合并无法保留原数字签名验证，原件未被改动。请移除此文件，或使用由您另行确认的静态副本");
        }
        if named(b"Subtype", b"Widget") && !signature_source {
            bail!("PDF含交互表单控件，当前合并无法保留表单功能，请提供另行确认的静态副本");
        }
        if named(b"S", b"JavaScript")
            || d.has(b"JS")
            || named(b"Type", b"EmbeddedFile")
            || named(b"Type", b"OCG")
            || named(b"Type", b"OCMD")
            || d.has(b"EF")
            || d.has(b"AF")
        {
            bail!("PDF含数字签名、交互表单、脚本、嵌入文件或可选图层，请提供普通静态副本");
        }
    }
    let ordered = validate_page_tree(doc)?;
    let pages = doc.get_pages();
    if pages.values().copied().collect::<Vec<_>>() != ordered {
        bail!("PDF页树与解析页面列表不一致，拒绝可能漏页的合并");
    }
    if pages.is_empty() {
        bail!("PDF没有页面");
    }
    for id in pages.values() {
        let page = doc.get_object(*id)?.as_dict()?;
        if page.has(b"AA") {
            bail!("PDF页面含自动动作，当前不支持");
        }
        if let Ok(annots) = page.get(b"Annots") {
            let (_, annots) = doc.dereference(annots)?;
            for annotation in annots.as_array()? {
                let (_, annotation) = doc.dereference(annotation)?;
                let d = annotation.as_dict()?;
                let subtype = d.get(b"Subtype")?.as_name()?;
                if !(signature_source && subtype == b"Widget")
                    && !matches!(
                        subtype,
                        b"Text"
                            | b"FreeText"
                            | b"Line"
                            | b"Square"
                            | b"Circle"
                            | b"Polygon"
                            | b"PolyLine"
                            | b"Highlight"
                            | b"Underline"
                            | b"Squiggly"
                            | b"StrikeOut"
                            | b"Stamp"
                            | b"Caret"
                            | b"Ink"
                            | b"Popup"
                            | b"Link"
                    )
                {
                    bail!("PDF包含当前不支持的页面注释，请转为普通静态副本");
                }
                if d.has(b"AA") || d.has(b"Dest") {
                    bail!("PDF注释含自动动作或文档内跳转，当前不支持");
                }
                if let Ok(action) = d.get(b"A") {
                    let (_, action) = doc.dereference(action)?;
                    let action = action.as_dict()?;
                    if action.get(b"S")?.as_name()? != b"URI" || action.has(b"Next") {
                        bail!("PDF注释含非URI动作，当前不支持");
                    }
                    let (_, uri) = doc.dereference(action.get(b"URI")?)?;
                    let uri = uri.as_str().context("PDF链接URI不是文本")?;
                    let colon = uri
                        .iter()
                        .position(|b| *b == b':')
                        .context("PDF相对URI链接会因合并改变目标，请提供绝对链接的静态副本")?;
                    if colon == 0
                        || !uri[0].is_ascii_alphabetic()
                        || !uri[..colon]
                            .iter()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
                    {
                        bail!("PDF链接URI没有有效的绝对协议，当前不支持");
                    }
                    if matches!(
                        String::from_utf8_lossy(&uri[..colon])
                            .to_ascii_lowercase()
                            .as_str(),
                        "javascript" | "vbscript" | "data"
                    ) {
                        bail!("PDF链接含脚本或内嵌数据，当前仅支持普通外部绝对URI");
                    }
                }
            }
        }
    }
    u32::try_from(pages.len()).context("PDF页数超过支持范围")
}

fn validate_page_tree(doc: &Document) -> Result<Vec<ObjectId>> {
    fn visit(
        doc: &Document,
        id: ObjectId,
        parent: Option<ObjectId>,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        pages: &mut Vec<ObjectId>,
    ) -> Result<u32> {
        if depth >= 256 {
            bail!("PDF页树超过256级，拒绝可能不完整的合并");
        }
        if !seen.insert(id) {
            bail!("PDF页树存在循环或重复页面/目录引用");
        }
        let dictionary = doc
            .objects
            .get(&id)
            .context("PDF页树引用了不存在的对象")?
            .as_dict()
            .context("PDF页树引用的对象不是字典")?;
        if let Some(parent) = parent {
            if dictionary
                .get(b"Parent")
                .ok()
                .and_then(|p| p.as_reference().ok())
                != Some(parent)
            {
                bail!("PDF页树Parent关系不一致");
            }
        } else if dictionary.has(b"Parent") {
            bail!("PDF根页树不应有Parent");
        }
        match dictionary.get(b"Type")?.as_name()? {
            b"Page" => {
                if dictionary.has(b"Kids") {
                    bail!("PDF页面节点含非法Kids");
                }
                pages.push(id);
                Ok(1)
            }
            b"Pages" => {
                let count = dictionary
                    .get(b"Count")?
                    .as_i64()
                    .context("PDF页树Count不是整数")?;
                if !(0..=u32::MAX as i64).contains(&count) {
                    bail!("PDF页树Count超出范围");
                }
                let (_, kids) = doc.dereference(dictionary.get(b"Kids")?)?;
                let mut actual = 0u32;
                for child in kids.as_array().context("PDF页树Kids不是数组")? {
                    let child_id = child.as_reference().context("PDF页树Kids必须是间接引用")?;
                    actual = actual
                        .checked_add(visit(doc, child_id, Some(id), depth + 1, seen, pages)?)
                        .context("PDF总页数超过支持范围")?;
                }
                if actual as i64 != count {
                    bail!("PDF页树Count与完整Kids页面数不一致：声明{count}，实际{actual}");
                }
                Ok(actual)
            }
            _ => bail!("PDF页树包含非Page/Pages节点"),
        }
    }
    let root = doc
        .catalog()?
        .get(b"Pages")?
        .as_reference()
        .context("PDF Catalog.Pages必须是间接引用")?;
    if doc
        .objects
        .get(&root)
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Type").ok())
        .and_then(|o| o.as_name().ok())
        != Some(b"Pages")
    {
        bail!("PDF Catalog.Pages不是Pages节点");
    }
    let mut pages = Vec::new();
    visit(doc, root, None, 0, &mut HashSet::new(), &mut pages)?;
    Ok(pages)
}

fn group_key(relative_file: &str, level: u8) -> (String, bool) {
    let directories: Vec<_> = relative_file
        .rsplit_once('/')
        .map(|(d, _)| d.split('/').collect())
        .unwrap_or_default();
    let shallow = directories.len() < level as usize;
    (
        directories[..directories.len().min(level as usize)].join("/"),
        shallow,
    )
}

struct Scanner {
    root: PathBuf,
    output: PathBuf,
    level: u8,
    explicit: Vec<String>,
    globs: Vec<Regex>,
    exclusions: Vec<Exclusion>,
    groups: BTreeMap<String, Vec<File>>,
    directories: Vec<String>,
    issues: Vec<Issue>,
    ignored: usize,
}
impl Scanner {
    fn excluded(&mut self, relative: &str, directory: bool) -> bool {
        let key = relative.to_lowercase();
        let mut excluded = false;
        for automatic in self.exclusions.iter_mut().filter(|e| e.kind == "state") {
            let rule = automatic.rule.to_lowercase();
            if key == rule || key.starts_with(&format!("{rule}/")) {
                automatic.matches.push(if directory {
                    format!("{relative}/")
                } else {
                    relative.into()
                });
                excluded = true;
            }
        }
        for (index, rule) in self.explicit.iter().enumerate() {
            let rule = rule.to_lowercase();
            if key == rule || key.starts_with(&format!("{rule}/")) {
                self.exclusions[index].matches.push(if directory {
                    format!("{relative}/")
                } else {
                    relative.into()
                });
                excluded = true;
            }
        }
        for (index, rule) in self.globs.iter().enumerate() {
            if rule.is_match(relative) || (directory && rule.is_match(&format!("{relative}/"))) {
                self.exclusions[self.explicit.len() + index]
                    .matches
                    .push(if directory {
                        format!("{relative}/")
                    } else {
                        relative.into()
                    });
                excluded = true;
            }
        }
        excluded
    }
    fn walk(&mut self, relative: &str) -> Result<()> {
        let directory = self.root.join(relative);
        checked_metadata(&directory)?;
        self.directories.push(relative.into());
        if relative.split('/').filter(|s| !s.is_empty()).count() == self.level as usize {
            self.groups.entry(relative.into()).or_default();
        }
        let mut entries: Vec<_> = fs::read_dir(&directory)
            .with_context(|| {
                format!(
                    "无法读取目录：{}，请检查权限后重新扫描",
                    directory.display()
                )
            })?
            .collect::<std::io::Result<_>>()?;
        entries.sort_by(|a, b| {
            natural_compare(
                &a.file_name().to_string_lossy(),
                &b.file_name().to_string_lossy(),
            )
        });
        for entry in entries {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("目录含非Unicode文件名，请改名后重新扫描"))?;
            let rel = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let meta = fs::symlink_metadata(entry.path())?;
            let is_dir = meta.is_dir();
            if path_key(&entry.path()) == path_key(&self.output) {
                if let Some(e) = self.exclusions.iter_mut().find(|e| e.kind == "output") {
                    e.matches.push(format!("{rel}/"));
                }
                continue;
            }
            if self.excluded(&rel, is_dir) {
                continue;
            }
            if let Err(error) = checked_metadata(&entry.path()) {
                self.issues.push(issue(
                    "warning",
                    "unsupported_path",
                    format!("未纳入：{error:#}"),
                    Some(rel),
                ));
                self.ignored += 1;
                continue;
            }
            if is_dir {
                self.walk(&rel)?;
                continue;
            }
            if !meta.is_file() || !name.to_ascii_lowercase().ends_with(".pdf") {
                self.ignored += 1;
                continue;
            }
            let (key, _) = group_key(&rel, self.level);
            let mut file = File {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                relative_path: rel,
                bytes: meta.len(),
                modified_ms: meta.modified()?.duration_since(UNIX_EPOCH)?.as_millis() as u64,
                sha256: String::new(),
                pages: 0,
                error: None,
            };
            let result = (|| -> Result<()> {
                file.sha256 = pdf::sha256_file(&entry.path())?;
                file.pages = inspect_pdf(&entry.path())?;
                let after = fs::metadata(entry.path())?;
                if after.len() != meta.len()
                    || after.modified()? != meta.modified()?
                    || pdf::sha256_file(&entry.path())? != file.sha256
                {
                    bail!("源文件在扫描时变化，请重新扫描");
                }
                Ok(())
            })();
            if let Err(error) = result {
                file.error = Some(format!("{error:#}"));
            }
            self.groups.entry(key).or_default().push(file);
        }
        Ok(())
    }
}

pub fn scan(request: ScanRequest) -> Result<Plan> {
    scan_with_state(request, None)
}
fn scan_with_state(request: ScanRequest, state_dir: Option<&Path>) -> Result<Plan> {
    if request.level > 32 {
        bail!("合并层级必须是0到32的整数");
    }
    if !matches!(
        request.sort.as_str(),
        "natural_asc" | "natural_desc" | "modified_asc" | "modified_desc"
    ) {
        bail!("不支持的排序方式");
    }
    if request.root.trim().is_empty() {
        bail!("请选择资料目录");
    }
    let root = absolute_checked(Path::new(&request.root), true)?;
    if !checked_metadata(&root)?.is_dir() {
        bail!("资料路径必须是目录");
    }
    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .context("不能以磁盘根目录作为资料目录")?
        .to_string();
    let output = match request
        .output_dir
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    {
        Some(path) => absolute_checked(Path::new(path), false)?,
        None => root
            .parent()
            .context("资料目录没有上级目录")?
            .join(format!("{root_name}-合并PDF")),
    };
    if path_key(&root) == path_key(&output) {
        bail!("输出目录不能与资料目录相同，请选择其他目录");
    }
    if output.exists() && !checked_metadata(&output)?.is_dir() {
        bail!("输出位置已存在且不是目录");
    }
    let explicit = request
        .exclude_paths
        .iter()
        .map(|p| relative_rule(p))
        .collect::<Result<Vec<_>>>()?;
    let globs = request
        .exclude_globs
        .iter()
        .map(|p| glob_regex(p))
        .collect::<Result<Vec<_>>>()?;
    let mut exclusions: Vec<_> = explicit
        .iter()
        .map(|s| Exclusion {
            rule: s.clone(),
            kind: "path".into(),
            matches: vec![],
        })
        .chain(request.exclude_globs.iter().map(|s| Exclusion {
            rule: s.clone(),
            kind: "glob".into(),
            matches: vec![],
        }))
        .collect();
    if output.starts_with(&root) {
        exclusions.push(Exclusion {
            rule: path_text(output.strip_prefix(&root)?)?.replace('\\', "/"),
            kind: "output".into(),
            matches: vec![],
        });
    }
    if let Some(state_dir) = state_dir {
        let state_dir = absolute_checked(state_dir, true)?;
        if root.starts_with(&state_dir) {
            bail!("不能将程序处理记录目录作为资料目录，请选择业务资料目录");
        }
        if let Ok(relative) = state_dir.strip_prefix(&root) {
            exclusions.push(Exclusion {
                rule: path_text(relative)?.replace('\\', "/"),
                kind: "state".into(),
                matches: vec![],
            });
        }
    }
    let mut scanner = Scanner {
        root: root.clone(),
        output: output.clone(),
        level: request.level,
        explicit,
        globs,
        exclusions,
        groups: BTreeMap::new(),
        directories: vec![],
        issues: vec![],
        ignored: 0,
    };
    scanner.walk("")?;
    let included: Vec<_> = scanner
        .groups
        .values()
        .flat_map(|files| files.iter().map(|f| f.relative_path.clone()))
        .collect();
    let empty_directories = scanner
        .directories
        .iter()
        .filter(|dir| {
            !included
                .iter()
                .any(|file| dir.is_empty() || file.starts_with(&format!("{dir}/")))
        })
        .cloned()
        .collect();
    let mut groups = Vec::new();
    for (relative, mut files) in scanner.groups {
        sort_files(&mut files, &request.sort);
        let name = if relative.is_empty() {
            root_name.clone()
        } else {
            relative.clone()
        };
        let default_name = if relative.is_empty() {
            root_name.clone()
        } else {
            relative.replace('/', "__")
        };
        let mut issues = vec![];
        let output_name = match output_name(&default_name) {
            Ok(name) => name,
            Err(e) => {
                issues.push(issue(
                    "error",
                    "output_name",
                    format!("{e:#}"),
                    Some(relative.clone()),
                ));
                format!("invalid-{}.pdf", uuid::Uuid::new_v4())
            }
        };
        for file in &files {
            if let Some(error) = &file.error {
                issues.push(issue(
                    "error",
                    "pdf_error",
                    error.clone(),
                    Some(file.relative_path.clone()),
                ));
            }
        }
        let pages = files.iter().try_fold(0u32, |sum, f| {
            sum.checked_add(f.pages).context("总页数超过支持范围")
        })?;
        groups.push(Group {
            id: uuid::Uuid::new_v4().to_string(),
            relative_path: relative.clone(),
            name,
            shallow: request.level > 0
                && relative.split('/').filter(|s| !s.is_empty()).count() < request.level as usize,
            output_path: path_text(&output.join(&output_name))?,
            output_name,
            empty: files.is_empty(),
            files,
            pages,
            issues,
        });
    }
    groups.sort_by(|a, b| natural_compare(&a.relative_path, &b.relative_path));
    let mut plan = Plan {
        schema_version: 1,
        mode: "generic_pdf".into(),
        id: uuid::Uuid::new_v4().to_string(),
        parent_plan_id: None,
        created_at: jobs::now(),
        root: path_text(&root)?,
        output_dir: path_text(&output)?,
        level: request.level,
        sort: request.sort,
        exclude_paths: scanner.explicit,
        exclude_globs: request.exclude_globs,
        groups,
        issues: scanner.issues,
        exclusions: scanner.exclusions,
        ignored_files: scanner.ignored,
        empty_directories,
    };
    refresh_output_issues(&mut plan)?;
    for e in &plan.exclusions {
        if e.matches.is_empty() {
            plan.issues.push(issue(
                "warning",
                "exclude_no_match",
                format!("排除规则没有命中：{}", e.rule),
                None,
            ));
        }
    }
    Ok(plan)
}

fn refresh_output_issues(plan: &mut Plan) -> Result<()> {
    let mut names: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, group) in plan.groups.iter_mut().enumerate() {
        group.issues.retain(|i| {
            !matches!(
                i.code.as_str(),
                "output_collision" | "output_is_source" | "output_existing"
            )
        });
        group.output_path = path_text(&Path::new(&plan.output_dir).join(&group.output_name))?;
        names
            .entry(group.output_name.to_lowercase())
            .or_default()
            .push(index);
    }
    let sources: HashSet<_> = plan
        .groups
        .iter()
        .flat_map(|g| g.files.iter())
        .map(|f| path_key(&Path::new(&plan.root).join(&f.relative_path)))
        .collect();
    for group in &mut plan.groups {
        if sources.contains(&path_key(Path::new(&group.output_path))) {
            group.issues.push(issue(
                "error",
                "output_is_source",
                "输出目标与源文件相同，请改名或更换输出目录",
                Some(group.output_path.clone()),
            ));
        }
    }
    for indices in names.values().filter(|v| v.len() > 1) {
        let labels = indices
            .iter()
            .map(|i| plan.groups[*i].name.clone())
            .collect::<Vec<_>>()
            .join("、");
        for index in indices {
            plan.groups[*index].issues.push(issue(
                "error",
                "output_collision",
                format!("输出文件名冲突：{labels}，请逐组改名"),
                None,
            ));
        }
    }
    Ok(())
}

pub fn inspect_existing_outputs(state_dir: &Path, plan: &mut Plan) -> Result<()> {
    for group in &mut plan.groups {
        group.issues.retain(|i| i.code != "output_existing");
        let path = Path::new(&group.output_path);
        if fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            continue;
        }
        let check = (|| -> Result<()> {
            checked_optional(path)?;
            // A pending journal is recovered by generation. Do not overwrite
            // its record in a preview or misclassify a missing manifest.
            let record = pdf::manifest_path(state_dir, path)?;
            if record.with_file_name("transaction.json").exists() {
                return Ok(());
            }
            let old: Manifest =
                serde_json::from_slice(&fs::read(record).context("存在来源不明的同名PDF")?)
                    .context("同名输出的记录损坏或属于其他模式")?;
            if old.mode != "generic_pdf"
                || old.producer != "voucher-workbench"
                || old.root != plan.root
                || old.group_key != group.relative_path
                || old.output_name != group.output_name
                || path_key(Path::new(&old.output_path)) != path_key(path)
                || pdf::sha256_file(path)? != old.output_sha256
            {
                bail!("已有PDF被修改或不属于此分组");
            }
            Ok(())
        })();
        if let Err(error) = check {
            group.issues.push(issue(
                "error",
                "output_existing",
                format!("{error:#}；拒绝覆盖，请改名或更换输出目录"),
                Some(group.output_path.clone()),
            ));
        }
    }
    Ok(())
}

pub fn reorder(original: &Plan, request: OrderRequest) -> Result<Plan> {
    if request.groups.is_empty() {
        bail!("未提供要更新的分组");
    }
    let mut plan = original.clone();
    let mut seen = HashSet::new();
    for order in request.groups {
        if !seen.insert(order.group_id.clone()) {
            bail!("分组ID重复");
        }
        let group = plan
            .groups
            .iter_mut()
            .find(|g| g.id == order.group_id)
            .context("未知分组ID")?;
        let expected: HashSet<_> = group.files.iter().map(|f| f.id.clone()).collect();
        let actual: HashSet<_> = order.file_ids.iter().cloned().collect();
        if actual != expected || order.file_ids.len() != expected.len() {
            bail!("手动顺序必须包含该组全部文件ID恰好一次，不能跨组、重复或遗漏");
        }
        let mut files: HashMap<_, _> = group.files.drain(..).map(|f| (f.id.clone(), f)).collect();
        group.files = order
            .file_ids
            .iter()
            .map(|id| files.remove(id).unwrap())
            .collect();
        if let Some(name) = order.output_name {
            group.output_name = output_name(&name)?;
            group.issues.retain(|i| i.code != "output_name");
        }
    }
    plan.parent_plan_id = Some(original.id.clone());
    plan.id = uuid::Uuid::new_v4().to_string();
    plan.created_at = jobs::now();
    plan.sort = "manual".into();
    refresh_output_issues(&mut plan)?;
    Ok(plan)
}

pub fn validate_generation(plan: &Plan, request: &GenerateRequest) -> Result<u32> {
    if plan.schema_version != 1 || plan.mode != "generic_pdf" {
        bail!("通用计划版本不受支持，请重新扫描");
    }
    if request.allow_unreviewed {
        bail!("通用PDF模式不使用方向复核参数");
    }
    if plan.issues.iter().any(|i| i.severity == "error") {
        bail!("资料扫描存在错误，请处理后重新扫描");
    }
    if let Some(ids) = &request.voucher_ids {
        let unique: HashSet<_> = ids.iter().collect();
        if ids.is_empty()
            || unique.len() != ids.len()
            || ids
                .iter()
                .any(|id| !plan.groups.iter().any(|g| &g.id == id))
        {
            bail!("请选择本计划中的有效分组，分组ID不能重复");
        }
    }
    let selected: Vec<_> = plan
        .groups
        .iter()
        .filter(|g| {
            request
                .voucher_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&g.id))
        })
        .collect();
    if selected.is_empty() || selected.iter().all(|g| g.empty) {
        bail!("没有可合并的PDF，请检查层级和排除规则");
    }
    if selected
        .iter()
        .any(|g| g.empty || g.issues.iter().any(|i| i.severity == "error"))
    {
        bail!("所选分组含空组或错误，请只选择预览中可生成的分组");
    }
    selected.iter().try_fold(0u32, |sum, g| {
        sum.checked_add(g.pages).context("总页数超过支持范围")
    })
}

fn check_file(plan: &Plan, file: &File) -> Result<PathBuf> {
    check_file_content(plan, file).with_context(|| {
        format!(
            "来源PDF无法访问或已变化，请检查该文件并重新扫描：{}",
            file.relative_path
        )
    })
}

fn check_file_content(plan: &Plan, file: &File) -> Result<PathBuf> {
    let root = absolute_checked(Path::new(&plan.root), true)?;
    if path_key(&root) != path_key(Path::new(&plan.root)) {
        bail!("资料目录已变化，请重新扫描");
    }
    let path = pdf::safe_source(&root, &file.relative_path)?;
    // safe_source rejects symlinks; this additionally rejects Windows reparse points.
    let mut current = root;
    for part in Path::new(&file.relative_path).components() {
        current.push(part.as_os_str());
        checked_metadata(&current)?;
    }
    let meta = checked_metadata(&path)?;
    let ms = meta.modified()?.duration_since(UNIX_EPOCH)?.as_millis() as u64;
    if !meta.is_file()
        || meta.len() != file.bytes
        || ms != file.modified_ms
        || file.sha256.is_empty()
        || pdf::sha256_file(&path)? != file.sha256
    {
        bail!("源文件已变化，请重新扫描：{}", file.relative_path);
    }
    Ok(path)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mapping {
    pub source: String,
    pub sha256: String,
    pub pages: u32,
    pub first_page: u32,
    pub last_page: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub producer: String,
    pub mode: String,
    pub version: String,
    pub created_at: String,
    pub plan_id: String,
    pub root: String,
    pub group_key: String,
    pub output_path: String,
    pub output_name: String,
    pub pages: u32,
    pub output_sha256: String,
    pub sources: Vec<Mapping>,
}
impl RecordHash for Manifest {
    fn output_hash(&self) -> &str {
        &self.output_sha256
    }
}
pub struct BuildResult {
    pub path: PathBuf,
    pub manifest: Manifest,
    pub skipped: bool,
}

fn prepare_output(plan: &Plan) -> Result<PathBuf> {
    let directory = absolute_checked(Path::new(&plan.output_dir), false)?;
    if path_key(&directory) != path_key(Path::new(&plan.output_dir))
        || path_key(&directory) == path_key(Path::new(&plan.root))
    {
        bail!("输出路径已经变化或与资料目录相同，请重新扫描");
    }
    fs::create_dir_all(&directory).context("无法创建输出目录，请选择有写入权限的本地目录")?;
    absolute_checked(&directory, true)?;
    // A same-directory write/rename probe also catches read-only and path limits.
    let probe = directory.join(format!(".{}.write-test", uuid::Uuid::new_v4()));
    let moved = probe.with_extension("rename-test");
    let test = (|| -> Result<()> {
        let f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&probe, &moved)?;
        fs::remove_file(&moved)?;
        Ok(())
    })();
    if test.is_err() {
        let _ = fs::remove_file(&probe);
        let _ = fs::remove_file(&moved);
    }
    test.context("输出目录不可写或无法重命名，请检查权限、磁盘空间和路径长度")?;
    Ok(directory)
}

pub fn build_group(
    state_dir: &Path,
    plan: &Plan,
    group: &Group,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u32),
) -> Result<BuildResult> {
    if cancel.load(Ordering::Acquire) {
        bail!("已取消");
    }
    if group.empty || group.files.is_empty() || group.issues.iter().any(|i| i.severity == "error") {
        bail!("分组为空或存在预览错误");
    }
    if output_name(&group.output_name)? != group.output_name {
        bail!("输出文件名不是已验证名称");
    }
    let directory = prepare_output(plan)?;
    let path = pdf::checked_output_child(&directory, &group.output_name)?;
    if path_key(&path) != path_key(Path::new(&group.output_path)) {
        bail!("输出路径与计划不符");
    }
    checked_optional(&path)?;
    // Detect case-only existing targets even on a case-sensitive build host.
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().to_lowercase() == group.output_name.to_lowercase()
            && entry.path() != path
        {
            bail!("输出目录中有仅大小写不同的同名文件，拒绝覆盖，请改名或换目录");
        }
    }
    for file in plan.groups.iter().flat_map(|g| &g.files) {
        if path_key(&Path::new(&plan.root).join(&file.relative_path)) == path_key(&path) {
            bail!("输出目标与源文件相同，拒绝覆盖");
        }
    }
    let records = OutputRecords::new(state_dir, &path)?;
    // Deserializing a generic manifest is intentionally strict: a voucher
    // journal can never be recovered/adopted through the generic path.
    pdf::recover_record_transaction::<Manifest>(&directory, &group.output_name, &records)?;
    let mut mappings = Vec::new();
    let mut next = 1u32;
    for file in &group.files {
        if file.error.is_some() || file.pages == 0 {
            bail!("来源PDF无法合并：{}", file.relative_path);
        }
        check_file(plan, file)?;
        let last = next
            .checked_add(file.pages - 1)
            .context("页数超过支持范围")?;
        mappings.push(Mapping {
            source: file.relative_path.clone(),
            sha256: file.sha256.clone(),
            pages: file.pages,
            first_page: next,
            last_page: last,
        });
        next = last.checked_add(1).context("页数超过支持范围")?;
    }
    let mut manifest = Manifest {
        schema_version: 1,
        producer: "voucher-workbench".into(),
        mode: "generic_pdf".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        created_at: jobs::now(),
        plan_id: plan.id.clone(),
        root: plan.root.clone(),
        group_key: group.relative_path.clone(),
        output_path: path_text(&path)?,
        output_name: group.output_name.clone(),
        pages: next - 1,
        output_sha256: String::new(),
        sources: mappings,
    };
    let previous_hash = if path.exists() {
        let old: Manifest = serde_json::from_slice(
            &fs::read(records.manifest())
                .context("输出已存在且无通用模式生成记录，拒绝覆盖；请改名或换目录")?,
        )
        .context("输出记录不属于通用模式或已损坏，拒绝覆盖")?;
        if old.producer != "voucher-workbench"
            || old.schema_version != 1
            || old.mode != "generic_pdf"
            || old.root != plan.root
            || old.group_key != group.relative_path
            || old.output_name != group.output_name
            || path_key(Path::new(&old.output_path)) != path_key(&path)
            || pdf::sha256_file(&path)? != old.output_sha256
        {
            bail!("已有输出被修改或不属于此分组，拒绝覆盖；请改名或换目录");
        }
        if old.sources == manifest.sources {
            // Store a task-specific snapshot even when bytes are reused.
            manifest.output_sha256 = old.output_sha256;
            progress(manifest.pages);
            return Ok(BuildResult {
                path,
                manifest,
                skipped: true,
            });
        }
        Some(old.output_sha256)
    } else {
        None
    };
    let temporary = directory.join(format!(".{}.partial.pdf", uuid::Uuid::new_v4()));
    let result = (|| -> Result<BuildResult> {
        let mut doc = Document::with_version("1.7");
        let root_pages = doc.new_object_id();
        let mut kids = Vec::new();
        for (file, mapping) in group.files.iter().zip(&manifest.sources) {
            if cancel.load(Ordering::Acquire) {
                bail!("已取消");
            }
            let source = check_file(plan, file)?;
            if inspect_pdf(&source)? != file.pages {
                bail!("来源PDF页数发生变化，请重新扫描");
            }
            let pages = pdf::append_pdf(&mut doc, root_pages, &mut kids, &source, 0, cancel)?;
            check_file(plan, file)?;
            if pages != mapping.pages {
                bail!("来源PDF页数与计划不符，请重新扫描");
            }
            progress(pages);
        }
        doc.objects.insert(
            root_pages,
            Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>manifest.pages}),
        );
        let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root_pages});
        doc.trailer.set("Root", catalog);
        doc.prune_objects();
        doc.save(&temporary)
            .context("临时PDF写入失败，请检查磁盘空间和目录权限")?
            .sync_all()?;
        if cancel.load(Ordering::Acquire) {
            bail!("已取消");
        }
        if Document::load(&temporary)?.get_pages().len() as u32 != manifest.pages {
            bail!("输出页数校验失败");
        }
        manifest.output_sha256 = pdf::sha256_file(&temporary)?;
        // Do not adopt an external edit that happened while we were assembling.
        let current = if path.exists() {
            checked_optional(&path)?;
            Some(pdf::sha256_file(&path)?)
        } else {
            None
        };
        if current != previous_hash {
            bail!("输出在生成期间被修改，拒绝覆盖，请核对后重试");
        }
        if cancel.load(Ordering::Acquire) {
            bail!("已取消");
        }
        let transaction = Transaction {
            temporary: temporary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            output: group.output_name.clone(),
            backup: previous_hash
                .as_ref()
                .map(|_| format!(".history/{}/{}", uuid::Uuid::new_v4(), group.output_name)),
            previous_hash,
            manifest: manifest.clone(),
            replace_existing: false,
        };
        if let Some(backup) = &transaction.backup {
            pdf::checked_output_child(&directory, backup)?;
        }
        pdf::atomic_json(&records.journal(), &transaction)?;
        pdf::recover_record_transaction::<Manifest>(&directory, &group.output_name, &records)?;
        Ok(BuildResult {
            path: path.clone(),
            manifest: manifest.clone(),
            skipped: false,
        })
    })();
    if result.is_err() && !records.journal().exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn checked_optional(path: &Path) -> Result<()> {
    match checked_metadata(path) {
        Ok(meta) if !meta.is_file() => bail!("输出目标已存在且不是普通文件，请改名或更换目录"),
        Ok(_) => Ok(()),
        Err(_)
            if fs::symlink_metadata(path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn execute(state: &AppState, job: &Job, cancel: &AtomicBool) -> Result<()> {
    let plan = state.generic_plan(&job.plan_id)?;
    let work: Vec<_> = plan
        .groups
        .iter()
        .filter(|g| {
            job.request
                .voucher_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&g.id))
        })
        .collect();
    let report_dir = state.dir.join("reports").join(&job.id);
    fs::create_dir_all(&report_dir)?;
    // Freeze configuration before work begins, not when the report is downloaded.
    pdf::atomic_json(&report_dir.join("generic-plan.json"), &plan)?;
    let positions: HashMap<_, _> = work
        .iter()
        .enumerate()
        .map(|(i, g)| (g.id.clone(), i))
        .collect();
    let mut accounted = vec![0u32; work.len()];
    let mut contexts = vec![(); job.threads.min(work.len())];
    workers::run(
        &work,
        &mut contexts,
        cancel,
        |_, group, progress| build_group(&state.dir, &plan, group, cancel, progress),
        |event| {
            match event {
                workers::Event::Started { index } => state.update_job(&job.id, |j| {
                    j.current = format!("最近开始：{}", work[index].name)
                })?,
                workers::Event::Progress { index, pages } => {
                    let increment = pages.min(work[index].pages.saturating_sub(accounted[index]));
                    accounted[index] += increment;
                    state.update_job(&job.id, |j| {
                        j.done = j.done.saturating_add(increment).min(j.total)
                    })?;
                }
                workers::Event::Finished { index, result } => {
                    let group = work[index];
                    if cancel.load(Ordering::Acquire) && result.is_err() {
                        return Ok(());
                    }
                    let result = match result {
                        Ok(output) => {
                            pdf::atomic_json(
                                &report_dir.join(format!("{}.json", group.id)),
                                &output.manifest,
                            )?;
                            JobResult {
                                voucher_id: group.id.clone(),
                                folder: group.name.clone(),
                                status: if output.skipped {
                                    "skipped"
                                } else {
                                    "completed"
                                }
                                .into(),
                                output_path: Some(path_text(&output.path)?),
                                pages: output.manifest.pages,
                                error: None,
                                needs_review: false,
                                rotation_change: None,
                            }
                        }
                        Err(error) => JobResult {
                            voucher_id: group.id.clone(),
                            folder: group.name.clone(),
                            status: "failed".into(),
                            output_path: None,
                            pages: 0,
                            error: Some(format!("{error:#}")),
                            needs_review: false,
                            rotation_change: None,
                        },
                    };
                    state.update_job(&job.id, |j| {
                        j.done = j
                            .done
                            .saturating_add(group.pages.saturating_sub(accounted[index]))
                            .min(j.total);
                        j.results.push(result);
                        j.results.sort_by_key(|r| positions[&r.voucher_id]);
                    })?;
                }
                workers::Event::Panicked => bail!("PDF工作线程异常，已提交成品保留，可重试继续"),
            }
            Ok(())
        },
    )
}

fn snapshot(state: &AppState, job: &Job) -> Result<Plan> {
    if job.kind != "generic_generate" {
        bail!("任务不是通用PDF合并任务");
    }
    let path = state
        .dir
        .join("reports")
        .join(&job.id)
        .join("generic-plan.json");
    let plan: Plan = serde_json::from_slice(
        &fs::read(path).context("任务预览快照尚未保存或已丢失，请检查处理记录")?,
    )?;
    if plan.id != job.plan_id || plan.mode != "generic_pdf" {
        bail!("任务预览快照与任务不符");
    }
    Ok(plan)
}
pub fn report(state: &AppState, job: &Job) -> Result<Value> {
    let plan = snapshot(state, job)?;
    let mut manifests = Vec::new();
    for result in &job.results {
        if matches!(result.status.as_str(), "completed" | "skipped") {
            let group = plan
                .groups
                .iter()
                .find(|g| g.id == result.voucher_id)
                .context("任务结果与预览不符")?;
            let manifest = read_snapshot(state, job, group)?;
            manifests.push(manifest);
        }
    }
    let successful_pages = job.results.iter().map(|r| r.pages).sum::<u32>();
    Ok(
        json!({"mode":"generic_pdf","job":job,"plan_id":plan.id,"root":plan.root,"output_dir":plan.output_dir,
        "level":plan.level,"sort":plan.sort,"exclude_paths":plan.exclude_paths,"exclude_globs":plan.exclude_globs,
        "exclusions":plan.exclusions,"issues":plan.issues,"groups":plan.groups,"successful_pages":successful_pages,"manifests":manifests}),
    )
}
fn read_snapshot(state: &AppState, job: &Job, group: &Group) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(
        state
            .dir
            .join("reports")
            .join(&job.id)
            .join(format!("{}.json", group.id)),
    )?)?;
    if manifest.mode != "generic_pdf"
        || manifest.plan_id != job.plan_id
        || manifest.group_key != group.relative_path
        || manifest.output_name != group.output_name
        || path_key(Path::new(&manifest.output_path)) != path_key(Path::new(&group.output_path))
    {
        bail!("任务输出快照与预览不符");
    }
    Ok(manifest)
}
pub fn output(state: &AppState, job: &Job, group_id: &str) -> Result<PathBuf> {
    let plan = snapshot(state, job)?;
    let result = job
        .results
        .iter()
        .find(|r| r.voucher_id == group_id && matches!(r.status.as_str(), "completed" | "skipped"))
        .context("该分组尚无成功输出")?;
    let group = plan
        .groups
        .iter()
        .find(|g| g.id == result.voucher_id)
        .context("分组不属于此任务")?;
    let manifest = read_snapshot(state, job, group)?;
    let directory = absolute_checked(Path::new(&plan.output_dir), true)?;
    let path = directory.join(&group.output_name);
    checked_optional(&path)?;
    if path_key(&path) != path_key(Path::new(&manifest.output_path))
        || pdf::sha256_file(&path)? != manifest.output_sha256
    {
        bail!("成品已变化，无法作为此历史任务下载；请查看最近任务或历史归档");
    }
    Ok(path)
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/scan", post(api_scan))
        .route("/plans", get(api_plans))
        .route("/plans/{id}", get(api_plan))
        .route("/plans/{id}/order", post(api_order))
        .route("/plans/{id}/generate", post(api_generate))
        .route("/jobs", get(api_jobs))
        .route("/plans/{id}/files/{file_id}", get(api_file))
        .route("/plans/{id}/open-output", post(api_open_output))
}
type ApiResult<T> = std::result::Result<T, ApiError>;
struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{:#}",self.0)})),
        )
            .into_response()
    }
}
async fn api_scan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScanRequest>,
) -> ApiResult<Json<Plan>> {
    let state_dir = state.dir.clone();
    let plan = tokio::task::spawn_blocking(move || {
        let mut plan = scan_with_state(request, Some(&state_dir))?;
        inspect_existing_outputs(&state_dir, &mut plan)?;
        Ok::<_, anyhow::Error>(plan)
    })
    .await??;
    Ok(Json(state.insert_generic_plan(plan)?))
}
async fn api_plans(State(state): State<Arc<AppState>>) -> Json<Vec<Plan>> {
    let mut plans: Vec<_> = state
        .data
        .lock()
        .unwrap()
        .generic_plans
        .values()
        .cloned()
        .collect();
    plans.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(plans)
}
async fn api_plan(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Plan>> {
    Ok(Json(state.generic_plan(&id)?))
}
async fn api_order(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
    Json(request): Json<OrderRequest>,
) -> ApiResult<Json<Plan>> {
    let mut plan = reorder(&state.generic_plan(&id)?, request)?;
    inspect_existing_outputs(&state.dir, &mut plan)?;
    Ok(Json(state.insert_generic_plan(plan)?))
}
async fn api_generate(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
    Json(request): Json<Generate>,
) -> ApiResult<Json<Job>> {
    Ok(Json(state.start(
        &id,
        "generic_generate",
        GenerateRequest {
            voucher_ids: request.group_ids,
            allow_unreviewed: false,
        },
    )?))
}
async fn api_jobs(State(state): State<Arc<AppState>>) -> Json<Vec<Job>> {
    let mut jobs: Vec<_> = state
        .data
        .lock()
        .unwrap()
        .jobs
        .values()
        .filter(|j| j.kind == "generic_generate")
        .cloned()
        .collect();
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(jobs)
}
async fn api_file(
    State(state): State<Arc<AppState>>,
    ApiPath((id, file_id)): ApiPath<(String, String)>,
) -> ApiResult<Response> {
    let plan = state.generic_plan(&id)?;
    let file = plan
        .groups
        .iter()
        .flat_map(|g| &g.files)
        .find(|f| f.id == file_id)
        .context("文件ID不属于此计划")?;
    let path = check_file(&plan, file)?;
    let bytes = tokio::fs::read(path).await?;
    if hex::encode(sha2::Sha256::digest(&bytes)) != file.sha256 {
        bail_api("源文件在读取期间变化，请重新扫描")?;
    }
    Ok((
        [(header::CONTENT_TYPE, "application/pdf")],
        Body::from(bytes),
    )
        .into_response())
}
fn bail_api(message: &str) -> ApiResult<()> {
    Err(ApiError(anyhow::anyhow!(message.to_owned())))
}
async fn api_open_output(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Value>> {
    let plan = state.generic_plan(&id)?;
    let directory = absolute_checked(Path::new(&plan.output_dir), true)?;
    if !checked_metadata(&directory)?.is_dir() {
        return Err(anyhow::anyhow!("输出目录不存在，请先生成").into());
    }
    // Only a registered directory, never shell text from the client.
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("explorer.exe");
        c.arg(&directory);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(&directory);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&directory);
        c
    };
    command
        .spawn()
        .context("无法打开输出目录，请在资源管理器中手动打开预览路径")?;
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod pdf_inspection_tests {
    use super::*;

    fn document() -> Document {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type"=>"Page", "Parent"=>pages,
            "MediaBox"=>vec![0.into(),0.into(),100.into(),200.into()]
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type"=>"Pages", "Kids"=>vec![Object::Reference(page)], "Count"=>1
            }),
        );
        let root = doc.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
        doc.trailer.set("Root", root);
        doc
    }

    fn form(doc: &mut Document, kind: &str, value: Option<lopdf::Dictionary>) {
        let mut field =
            dictionary! {"FT"=>Object::Name(kind.as_bytes().to_vec()), "Subtype"=>"Widget"};
        if let Some(value) = value {
            let value = doc.add_object(value);
            field.set("V", value);
        }
        let field = doc.add_object(field);
        let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_object_mut(catalog)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "AcroForm",
                dictionary! {"Fields"=>vec![Object::Reference(field)]},
            );
    }

    fn inspect_both(doc: &mut Document) -> std::result::Result<u32, String> {
        let mut bytes = vec![];
        doc.save_to(&mut bytes).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("synthetic.pdf");
        fs::write(&path, &bytes).unwrap();
        let from_path = inspect_pdf(&path).map_err(|e| e.to_string());
        let from_bytes = inspect_pdf_bytes(&bytes).map_err(|e| e.to_string());
        assert_eq!(from_path, from_bytes);
        from_bytes
    }

    #[test]
    fn path_and_bytes_apply_the_same_static_pdf_boundary() {
        assert_eq!(inspect_both(&mut document()).unwrap(), 1);
        let mut interactive = document();
        form(&mut interactive, "Tx", None);
        let error = inspect_both(&mut interactive).unwrap_err();
        assert!(error.contains("交互表单") && !error.contains("数字签名"));
        let mut encrypted = document();
        encrypted
            .trailer
            .set("Encrypt", dictionary! {"Filter"=>"Standard", "V"=>1,"R"=>2});
        assert!(inspect_both(&mut encrypted)
            .unwrap_err()
            .contains("加密PDF"));
        let mut action = document();
        let root = action.trailer.get(b"Root").unwrap().as_reference().unwrap();
        action
            .get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "OpenAction",
                dictionary! {"S"=>"JavaScript","JS"=>Object::string_literal("synthetic")},
            );
        assert!(inspect_both(&mut action)
            .unwrap_err()
            .contains("打开时动作"));
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("invalid.pdf");
        fs::write(&path, b"invalid synthetic PDF").unwrap();
        assert_eq!(
            inspect_pdf(&path).unwrap_err().to_string(),
            inspect_pdf_bytes(b"invalid synthetic PDF")
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn populated_signature_requires_field_value_and_payload_evidence() {
        let mut signed = document();
        form(
            &mut signed,
            "Sig",
            Some(dictionary! {
                "Type"=>"Sig", "ByteRange"=>vec![0.into(),20.into(),40.into(),10.into()],
                "Contents"=>Object::String(vec![48,1,2,3],lopdf::StringFormat::Hexadecimal)
            }),
        );
        let error = inspect_both(&mut signed).unwrap_err();
        assert!(
            error.contains("已填写的数字签名字段")
                && error.contains("未验证签名有效性")
                && error.contains("原件未被改动")
        );
        for value in [
            None,
            Some(dictionary! {
                "Type"=>"Sig", "ByteRange"=>vec![0.into(),0.into(),0.into(),0.into()],
                "Contents"=>Object::String(vec![0;8],lopdf::StringFormat::Hexadecimal)
            }),
        ] {
            let mut unsigned = document();
            form(&mut unsigned, "Sig", value);
            let error = inspect_both(&mut unsigned).unwrap_err();
            assert!(error.contains("未填写或无法验证的签名字段"));
            assert!(!error.contains("已填写的数字签名字段"));
        }
    }

    #[test]
    fn named_destinations_alone_are_not_reported_as_active_content() {
        let mut doc = document();
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        doc.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("Names", dictionary! {
            "Dests"=>dictionary! {"Names"=>vec![Object::string_literal("synthetic"),Object::Array(vec![Object::Reference(page),Object::Name(b"Fit".to_vec())])]}
        });
        assert_eq!(inspect_both(&mut doc).unwrap(), 1);
    }
}

#[cfg(test)]
mod path_observation_tests {
    use super::*;

    #[test]
    fn missing_component_created_between_observations_is_validated_as_a_directory() {
        for must_exist in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let base = temporary.path().canonicalize().unwrap();
            let directory = base.join("new-output");
            let mut observations = 0;
            check_component_with(&directory, must_exist, true, |path| {
                observations += 1;
                let observed = checked_metadata(path);
                if observations == 1 {
                    assert!(observed.as_ref().is_err_and(is_missing));
                    fs::create_dir(path).unwrap();
                }
                observed
            })
            .unwrap();
            assert_eq!(observations, 2);
            assert_eq!(
                absolute_checked(&directory, must_exist).unwrap(),
                directory.canonicalize().unwrap()
            );
        }
    }

    #[test]
    fn missing_component_stays_missing_or_becomes_a_file_without_weakening_directory_checks() {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let missing = base.join("missing");
        check_component_with(&missing, false, true, checked_metadata).unwrap();
        assert!(check_component_with(&missing, true, true, checked_metadata)
            .unwrap_err()
            .to_string()
            .contains("目录不存在"));
        let became_file = base.join("new-file");
        let mut observations = 0;
        let error = check_component_with(&became_file, false, true, |path| {
            observations += 1;
            let observed = checked_metadata(path);
            if observations == 1 {
                assert!(observed.as_ref().is_err_and(is_missing));
                fs::write(path, b"ordinary file").unwrap();
            }
            observed
        })
        .unwrap_err();
        assert!(error.to_string().contains("中间组件不是目录"));
        assert!(absolute_checked(&became_file.join("child"), false).is_err());
        assert_eq!(
            absolute_checked(&became_file, true).unwrap(),
            became_file.canonicalize().unwrap()
        );
    }

    #[test]
    fn a_new_security_error_is_preserved_and_other_original_errors_are_not_retried() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().canonicalize().unwrap().join("leaf");
        let mut observations = 0;
        let error = check_component_with(&path, false, true, |_| {
            observations += 1;
            if observations == 1 {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())
            } else {
                bail!("重解析点或云占位文件拒绝")
            }
        })
        .unwrap_err();
        assert_eq!(observations, 2);
        assert!(error.to_string().contains("云占位"));
        let mut observations = 0;
        let error = check_component_with(&path, false, true, |_| {
            observations += 1;
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into())
        })
        .unwrap_err();
        assert_eq!(observations, 1);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_component_replaced_by_a_real_symlink_is_rejected_on_reobservation() {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let target = base.join("target");
        fs::create_dir(&target).unwrap();
        for must_exist in [false, true] {
            let link = base.join(if must_exist {
                "required-link"
            } else {
                "optional-link"
            });
            let mut observations = 0;
            let error = check_component_with(&link, must_exist, true, |path| {
                observations += 1;
                let observed = checked_metadata(path);
                if observations == 1 {
                    assert!(observed.as_ref().is_err_and(is_missing));
                    std::os::unix::fs::symlink(&target, path).unwrap();
                }
                observed
            })
            .unwrap_err();
            assert_eq!(observations, 2);
            assert!(error.to_string().contains("符号链接"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn missing_component_replaced_by_a_real_junction_is_rejected_on_reobservation() {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let target = base.join("target");
        fs::create_dir(&target).unwrap();
        let junction = base.join("new-junction");
        let mut observations = 0;
        let error = check_component_with(&junction, false, true, |path| {
            observations += 1;
            let observed = checked_metadata(path);
            if observations == 1 {
                assert!(observed.as_ref().is_err_and(is_missing));
                let result = std::process::Command::new("cmd.exe")
                    .args(["/D", "/C", "mklink", "/J"])
                    .arg(path)
                    .arg(&target)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "junction fixture failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
            observed
        })
        .unwrap_err();
        assert_eq!(observations, 2);
        let reason = error.to_string();
        assert!(
            reason.contains("符号链接") || reason.contains("重解析点"),
            "junction must be rejected as a link or reparse point: {error:#}"
        );
        fs::remove_dir(junction).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_leaf_link_appearing_after_both_missing_observations_is_not_canonicalized() {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let target = base.join("target");
        fs::create_dir(&target).unwrap();
        let leaf = base.join("late-link");
        let mut observations = 0;
        check_component_with(&leaf, false, false, |path| {
            observations += 1;
            let observed = checked_metadata(path);
            assert!(observed.as_ref().is_err_and(is_missing));
            if observations == 2 {
                std::os::unix::fs::symlink(&target, path).unwrap();
            }
            observed
        })
        .unwrap();
        assert_eq!(observations, 2);
        assert!(leaf.exists());
        assert!(canonicalize_checked_leaf(&leaf)
            .unwrap_err()
            .to_string()
            .contains("符号链接"));
    }
}
