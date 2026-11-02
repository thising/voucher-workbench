//! The product's general batch workflow. Source data is immutable; edits create
//! new plans, and optional OCR/review never becomes an export permission gate.
#[path = "batch_conflicts.rs"]
mod conflicts;
use crate::{
    generic::{absolute_checked, checked_metadata, glob_regex, natural_compare, output_name},
    jobs::{self, AppState, GenerateRequest, Job, JobResult},
    model::Orientation,
    orientation::{self, Engine},
    pdf::{self, OutputRecords, RecordHash, Transaction},
    rename_index::{self, Evidence},
    workers,
};
use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{Path as ApiPath, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
pub(crate) use conflicts::close_output_conflicts;
pub(crate) use conflicts::ConflictCoordinator;
pub use conflicts::OutputConflict;
use lopdf::{dictionary, Document, Object};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Instant, UNIX_EPOCH},
};

pub const DISCLAIMER_VERSION: &str = "batch-orientation-disclaimer-v2";
pub const DEFAULT_THREADS: usize = 4;
pub const DEFAULT_RULES: &str = "# 每行一条排除规则；以 # 开头的整行是注释\n**/indexImg/**\n**/index.html\n**/.DS_Store\n**/._*\n**/Thumbs.db";
const MAX_DEPTH: u16 = 256;
fn default_rules() -> String {
    DEFAULT_RULES.into()
}
fn default_mode() -> String {
    "name".into()
}
fn default_threads() -> usize {
    DEFAULT_THREADS
}
fn now() -> String {
    jobs::now()
}
fn path_text(p: &Path) -> Result<String> {
    p.to_str()
        .map(str::to_owned)
        .context("路径含无法保存的非Unicode名称，请改名后重试")
}
fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").to_lowercase()
}
fn issue(severity: &str, code: &str, message: impl Into<String>, path: Option<String>) -> Issue {
    Issue {
        severity: severity.into(),
        code: code.into(),
        message: message.into(),
        path,
    }
}
pub use crate::generic::{Exclusion, Issue};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Sort {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub descending: bool,
    #[serde(default)]
    pub separator: Option<String>,
}
impl Default for Sort {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            descending: false,
            separator: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub root: String,
    #[serde(default)]
    pub level: u16,
    #[serde(default = "default_rules")]
    pub exclude_rules: String,
    #[serde(default)]
    pub index: Option<rename_index::Config>,
    #[serde(default)]
    pub sort: Sort,
    #[serde(default)]
    pub output_names: Vec<OutputName>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputName {
    pub relative_path: String,
    pub output_name: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationChange {
    pub from_cw: u16,
    pub to_cw: u16,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisSummary {
    pub changed: u32,
    pub unchanged: u32,
    pub skipped: u32,
    pub failed: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    pub relative_path: String,
    pub kind: String,
    pub bytes: u64,
    pub modified_ms: u64,
    pub sha256: String,
    pub pages: u32,
    #[serde(default)]
    pub pdf_has_signature: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub rotation_cw: u16,
    pub page_rotations: Vec<u16>,
    pub reviewed: bool,
    pub reviewed_pages: Vec<bool>,
    pub manual_rotation: bool,
    pub orientation: Option<Orientation>,
    pub orientation_error: Option<String>,
    #[serde(default)]
    pub auto_rotation_change: Option<RotationChange>,
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
    pub rename: Evidence,
    pub sort: Sort,
    pub review_count: usize,
    pub image_count: usize,
    pub pdf_count: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Workflow {
    pub consent_version: Option<String>,
    pub consented_at: Option<String>,
    pub analysis_completed: bool,
    #[serde(default = "default_threads")]
    pub threads: usize,
}
impl Default for Workflow {
    fn default() -> Self {
        Self {
            consent_version: None,
            consented_at: None,
            analysis_completed: false,
            threads: DEFAULT_THREADS,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OmittedSource {
    pub group_id: String,
    pub position: usize,
    pub file: File,
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
    pub level: u16,
    pub sort: Sort,
    pub exclude_rules: String,
    pub index: Option<rename_index::Snapshot>,
    pub index_error: Option<String>,
    pub groups: Vec<Group>,
    pub issues: Vec<Issue>,
    pub exclusions: Vec<Exclusion>,
    pub ignored_files: usize,
    pub empty_directories: Vec<String>,
    pub orientation_workflow: Workflow,
    #[serde(default)]
    pub omitted_sources: Vec<OmittedSource>,
    #[serde(default)]
    pub removed_sources: Vec<OmittedSource>,
    #[serde(default)]
    pub scan_metrics: Option<ScanMetrics>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Level {
    pub level: u16,
    pub folder_count: usize,
    pub file_count: usize,
    pub samples: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Initialization {
    pub root: String,
    pub max_level: u16,
    pub levels: Vec<Level>,
    pub folder_count: usize,
    pub file_count: usize,
    pub issues: Vec<Issue>,
    pub output_dir: String,
    pub level: u16,
    pub groups: Vec<PreviewGroup>,
    pub summary: Summary,
    pub exclusions: Vec<Exclusion>,
    pub ignored_files: usize,
    pub elapsed_ms: u64,
    pub index: Option<rename_index::Snapshot>,
    pub index_error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct PreviewGroup {
    pub relative_path: String,
    pub path: String,
    pub name: String,
    pub shallow: bool,
    pub candidate_count: usize,
    pub image_count: usize,
    pub pdf_count: usize,
    pub bytes: u64,
    pub output_name: String,
    pub rename: Evidence,
    pub issues: Vec<Issue>,
    pub empty: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub group_count: usize,
    pub nonempty_groups: usize,
    pub candidate_count: usize,
    pub image_count: usize,
    pub pdf_count: usize,
    pub bytes: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanTimings {
    pub metadata_ms: f64,
    pub read_ms: f64,
    pub hash_ms: f64,
    pub pdf_ms: f64,
    pub image_ms: f64,
    pub index_ms: f64,
    pub output_ms: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanCounts {
    pub image_files: usize,
    pub pdf_files: usize,
    pub source_errors: usize,
    pub bytes_read: u64,
    pub hash_calls: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanMetrics {
    pub elapsed_ms: u64,
    pub timings: ScanTimings,
    pub counts: ScanCounts,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanLog {
    pub level: String,
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanProgress {
    pub id: String,
    pub status: String,
    pub plan_id: Option<String>,
    pub phase: String,
    pub done: u32,
    pub total: u32,
    pub elapsed_ms: u64,
    pub current_file: Option<String>,
    pub timings: ScanTimings,
    pub counts: ScanCounts,
    pub logs: Vec<ScanLog>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupEdit {
    pub group_id: String,
    pub file_ids: Option<Vec<String>>,
    pub output_name: Option<String>,
    pub sort: Option<Sort>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEdit {
    pub file_id: String,
    pub rotation_cw: Option<u16>,
    pub page_rotations: Option<Vec<u16>>,
    pub reviewed: Option<bool>,
    pub reviewed_pages: Option<Vec<bool>>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub output_dir: Option<String>,
    #[serde(default)]
    pub groups: Vec<GroupEdit>,
    #[serde(default)]
    pub files: Vec<FileEdit>,
    #[serde(default)]
    pub remove_file_ids: Vec<String>,
    #[serde(default)]
    pub restore_file_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Analyze {
    pub accept_disclaimer: bool,
    #[serde(default = "default_threads")]
    pub threads: usize,
    pub group_ids: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Generate {
    #[serde(default = "default_threads")]
    pub threads: usize,
    pub group_ids: Option<Vec<String>>,
    pub output_dir: Option<String>,
}

fn is_image(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "bmp" | "webp" | "tif" | "tiff"
            )
        })
}
fn is_source(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".pdf") || is_image(name)
}
fn depth(relative: &str) -> usize {
    relative.split('/').filter(|s| !s.is_empty()).count()
}
fn group_key(relative: &str, level: u16) -> String {
    let dirs = relative
        .rsplit_once('/')
        .map(|(s, _)| s.split('/').collect::<Vec<_>>())
        .unwrap_or_default();
    dirs[..dirs.len().min(level as usize)].join("/")
}
fn validate_sort(sort: &Sort, manual: bool) -> Result<()> {
    if !matches!(
        sort.mode.as_str(),
        "name" | "modified" | "prefix" | "suffix"
    ) && !(manual && sort.mode == "manual")
    {
        bail!("排序模式无效");
    }
    if sort
        .separator
        .as_ref()
        .is_some_and(|s| s.is_empty() || s.contains(['\r', '\n']))
    {
        bail!("排序分隔符不能为空或包含换行");
    }
    Ok(())
}
fn part(file: &File, sort: &Sort) -> String {
    let stem = Path::new(&file.name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&file.name);
    let parts: Vec<_> = if let Some(separator) = &sort.separator {
        stem.split(separator).filter(|p| !p.is_empty()).collect()
    } else {
        stem.split(|c: char| c == '_' || c == '-' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .collect()
    };
    if sort.mode == "prefix" {
        parts.first().copied().unwrap_or(stem).into()
    } else {
        parts.last().copied().unwrap_or(stem).into()
    }
}
fn sort_files(files: &mut [File], sort: &Sort) {
    files.sort_by(|a, b| {
        let order = match sort.mode.as_str() {
            "modified" => a.modified_ms.cmp(&b.modified_ms),
            "prefix" | "suffix" => natural_compare(&part(a, sort), &part(b, sort)),
            _ => natural_compare(&a.name, &b.name),
        };
        let order = if sort.descending {
            order.reverse()
        } else {
            order
        };
        order.then_with(|| natural_compare(&a.relative_path, &b.relative_path))
    });
}
pub fn needs_review(file: &File) -> bool {
    file.kind == "image"
        && !file.manual_rotation
        && file.auto_rotation_change.is_some_and(|change| {
            change.from_cw != change.to_cw && file.rotation_cw == change.to_cw
        })
}
pub(crate) fn normalize_review_counts(plan: &mut Plan) {
    // review_count is derived UI state, not evidence that old pictures changed.
    // Reinterpret in memory only; historical JSON files are not rewritten.
    for group in &mut plan.groups {
        group.review_count = group.files.iter().filter(|file| needs_review(file)).count();
    }
}
fn unreviewed_pages(file: &File) -> usize {
    usize::from(needs_review(file))
}

fn refresh_group(group: &mut Group) -> Result<()> {
    group.pages = group.files.iter().try_fold(0u32, |n, f| {
        n.checked_add(f.pages).context("分组页数超出支持范围")
    })?;
    group.empty = group.files.is_empty();
    group.review_count = group.files.iter().filter(|f| needs_review(f)).count();
    group.image_count = group.files.iter().filter(|f| f.kind == "image").count();
    group.pdf_count = group.files.len() - group.image_count;
    group
        .issues
        .retain(|i| !matches!(i.code.as_str(), "source_error" | "pdf_signature"));
    let signed = group
        .files
        .iter()
        .filter(|file| file.pdf_has_signature && file.error.is_none())
        .count();
    if signed > 0 {
        group.issues.push(issue("warning","pdf_signature",format!("包含{signed}份带签名PDF；合并仅保留可用静态外观，不保留原数字签名验证。原件不会修改。"),Some(group.relative_path.clone())));
    }
    group.issues.extend(group.files.iter().filter_map(|f| {
        f.error.as_ref().map(|e| {
            issue(
                "error",
                "source_error",
                e.clone(),
                Some(f.relative_path.clone()),
            )
        })
    }));
    Ok(())
}

pub fn initialize(root: &str, state_dir: Option<&Path>) -> Result<Initialization> {
    preview(
        ScanRequest {
            root: root.into(),
            level: 0,
            exclude_rules: default_rules(),
            index: None,
            sort: Sort::default(),
            output_names: vec![],
        },
        state_dir,
    )
}

struct Rule {
    regex: Regex,
    exclusion: usize,
}
struct Scanner {
    root: std::path::PathBuf,
    output: std::path::PathBuf,
    state: Option<std::path::PathBuf>,
    level: u16,
    rules: Vec<Rule>,
    groups: BTreeMap<String, Vec<File>>,
    dirs: Vec<String>,
    issues: Vec<Issue>,
    exclusions: Vec<Exclusion>,
    ignored: usize,
}
impl Scanner {
    fn walk(&mut self, relative: &str, cancel: &AtomicBool) -> Result<()> {
        if depth(relative) > MAX_DEPTH as usize {
            bail!("目录层级超过256层，请缩小待处理范围");
        }
        let directory = self.root.join(relative);
        checked_metadata(&directory)?;
        self.dirs.push(relative.into());
        if depth(relative) == self.level as usize {
            self.groups.entry(relative.into()).or_default();
        }
        let mut entries = fs::read_dir(&directory)
            .with_context(|| {
                format!(
                    "无法读取目录：{}，请检查权限后重新扫描",
                    directory.display()
                )
            })?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by(|a, b| {
            natural_compare(
                &a.file_name().to_string_lossy(),
                &b.file_name().to_string_lossy(),
            )
        });
        for entry in entries {
            if cancel.load(Ordering::Acquire) {
                bail!("扫描已取消，未保存不完整计划");
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("目录含非Unicode文件名，请改名后重新扫描"))?;
            let rel = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let raw = fs::symlink_metadata(entry.path())?;
            let is_dir = raw.is_dir();
            let automatic = if path_key(&entry.path()) == path_key(&self.output) {
                Some("output")
            } else if self
                .state
                .as_ref()
                .is_some_and(|s| path_key(s) == path_key(&entry.path()))
            {
                Some("state")
            } else {
                None
            };
            if let Some(kind) = automatic {
                if let Some(e) = self.exclusions.iter_mut().find(|e| e.kind == kind) {
                    e.matches.push(format!("{rel}/"));
                }
                continue;
            }
            let mut excluded = false;
            for rule in &self.rules {
                if rule.regex.is_match(&rel) || (is_dir && rule.regex.is_match(&format!("{rel}/")))
                {
                    self.exclusions[rule.exclusion].matches.push(if is_dir {
                        format!("{rel}/")
                    } else {
                        rel.clone()
                    });
                    excluded = true;
                }
            }
            if excluded {
                continue;
            }
            if let Err(e) = checked_metadata(&entry.path()) {
                self.issues.push(issue(
                    "warning",
                    "unsupported_path",
                    format!("未纳入：{e:#}"),
                    Some(rel),
                ));
                self.ignored += 1;
                continue;
            }
            if is_dir {
                self.walk(&rel, cancel)?;
                continue;
            }
            if !raw.is_file() || !is_source(&name) {
                self.ignored += 1;
                continue;
            }
            let file = File {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                relative_path: rel.clone(),
                kind: if is_image(&rel) { "image" } else { "pdf" }.into(),
                bytes: raw.len(),
                modified_ms: raw
                    .modified()?
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX),
                sha256: String::new(),
                pages: 0,
                pdf_has_signature: false,
                width: None,
                height: None,
                rotation_cw: 0,
                page_rotations: vec![],
                reviewed: false,
                reviewed_pages: vec![],
                manual_rotation: false,
                orientation: None,
                orientation_error: None,
                auto_rotation_change: None,
                error: None,
            };
            self.groups
                .entry(group_key(&rel, self.level))
                .or_default()
                .push(file);
        }
        Ok(())
    }
}
fn metadata_scanner(
    request: &ScanRequest,
    state_dir: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<Scanner> {
    validate_sort(&request.sort, false)?;
    if request.level > MAX_DEPTH {
        bail!("合并级别超出支持范围，请重新初始化目录");
    }
    if request.root.trim().is_empty() {
        bail!("请选择待处理文件夹");
    }
    let root = absolute_checked(Path::new(&request.root), true)?;
    if !checked_metadata(&root)?.is_dir() {
        bail!("待处理路径不是文件夹");
    }
    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .context("不能选择磁盘根目录作为待处理文件夹")?;
    let output = root
        .parent()
        .context("资料目录没有上级目录")?
        .join(format!("{root_name}-PDF"));
    let state = state_dir.map(|s| absolute_checked(s, true)).transpose()?;
    if state.as_ref().is_some_and(|s| root.starts_with(s)) {
        bail!("不能将处理记录目录作为待处理资料目录");
    }
    let mut issues = vec![];
    let mut rules = vec![];
    let mut exclusions = vec![];
    for (line, raw) in request.exclude_rules.lines().enumerate() {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        match glob_regex(raw) {
            Ok(regex) => {
                rules.push(Rule {
                    regex,
                    exclusion: exclusions.len(),
                });
                exclusions.push(Exclusion {
                    rule: raw.into(),
                    kind: "glob".into(),
                    matches: vec![],
                });
            }
            Err(e) => issues.push(issue(
                "warning",
                "invalid_exclusion",
                format!("第{}行排除规则无效，已忽略：{e:#}", line + 1),
                Some(raw.into()),
            )),
        }
    }
    if let Some(s) = &state {
        if let Ok(rel) = s.strip_prefix(&root) {
            exclusions.push(Exclusion {
                rule: path_text(rel)?.replace('\\', "/"),
                kind: "state".into(),
                matches: vec![],
            });
        }
    }
    let mut scanner = Scanner {
        root,
        output,
        state,
        level: request.level,
        rules,
        groups: BTreeMap::new(),
        dirs: vec![],
        issues,
        exclusions,
        ignored: 0,
    };
    scanner.walk("", cancel)?;
    Ok(scanner)
}
fn read_index(
    config: Option<rename_index::Config>,
    issues: &mut Vec<Issue>,
) -> (Option<rename_index::Index>, Option<String>) {
    let index_result = config.map(|mut config| {
        config.duplicates = "first".into();
        rename_index::Index::read(config)
    });
    let (index, index_error) = match index_result {
        None => (None, None),
        Some(Ok(index)) => (Some(index), None),
        Some(Err(e)) => {
            let e = format!("重命名索引未启用，所有分组回退文件夹名：{e:#}");
            issues.push(issue("warning", "index_unavailable", e.clone(), None));
            (None, Some(e))
        }
    };
    (index, index_error)
}
fn assemble_plan(request: ScanRequest, scanner: Scanner) -> Result<Plan> {
    let root = scanner.root.clone();
    let output = scanner.output.clone();
    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .context("目录名称无效")?;
    let mut scanner = scanner;
    let (index, index_error) = read_index(request.index.clone(), &mut scanner.issues);
    let included: Vec<_> = scanner
        .groups
        .values()
        .flat_map(|v| v.iter().map(|f| f.relative_path.clone()))
        .collect();
    let empty_directories = scanner
        .dirs
        .iter()
        .filter(|d| {
            !included
                .iter()
                .any(|f| d.is_empty() || f.starts_with(&format!("{d}/")))
        })
        .cloned()
        .collect();
    let mut groups = vec![];
    for (relative, mut files) in scanner.groups {
        sort_files(&mut files, &request.sort);
        let name = if relative.is_empty() {
            root_name.to_owned()
        } else {
            relative.rsplit('/').next().unwrap().into()
        };
        let rename = index
            .as_ref()
            .map(|i| i.resolve(&name, &relative))
            .unwrap_or_else(Evidence::folder);
        let raw_name = rename
            .value
            .as_ref()
            .map(|v| {
                if index
                    .as_ref()
                    .is_some_and(|i| i.snapshot.config.naming == "folder_index")
                {
                    format!("{name}-{v}")
                } else {
                    v.clone()
                }
            })
            .unwrap_or_else(|| name.clone());
        let mut group_issues = vec![];
        if matches!(rename.status.as_str(), "conflict" | "unmatched" | "empty") {
            group_issues.push(issue(
                "warning",
                "rename_fallback",
                rename.message.clone(),
                Some(relative.clone()),
            ));
        }
        let raw_name = raw_name.replace(['/', '\\'], "_");
        let output_name = match output_name(&raw_name) {
            Ok(s) => s,
            Err(e) => {
                group_issues.push(issue(
                    "error",
                    "output_name",
                    format!("{e:#}"),
                    Some(relative.clone()),
                ));
                format!("invalid-{}.pdf", uuid::Uuid::new_v4())
            }
        };
        let mut group = Group {
            id: uuid::Uuid::new_v4().to_string(),
            relative_path: relative.clone(),
            name,
            shallow: depth(&relative) < request.level as usize,
            output_path: path_text(&output.join(&output_name))?,
            output_name,
            files,
            pages: 0,
            issues: group_issues,
            empty: false,
            rename,
            sort: request.sort.clone(),
            review_count: 0,
            image_count: 0,
            pdf_count: 0,
        };
        refresh_group(&mut group)?;
        groups.push(group);
    }
    groups.sort_by(|a, b| natural_compare(&a.relative_path, &b.relative_path));
    let mut plan = Plan {
        schema_version: 1,
        mode: "batch_merge".into(),
        id: uuid::Uuid::new_v4().to_string(),
        parent_plan_id: None,
        created_at: now(),
        root: path_text(&root)?,
        output_dir: path_text(&output)?,
        level: request.level,
        sort: request.sort,
        exclude_rules: request.exclude_rules,
        index: index.map(|i| i.snapshot),
        index_error,
        groups,
        issues: scanner.issues,
        exclusions: scanner.exclusions,
        ignored_files: scanner.ignored,
        empty_directories,
        orientation_workflow: Workflow::default(),
        omitted_sources: vec![],
        removed_sources: vec![],
        scan_metrics: None,
    };
    apply_output_names(&mut plan, &request.output_names)?;
    refresh_output_issues(&mut plan)?;
    for e in &plan.exclusions {
        if e.kind == "glob" && e.matches.is_empty() {
            plan.issues.push(issue(
                "info",
                "exclude_no_match",
                format!("排除规则没有命中：{}", e.rule),
                None,
            ));
        }
    }
    Ok(plan)
}

fn work_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
fn apply_output_names(plan: &mut Plan, names: &[OutputName]) -> Result<()> {
    let mut unique = HashSet::new();
    for item in names {
        if !unique.insert(&item.relative_path) {
            bail!("预扫描名称包含重复目录，请刷新预览后重试");
        }
        let group = plan
            .groups
            .iter_mut()
            .find(|g| g.relative_path == item.relative_path)
            .context("预扫描名称所属目录已变化，请刷新目录预览后重试")?;
        group.output_name = output_name(&item.output_name)?;
        group.issues.retain(|i| i.code != "output_name");
    }
    Ok(())
}
fn available_levels(scanner: &Scanner) -> Vec<Level> {
    let max = scanner.dirs.iter().map(|d| depth(d)).max().unwrap_or(0) as u16;
    (0..=max)
        .map(|level| {
            let mut groups = HashSet::new();
            for directory in &scanner.dirs {
                if depth(directory) == level as usize {
                    groups.insert(directory.clone());
                }
            }
            for file in scanner.groups.values().flatten() {
                groups.insert(group_key(&file.relative_path, level));
            }
            let mut samples: Vec<_> = scanner
                .dirs
                .iter()
                .filter(|d| depth(d) == level as usize)
                .cloned()
                .collect();
            samples.sort_by(|a, b| natural_compare(a, b));
            samples.truncate(3);
            if level == 0 {
                samples = vec![scanner
                    .root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()];
            }
            Level {
                level,
                folder_count: groups.len(),
                file_count: scanner.groups.values().map(Vec::len).sum(),
                samples,
            }
        })
        .collect()
}
pub fn preview(request: ScanRequest, state_dir: Option<&Path>) -> Result<Initialization> {
    let started = Instant::now();
    let scanner = metadata_scanner(&request, state_dir, &AtomicBool::new(false))?;
    let levels = available_levels(&scanner);
    let max_level = levels.last().map(|l| l.level).unwrap_or(0);
    let folder_count = scanner.dirs.len();
    let plan = assemble_plan(request, scanner)?;
    let mut summary = Summary {
        group_count: plan.groups.len(),
        nonempty_groups: 0,
        candidate_count: 0,
        image_count: 0,
        pdf_count: 0,
        bytes: 0,
    };
    let groups = plan
        .groups
        .iter()
        .map(|g| {
            summary.nonempty_groups += usize::from(!g.empty);
            summary.candidate_count += g.files.len();
            summary.image_count += g.image_count;
            summary.pdf_count += g.pdf_count;
            let bytes = g.files.iter().map(|f| f.bytes).sum();
            summary.bytes += bytes;
            Ok(PreviewGroup {
                relative_path: g.relative_path.clone(),
                path: path_text(&Path::new(&plan.root).join(&g.relative_path))?,
                name: g.name.clone(),
                shallow: g.shallow,
                candidate_count: g.files.len(),
                image_count: g.image_count,
                pdf_count: g.pdf_count,
                bytes,
                output_name: g.output_name.clone(),
                rename: g.rename.clone(),
                issues: g.issues.clone(),
                empty: g.empty,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Initialization {
        root: plan.root,
        output_dir: plan.output_dir,
        level: plan.level,
        max_level,
        levels,
        folder_count,
        file_count: summary.candidate_count,
        groups,
        summary,
        issues: plan.issues,
        exclusions: plan.exclusions,
        ignored_files: plan.ignored_files,
        elapsed_ms: elapsed_ms(started),
        index: plan.index,
        index_error: plan.index_error,
    })
}
fn inspect_candidate(root: &Path, source: &File) -> (File, ScanTimings, ScanCounts) {
    let mut file = source.clone();
    let mut times = ScanTimings::default();
    let mut counts = ScanCounts::default();
    if file.kind == "image" {
        counts.image_files = 1;
    } else {
        counts.pdf_files = 1;
    }
    let result = (|| -> Result<()> {
        let path = pdf::safe_source(root, &file.relative_path)?;
        let before = checked_metadata(&path)?;
        if !before.is_file()
            || before.len() != file.bytes
            || before.modified()?.duration_since(UNIX_EPOCH)?.as_millis() as u64 != file.modified_ms
        {
            bail!("来源在目录预览后发生变化，请重新扫描");
        }
        let t = Instant::now();
        let bytes = fs::read(&path)?;
        times.read_ms = work_ms(t);
        counts.bytes_read = bytes.len() as u64;
        let t = Instant::now();
        file.sha256 = hex::encode(sha2::Sha256::digest(&bytes));
        times.hash_ms = work_ms(t);
        counts.hash_calls = 1;
        let t = Instant::now();
        let inspected = (|| -> Result<()> {
            if file.kind == "pdf" {
                let inspected = crate::generic::inspect_batch_pdf_bytes(&bytes)?;
                file.pages = inspected.pages;
                file.pdf_has_signature = inspected.signature_notice;
                file.page_rotations = vec![0; file.pages as usize];
                file.reviewed_pages = vec![false; file.pages as usize];
            } else {
                let format = image::guess_format(&bytes).context("无法识别图片格式")?;
                crate::scan::reject_multiframe_bytes(&bytes, Some(format))?;
                let image = orientation::load_image_bytes(bytes)?;
                file.width = Some(image.width());
                file.height = Some(image.height());
                file.pages = 1;
            }
            Ok(())
        })();
        if file.kind == "pdf" {
            times.pdf_ms = work_ms(t);
        } else {
            times.image_ms = work_ms(t);
        }
        let after = checked_metadata(&path)?;
        if after.len() != before.len() || after.modified()? != before.modified()? {
            bail!("来源在内容校验时发生变化，请重新扫描");
        }
        inspected
    })();
    if let Err(error) = result {
        counts.source_errors = 1;
        file.error = Some(format!("无法纳入{}：{error:#}", file.relative_path));
    }
    (file, times, counts)
}
fn add_metrics(total: &mut ScanMetrics, times: &ScanTimings, counts: &ScanCounts) {
    total.timings.read_ms += times.read_ms;
    total.timings.hash_ms += times.hash_ms;
    total.timings.pdf_ms += times.pdf_ms;
    total.timings.image_ms += times.image_ms;
    total.counts.image_files += counts.image_files;
    total.counts.pdf_files += counts.pdf_files;
    total.counts.source_errors += counts.source_errors;
    total.counts.bytes_read += counts.bytes_read;
    total.counts.hash_calls += counts.hash_calls;
}
pub fn scan(request: ScanRequest, state_dir: Option<&Path>) -> Result<Plan> {
    scan_with_progress(request, state_dir, &AtomicBool::new(false), |_| Ok(()))
}
fn scan_with_progress(
    request: ScanRequest,
    state_dir: Option<&Path>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&ScanProgress) -> Result<()>,
) -> Result<Plan> {
    let started = Instant::now();
    let mut status = ScanProgress {
        id: String::new(),
        status: "running".into(),
        plan_id: None,
        phase: "enumerating".into(),
        done: 0,
        total: 0,
        elapsed_ms: 0,
        current_file: None,
        timings: ScanTimings::default(),
        counts: ScanCounts::default(),
        logs: vec![],
        error: None,
    };
    progress(&status)?;
    if cancel.load(Ordering::Acquire) {
        bail!("扫描已取消");
    }
    let mut scanner = metadata_scanner(&request, state_dir, cancel)?;
    let mut metrics = ScanMetrics::default();
    metrics.timings.metadata_ms = work_ms(started);
    let files: Vec<_> = scanner.groups.values().flatten().cloned().collect();
    status.total = u32::try_from(files.len()).context("来源数量超过支持范围")?;
    status.phase = "validating".into();
    status.logs.push(ScanLog {
        level: "info".into(),
        code: "discovered".into(),
        message: format!(
            "目录读取完成：{}个候选来源；仅完整校验后保存计划",
            files.len()
        ),
    });
    for issue in scanner.issues.iter().filter(|i| i.severity == "warning") {
        status.logs.push(ScanLog {
            level: "warning".into(),
            code: issue.code.clone(),
            message: issue.message.clone(),
        });
    }
    if status.logs.len() > 200 {
        status.logs.drain(..status.logs.len() - 200);
    }
    status.timings = metrics.timings.clone();
    status.elapsed_ms = elapsed_ms(started);
    progress(&status)?;
    let mut contexts = vec![(); DEFAULT_THREADS.min(files.len())];
    let mut validated = HashMap::new();
    workers::run(
        &files,
        &mut contexts,
        cancel,
        |_, file, _| inspect_candidate(&scanner.root, file),
        |event| {
            match event {
                workers::Event::Started { index } => {
                    status.current_file = Some(files[index].relative_path.clone());
                }
                workers::Event::Finished {
                    result: (file, times, counts),
                    ..
                } => {
                    add_metrics(&mut metrics, &times, &counts);
                    status.done += 1;
                    if let Some(error) = &file.error {
                        status.logs.push(ScanLog {
                            level: "error".into(),
                            code: "source_error".into(),
                            message: error.clone(),
                        });
                        if status.logs.len() > 200 {
                            status.logs.remove(0);
                        }
                    }
                    validated.insert(file.id.clone(), file);
                }
                workers::Event::Panicked => bail!("扫描工作线程异常，未保存不完整计划；请重试"),
                workers::Event::Progress { .. } => {}
            }
            status.timings = metrics.timings.clone();
            status.counts = metrics.counts.clone();
            status.elapsed_ms = elapsed_ms(started);
            progress(&status)
        },
    )?;
    if cancel.load(Ordering::Acquire) {
        bail!("扫描已取消，未保存不完整计划");
    }
    for file in scanner.groups.values_mut().flatten() {
        *file = validated
            .remove(&file.id)
            .context("来源校验未完成，请重试扫描")?;
    }
    status.phase = "naming".into();
    progress(&status)?;
    let t = Instant::now();
    let mut plan = assemble_plan(request, scanner)?;
    metrics.timings.index_ms = work_ms(t);
    status.phase = "saving".into();
    let t = Instant::now();
    if let Some(dir) = state_dir {
        inspect_existing_outputs(dir, &mut plan)?;
    }
    metrics.timings.output_ms = work_ms(t);
    if let Some(error) = &plan.index_error {
        status.logs.push(ScanLog {
            level: "warning".into(),
            code: "index_unavailable".into(),
            message: error.clone(),
        });
    }
    if status.logs.len() > 200 {
        status.logs.drain(..status.logs.len() - 200);
    }
    metrics.elapsed_ms = elapsed_ms(started);
    status.timings = metrics.timings.clone();
    status.counts = metrics.counts.clone();
    status.elapsed_ms = metrics.elapsed_ms;
    status.current_file = None;
    progress(&status)?;
    if cancel.load(Ordering::Acquire) {
        bail!("扫描已取消，未保存不完整计划");
    }
    plan.scan_metrics = Some(metrics);
    Ok(plan)
}

fn refresh_output_issues(plan: &mut Plan) -> Result<()> {
    let sources: HashSet<_> = plan
        .groups
        .iter()
        .flat_map(|g| &g.files)
        .map(|f| path_key(&Path::new(&plan.root).join(&f.relative_path)))
        .collect();
    let mut names: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, g) in plan.groups.iter_mut().enumerate() {
        g.issues.retain(|i| {
            !matches!(
                i.code.as_str(),
                "output_collision" | "output_is_source" | "output_existing"
            )
        });
        g.output_path = path_text(&Path::new(&plan.output_dir).join(&g.output_name))?;
        if sources.contains(&path_key(Path::new(&g.output_path))) {
            g.issues.push(issue(
                "error",
                "output_is_source",
                "输出目标是来源文件，请改名或换目录",
                Some(g.output_path.clone()),
            ));
        }
        if !g.empty {
            names
                .entry(g.output_name.to_lowercase())
                .or_default()
                .push(i);
        }
    }
    for indices in names.values().filter(|v| v.len() > 1) {
        for i in indices {
            plan.groups[*i].issues.push(issue(
                "error",
                "output_collision",
                "多个分组输出同名，请手动改名或增加文件夹前缀",
                None,
            ));
        }
    }
    Ok(())
}
type FilePositions = HashMap<String, HashMap<String, usize>>;

fn virtual_file_positions(plan: &Plan) -> FilePositions {
    let mut hidden: HashMap<&str, Vec<&OmittedSource>> = HashMap::new();
    for source in plan.removed_sources.iter().chain(&plan.omitted_sources) {
        hidden.entry(&source.group_id).or_default().push(source);
    }
    plan.groups
        .iter()
        .map(|group| {
            let mut order: Vec<_> = group.files.iter().map(|f| f.id.clone()).collect();
            if let Some(sources) = hidden.get_mut(group.id.as_str()) {
                sources.sort_by_key(|s| s.position);
                for source in sources {
                    order.insert(source.position.min(order.len()), source.file.id.clone());
                }
            }
            (
                group.id.clone(),
                order
                    .into_iter()
                    .enumerate()
                    .map(|(position, id)| (id, position))
                    .collect(),
            )
        })
        .collect()
}

fn restore_file(group: &mut Group, source: OmittedSource, positions: &FilePositions) -> Result<()> {
    let ranks = positions
        .get(&group.id)
        .context("恢复顺序所属合并单元不存在")?;
    let rank = ranks.get(&source.file.id).context("恢复顺序缺少来源文件")?;
    let insertion = group
        .files
        .iter()
        .position(|file| ranks.get(&file.id).is_some_and(|position| position > rank))
        .unwrap_or(group.files.len());
    group.files.insert(insertion, source.file);
    Ok(())
}

pub fn edit(original: &Plan, request: EditRequest) -> Result<Plan> {
    let mut plan = original.clone();
    // Positions include hidden sources. Active-vector indices would drift as
    // separate edits remove earlier files and could silently reorder pages.
    let positions = if request.remove_file_ids.is_empty() && request.restore_file_ids.is_empty() {
        HashMap::new()
    } else {
        virtual_file_positions(&plan)
    };
    let removing: HashSet<_> = request.remove_file_ids.iter().collect();
    let restoring: HashSet<_> = request.restore_file_ids.iter().collect();
    if removing.len() != request.remove_file_ids.len()
        || restoring.len() != request.restore_file_ids.len()
        || removing.iter().any(|id| restoring.contains(id))
    {
        bail!("移除/恢复文件ID重复或相互冲突");
    }
    for id in &request.remove_file_ids {
        if !plan
            .groups
            .iter()
            .flat_map(|g| &g.files)
            .any(|f| &f.id == id)
        {
            bail!("待移除文件不属于当前合并工作，请刷新后重试");
        }
    }
    for id in &request.restore_file_ids {
        if !plan.removed_sources.iter().any(|s| &s.file.id == id) {
            bail!("待恢复文件不在移除记录中");
        }
    }
    for group in &mut plan.groups {
        let mut retained = vec![];
        for (position, file) in group.files.drain(..).enumerate() {
            if removing.contains(&file.id) {
                plan.removed_sources.push(OmittedSource {
                    group_id: group.id.clone(),
                    position: positions
                        .get(&group.id)
                        .and_then(|r| r.get(&file.id))
                        .copied()
                        .unwrap_or(position),
                    file,
                });
            } else {
                retained.push(file);
            }
        }
        group.files = retained;
    }
    let output_relative = Path::new(&plan.output_dir)
        .strip_prefix(Path::new(&plan.root))
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"));
    let mut remaining = vec![];
    for source in plan.removed_sources.drain(..) {
        if restoring.contains(&source.file.id) {
            if output_relative
                .as_ref()
                .is_some_and(|p| source.file.relative_path.starts_with(&format!("{p}/")))
            {
                plan.omitted_sources.push(source);
            } else {
                let group = plan
                    .groups
                    .iter_mut()
                    .find(|g| g.id == source.group_id)
                    .context("恢复文件所属合并单元不存在")?;
                restore_file(group, source, &positions)?;
                if group.sort.mode != "manual" {
                    sort_files(&mut group.files, &group.sort);
                }
            }
        } else {
            remaining.push(source);
        }
    }
    plan.removed_sources = remaining;
    let mut groups = HashSet::new();
    for update in request.groups {
        if !groups.insert(update.group_id.clone()) {
            bail!("分组ID重复");
        }
        let group = plan
            .groups
            .iter_mut()
            .find(|g| g.id == update.group_id)
            .context("分组不属于本计划")?;
        if let Some(ids) = update.file_ids {
            let expected: HashSet<_> = group.files.iter().map(|f| f.id.clone()).collect();
            let actual: HashSet<_> = ids.iter().cloned().collect();
            if actual != expected || ids.len() != expected.len() {
                bail!("手动顺序必须包含该组全部文件且恰好一次");
            }
            let mut files: HashMap<_, _> =
                group.files.drain(..).map(|f| (f.id.clone(), f)).collect();
            group.files = ids.iter().map(|id| files.remove(id).unwrap()).collect();
            group.sort.mode = "manual".into();
        }
        if let Some(sort) = update.sort {
            validate_sort(&sort, false)?;
            sort_files(&mut group.files, &sort);
            group.sort = sort;
        }
        if let Some(name) = update.output_name {
            group.output_name = output_name(&name)?;
            group.issues.retain(|i| i.code != "output_name");
        }
    }
    let mut files = HashSet::new();
    for update in request.files {
        if !files.insert(update.file_id.clone()) {
            bail!("文件ID重复");
        }
        let file = plan
            .groups
            .iter_mut()
            .flat_map(|g| &mut g.files)
            .find(|f| f.id == update.file_id)
            .context("文件不属于本计划")?;
        if let Some(angle) = update.rotation_cw {
            validate_angle(angle)?;
            file.rotation_cw = angle;
            file.auto_rotation_change = None;
            file.manual_rotation = true;
            file.reviewed = false;
            if file.kind == "pdf" {
                file.reviewed_pages.fill(false);
            }
        }
        if let Some(angles) = update.page_rotations {
            if file.kind != "pdf" || angles.len() != file.pages as usize {
                bail!("逐页角度数量必须等于PDF页数");
            }
            for angle in &angles {
                validate_angle(*angle)?;
            }
            for (i, angle) in angles.iter().enumerate() {
                if file.page_rotations[i] != *angle {
                    file.reviewed_pages[i] = false;
                }
            }
            file.auto_rotation_change = None;
            file.page_rotations = angles;
            file.manual_rotation = true;
            file.reviewed = false;
        }
        if let Some(reviewed) = update.reviewed {
            file.reviewed = reviewed;
            file.reviewed_pages.fill(reviewed);
        }
        if let Some(reviewed) = update.reviewed_pages {
            if file.kind != "pdf" || reviewed.len() != file.pages as usize {
                bail!("PDF逐页核对状态数量与页数不符");
            }
            file.reviewed_pages = reviewed;
            file.reviewed = file.reviewed_pages.iter().all(|v| *v);
        }
    }
    if let Some(output) = request.output_dir {
        let output = absolute_checked(Path::new(&output), false)?;
        if path_key(&output) == path_key(Path::new(&plan.root)) {
            bail!("输出目录不能与待处理目录相同");
        }
        if output.exists() && !checked_metadata(&output)?.is_dir() {
            bail!("输出位置不是文件夹");
        }
        let positions = virtual_file_positions(&plan);
        for omitted in plan.omitted_sources.drain(..) {
            if let Some(g) = plan.groups.iter_mut().find(|g| g.id == omitted.group_id) {
                restore_file(g, omitted, &positions)?;
            }
        }
        for group in &mut plan.groups {
            if group.sort.mode != "manual" {
                sort_files(&mut group.files, &group.sort);
            }
        }
        let positions = virtual_file_positions(&plan);
        plan.exclusions.retain(|e| e.kind != "output");
        if let Ok(relative) = output.strip_prefix(Path::new(&plan.root)) {
            let relative = path_text(relative)?.replace('\\', "/");
            for g in &mut plan.groups {
                let mut keep = vec![];
                for (position, f) in g.files.drain(..).enumerate() {
                    if f.relative_path.starts_with(&format!("{relative}/")) {
                        plan.omitted_sources.push(OmittedSource {
                            group_id: g.id.clone(),
                            position: positions
                                .get(&g.id)
                                .and_then(|r| r.get(&f.id))
                                .copied()
                                .unwrap_or(position),
                            file: f,
                        });
                    } else {
                        keep.push(f);
                    }
                }
                g.files = keep;
            }
            plan.exclusions.push(Exclusion {
                rule: relative.clone(),
                kind: "output".into(),
                matches: if plan.omitted_sources.is_empty() {
                    vec![]
                } else {
                    vec![format!("{relative}/")]
                },
            });
        }
        plan.output_dir = path_text(&output)?;
    }
    for g in &mut plan.groups {
        refresh_group(g)?;
    }
    refresh_output_issues(&mut plan)?;
    plan.parent_plan_id = Some(original.id.clone());
    plan.id = uuid::Uuid::new_v4().to_string();
    plan.created_at = now();
    Ok(plan)
}
fn validate_angle(angle: u16) -> Result<()> {
    if !matches!(angle, 0 | 90 | 180 | 270) {
        bail!("旋转角度必须是0/90/180/270");
    }
    Ok(())
}
fn selected_groups<'a>(plan: &'a Plan, request: &GenerateRequest) -> Result<Vec<&'a Group>> {
    if plan.schema_version != 1 || plan.mode != "batch_merge" {
        bail!("批量合并计划版本不受支持，请重新扫描");
    }
    if let Some(ids) = &request.voucher_ids {
        let unique: HashSet<_> = ids.iter().collect();
        if ids.is_empty()
            || unique.len() != ids.len()
            || ids
                .iter()
                .any(|id| !plan.groups.iter().any(|g| &g.id == id))
        {
            bail!("请选择本计划内有效且不重复的合并单元");
        }
    }
    Ok(plan
        .groups
        .iter()
        .filter(|g| {
            request
                .voucher_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&g.id))
        })
        .collect())
}
pub(crate) fn validate_generation(plan: &Plan, request: &GenerateRequest) -> Result<u32> {
    let selected = selected_groups(plan, request)?;
    if selected.is_empty() || selected.iter().all(|g| g.empty) {
        bail!("没有可合并的来源文件");
    }
    if plan.issues.iter().any(|i| i.severity == "error")
        || selected.iter().any(|g| {
            g.empty
                || g.issues
                    .iter()
                    .any(|i| i.severity == "error" && i.code != "output_existing")
        })
    {
        bail!("所选合并单元含错误或空组，请检查/移除错误来源或仅选择有效组；旧版PDF检查错误请重新扫描");
    }
    selected.iter().try_fold(0u32, |n, g| {
        n.checked_add(g.pages).context("总页数超过支持范围")
    })
}
pub(crate) fn validate_analysis(plan: &Plan, request: &GenerateRequest) -> Result<u32> {
    if plan.orientation_workflow.consent_version.as_deref() != Some(DISCLAIMER_VERSION)
        || plan.orientation_workflow.consented_at.is_none()
    {
        bail!("自动调整方向前须明确同意当前使用须知与免责声明");
    }
    let count = selected_groups(plan, request)?
        .iter()
        .flat_map(|g| &g.files)
        .filter(|f| f.kind == "image" && f.error.is_none())
        .count();
    if count == 0 {
        bail!("所选合并单元没有可识别的图片；PDF请使用逐页手动旋转");
    }
    u32::try_from(count).context("图片数量超过支持范围")
}
pub fn analysis_revision(original: &Plan, threads: usize) -> Result<Plan> {
    if !(1..=10).contains(&threads) {
        bail!("并行线程数必须为1到10的整数");
    }
    let mut plan = original.clone();
    plan.parent_plan_id = Some(original.id.clone());
    plan.id = uuid::Uuid::new_v4().to_string();
    plan.created_at = now();
    plan.orientation_workflow = Workflow {
        consent_version: Some(DISCLAIMER_VERSION.into()),
        consented_at: Some(now()),
        analysis_completed: false,
        threads,
    };
    Ok(plan)
}

pub fn inspect_existing_outputs(_state_dir: &Path, plan: &mut Plan) -> Result<()> {
    for group in &mut plan.groups {
        group
            .issues
            .retain(|issue| !matches!(issue.code.as_str(), "output_existing" | "output_unsafe"));
        let check = (|| -> Result<()> {
            let directory = absolute_checked(Path::new(&plan.output_dir), false)?;
            if directory.exists() {
                if !checked_metadata(&directory)?.is_dir() {
                    bail!("输出位置不是普通文件夹");
                }
                let path = resolve_output_path(&directory, &group.output_name)?;
                checked_optional(&path)?;
            }
            Ok(())
        })();
        if let Err(error) = check {
            group.issues.push(issue(
                "error",
                "output_unsafe",
                format!("输出路径不安全：{error:#}"),
                Some(group.output_path.clone()),
            ));
        }
    }
    Ok(())
}

fn resolve_output_path(directory: &Path, name: &str) -> Result<PathBuf> {
    let desired = pdf::checked_output_child(directory, name)?;
    let mut matches = vec![];
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().to_lowercase() == name.to_lowercase() {
            matches.push(entry.path());
        }
    }
    if matches.len() > 1 {
        bail!("输出目录中存在多个仅大小写不同的同名文件，无法安全判断覆盖对象；请先改名");
    }
    let path = matches.pop().unwrap_or(desired);
    checked_optional(&path)?;
    Ok(path)
}

fn validate_manifest_owner(
    manifest: &Manifest,
    root: &str,
    group: &Group,
    path: &Path,
) -> Result<()> {
    if manifest.producer != "pdf-workbench"
        || manifest.mode != "batch_merge"
        || manifest.schema_version != 1
        || manifest.root != root
        || manifest.group_key != group.relative_path
        || manifest.output_name != group.output_name
        || path_key(Path::new(&manifest.output_path)) != path_key(path)
    {
        bail!("输出记录/事务不属于当前批量合并单元，拒绝采用或覆盖");
    }
    Ok(())
}

fn check_file(plan: &Plan, file: &File) -> Result<PathBuf> {
    check_file_content(plan, file).with_context(|| {
        format!(
            "来源文件无法访问或已变化，请检查该文件并重新扫描：{}",
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
    pub kind: String,
    #[serde(default)]
    pub pdf_has_signature: bool,
    pub rotation_cw: u16,
    pub page_rotations: Vec<u16>,
    pub reviewed: bool,
    pub reviewed_pages: Vec<bool>,
    pub manual_rotation: bool,
    pub orientation: Option<Orientation>,
    pub orientation_error: Option<String>,
    #[serde(default)]
    pub auto_rotation_change: Option<RotationChange>,
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
    pub index: Option<rename_index::Snapshot>,
    pub rename: Evidence,
    pub orientation_workflow: Workflow,
    pub unreviewed_files: usize,
    pub unreviewed_pages: usize,
    #[serde(default)]
    pub overwrite_authorization: Option<conflicts::OutputAuthorization>,
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
    build_group_with_resolver(state_dir, plan, group, cancel, progress, None)?
        .context("非交互生成不能跳过既存文件")
}

type ConflictResolver<'a> = dyn Fn(&Path, &str) -> Result<conflicts::Decision> + 'a;

fn build_group_with_resolver(
    state_dir: &Path,
    plan: &Plan,
    group: &Group,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u32),
    resolver: Option<&ConflictResolver<'_>>,
) -> Result<Option<BuildResult>> {
    if cancel.load(Ordering::Acquire) {
        bail!("已取消");
    }
    if group.empty
        || group.files.is_empty()
        || group
            .issues
            .iter()
            .any(|i| i.severity == "error" && !(resolver.is_some() && i.code == "output_existing"))
    {
        bail!("分组为空或存在预览错误");
    }
    if output_name(&group.output_name)? != group.output_name {
        bail!("输出文件名不是已验证名称");
    }
    if let Some(index) = &plan.index {
        rename_index::verify(index)?;
    }
    let directory = prepare_output(plan)?;
    let path = resolve_output_path(&directory, &group.output_name)?;
    if path_key(&path) != path_key(Path::new(&group.output_path)) {
        bail!("输出路径与计划不符");
    }
    checked_optional(&path)?;
    let physical_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("输出名称无效")?
        .to_owned();
    for file in plan.groups.iter().flat_map(|g| &g.files) {
        if path_key(&Path::new(&plan.root).join(&file.relative_path)) == path_key(&path) {
            bail!("输出目标与源文件相同，拒绝覆盖");
        }
    }
    let records = OutputRecords::new(state_dir, &path)?;
    // Deserializing a generic manifest is intentionally strict: a voucher
    // journal can never be recovered/adopted through the generic path.
    if records.journal().exists() {
        let pending: Transaction<Manifest> = serde_json::from_slice(&fs::read(records.journal())?)
            .context("输出事务不属于当前批量合并模式，拒绝恢复")?;
        validate_manifest_owner(&pending.manifest, &plan.root, group, &path)?;
    }
    pdf::recover_record_transaction::<Manifest>(&directory, &physical_name, &records)?;
    let mut authorization = None;
    let mut interactive_previous = None;
    if let Some(resolve) = resolver {
        if path.exists() {
            checked_optional(&records.manifest())?;
            let hash = pdf::sha256_file(&path)?;
            match resolve(&path, &hash)? {
                conflicts::Decision::Skip => return Ok(None),
                conflicts::Decision::Overwrite(approved) => {
                    if approved.previous_sha256 != hash
                        || path_key(Path::new(&approved.output_path)) != path_key(&path)
                    {
                        bail!("覆盖授权与当前目标不符，请重新确认");
                    }
                    checked_optional(&path)?;
                    if resolve_output_path(&directory, &group.output_name)? != path
                        || pdf::sha256_file(&path)? != hash
                    {
                        bail!("目标在等待确认期间发生变化，未覆盖；请重试并重新确认");
                    }
                    interactive_previous = Some(hash);
                    authorization = Some(approved);
                }
            }
        }
    }
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
        validate_angle(file.rotation_cw)?;
        for angle in &file.page_rotations {
            validate_angle(*angle)?;
        }
        mappings.push(Mapping {
            kind: file.kind.clone(),
            pdf_has_signature: file.pdf_has_signature,
            rotation_cw: file.rotation_cw,
            page_rotations: file.page_rotations.clone(),
            reviewed: file.reviewed,
            reviewed_pages: file.reviewed_pages.clone(),
            manual_rotation: file.manual_rotation,
            orientation: file.orientation.clone(),
            orientation_error: file.orientation_error.clone(),
            auto_rotation_change: file.auto_rotation_change,
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
        producer: "pdf-workbench".into(),
        mode: "batch_merge".into(),
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
        index: plan.index.clone(),
        rename: group.rename.clone(),
        orientation_workflow: plan.orientation_workflow.clone(),
        unreviewed_files: group.files.iter().filter(|f| needs_review(f)).count(),
        unreviewed_pages: group.files.iter().map(unreviewed_pages).sum(),
        overwrite_authorization: authorization,
    };
    let previous_hash = if resolver.is_some() {
        interactive_previous
    } else if path.exists() {
        let old: Manifest = serde_json::from_slice(
            &fs::read(records.manifest())
                .context("输出已存在且无通用模式生成记录，拒绝覆盖；请改名或换目录")?,
        )
        .context("输出记录不属于通用模式或已损坏，拒绝覆盖")?;
        if old.producer != "pdf-workbench"
            || old.schema_version != 1
            || old.mode != "batch_merge"
            || old.root != plan.root
            || old.group_key != group.relative_path
            || old.output_name != group.output_name
            || path_key(Path::new(&old.output_path)) != path_key(&path)
            || pdf::sha256_file(&path)? != old.output_sha256
        {
            bail!("已有输出被修改或不属于此分组，拒绝覆盖；请改名或换目录");
        }
        if old.sources == manifest.sources
            && old.index == manifest.index
            && old.rename == manifest.rename
            && old.orientation_workflow == manifest.orientation_workflow
        {
            // Store a task-specific snapshot even when bytes are reused.
            manifest.output_sha256 = old.output_sha256;
            progress(manifest.pages);
            return Ok(Some(BuildResult {
                path,
                manifest,
                skipped: true,
            }));
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
            let pages = if file.kind == "pdf" {
                let inspected = crate::generic::inspect_batch_pdf(&source)?;
                if inspected.signature_notice != file.pdf_has_signature {
                    bail!("PDF签名提示来自旧版检查，请重新扫描后再合并；历史计划未修改");
                }
                if inspected.pages != file.pages {
                    bail!("来源PDF页数发生变化，请重新扫描");
                }
                pdf::append_batch_pdf_rotations(
                    &mut doc,
                    root_pages,
                    &mut kids,
                    &source,
                    file.rotation_cw,
                    &file.page_rotations,
                    cancel,
                )?
            } else {
                pdf::append_image(&mut doc, root_pages, &mut kids, &source, file.rotation_cw)?;
                1
            };
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
        if let Some(index) = &plan.index {
            rename_index::verify(index)?;
        }
        // Do not adopt an external edit that happened while we were assembling.
        if resolve_output_path(&directory, &group.output_name)? != path {
            bail!("输出路径或名称在生成期间发生变化，未覆盖，请重新确认");
        }
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
        let replace_existing = previous_hash.is_some();
        let transaction = Transaction {
            temporary: temporary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            output: physical_name.clone(),
            backup: None,
            previous_hash,
            manifest: manifest.clone(),
            replace_existing,
        };
        if let Some(backup) = &transaction.backup {
            pdf::checked_output_child(&directory, backup)?;
        }
        pdf::atomic_json(&records.journal(), &transaction)?;
        pdf::recover_record_transaction::<Manifest>(&directory, &physical_name, &records)?;
        Ok(BuildResult {
            path: path.clone(),
            manifest: manifest.clone(),
            skipped: false,
        })
    })();
    if result.is_err() && !records.journal().exists() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(Some)
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

fn execute_generation(state: &AppState, job: &Job, cancel: &AtomicBool) -> Result<()> {
    let plan = state.batch_plan(&job.plan_id)?;
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
    pdf::atomic_json(&report_dir.join("batch-plan.json"), &plan)?;
    let positions: HashMap<_, _> = work
        .iter()
        .enumerate()
        .map(|(i, g)| (g.id.clone(), i))
        .collect();
    let mut accounted = vec![0u32; work.len()];
    let mut contexts = vec![(); job.threads.min(work.len())];
    let coordinator = Arc::new(ConflictCoordinator::default());
    state
        .output_conflicts
        .lock()
        .unwrap()
        .insert(job.id.clone(), coordinator.clone());
    workers::run_with_stop(
        &work,
        &mut contexts,
        cancel,
        |_, group, progress, stop| {
            let context = conflicts::RequestContext {
                state,
                job,
                cancel,
                stop,
            };
            let resolve =
                |path: &Path, hash: &str| coordinator.request(&context, group, path, hash);
            build_group_with_resolver(&state.dir, &plan, group, cancel, progress, Some(&resolve))
        },
        |event| {
            let result = (|| {
                match event {
                    workers::Event::Started { index } => state.update_job(&job.id, |j| {
                        j.current = format!("最近开始：{}", work[index].name)
                    })?,
                    workers::Event::Progress { index, pages } => {
                        let increment =
                            pages.min(work[index].pages.saturating_sub(accounted[index]));
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
                            Ok(Some(output)) => {
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
                                    needs_review: group.review_count > 0,
                                    rotation_change: None,
                                }
                            }
                            Ok(None) => JobResult {
                                voucher_id: group.id.clone(),
                                folder: group.name.clone(),
                                status: "skipped_existing".into(),
                                output_path: None,
                                pages: 0,
                                error: None,
                                needs_review: false,
                                rotation_change: None,
                            },
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
                    workers::Event::Panicked => {
                        bail!("PDF工作线程异常，已提交成品保留，可重试继续")
                    }
                }
                Ok(())
            })();
            if result.is_err() {
                coordinator.close();
                let _ = state.update_job(&job.id, |job| job.output_conflict = None);
            }
            result
        },
    )
}

fn snapshot(state: &AppState, job: &Job) -> Result<Plan> {
    if !matches!(job.kind.as_str(), "batch_generate" | "batch_analyze") {
        bail!("任务不是通用PDF合并任务");
    }
    let path = state
        .dir
        .join("reports")
        .join(&job.id)
        .join("batch-plan.json");
    let mut plan: Plan = serde_json::from_slice(
        &fs::read(path).context("任务预览快照尚未保存或已丢失，请检查处理记录")?,
    )?;
    if plan.id != job.plan_id || plan.mode != "batch_merge" {
        bail!("任务预览快照与任务不符");
    }
    normalize_review_counts(&mut plan);
    Ok(plan)
}
pub fn report(state: &AppState, job: &Job) -> Result<Value> {
    if job.kind == "batch_scan" {
        return Ok(serde_json::to_value(scan_status(state, &job.id)?)?);
    }
    let plan = snapshot(state, job)?;
    let mut manifests = Vec::new();
    for result in &job.results {
        if job.kind == "batch_generate" && matches!(result.status.as_str(), "completed" | "skipped")
        {
            let group = plan
                .groups
                .iter()
                .find(|g| g.id == result.voucher_id)
                .context("任务结果与预览不符")?;
            let manifest = read_snapshot(state, job, group)?;
            manifests.push(manifest);
        }
    }
    let successful_pages = if job.kind == "batch_generate" {
        job.results
            .iter()
            .filter(|r| matches!(r.status.as_str(), "completed" | "skipped"))
            .map(|r| r.pages)
            .sum::<u32>()
    } else {
        0
    };
    let analyzed_files = if job.kind == "batch_analyze" {
        job.done as usize
    } else {
        0
    };
    let selected = selected_groups(&plan, &job.request)?;
    let unreviewed_files = selected
        .iter()
        .flat_map(|g| &g.files)
        .filter(|f| needs_review(f))
        .count();
    let unreviewed_pages = selected
        .iter()
        .flat_map(|g| &g.files)
        .map(unreviewed_pages)
        .sum::<usize>();
    Ok(
        json!({"mode":"batch_merge","job":job,"plan_id":plan.id,"root":plan.root,"output_dir":plan.output_dir,
        "level":plan.level,"sort":plan.sort,"exclude_rules":plan.exclude_rules,"index":plan.index,"index_error":plan.index_error,"orientation_workflow":plan.orientation_workflow,
        "exclusions":plan.exclusions,"removed_sources":plan.removed_sources,"omitted_sources":plan.omitted_sources,"scan_metrics":plan.scan_metrics,"issues":plan.issues,"groups":plan.groups,"successful_pages":successful_pages,"analyzed_files":analyzed_files,"orientation_summary":job.orientation_summary,"unreviewed_files":unreviewed_files,"unreviewed_pages":unreviewed_pages,"manifests":manifests,
        "review_notice":"自动识别和人工核对均不阻止合并；使用者应核查成品并承担最终判断责任"}),
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
    if manifest.mode != "batch_merge"
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
    let path = resolve_output_path(&directory, &group.output_name)?;
    checked_optional(&path)?;
    if path_key(&path) != path_key(Path::new(&manifest.output_path))
        || pdf::sha256_file(&path)? != manifest.output_sha256
    {
        bail!("成品已变化，无法作为此历史任务下载；请查看最近任务或历史归档");
    }
    Ok(path)
}

/// Revalidate the exact bytes returned to the desktop/download layer.
pub fn read_output(state: &AppState, job: &Job, group_id: &str) -> Result<Vec<u8>> {
    let path = output(state, job, group_id)?;
    let plan = snapshot(state, job)?;
    let group = plan
        .groups
        .iter()
        .find(|g| g.id == group_id)
        .context("分组不属于此任务")?;
    let manifest = read_snapshot(state, job, group)?;
    let bytes = fs::read(path)?;
    if hex::encode(sha2::Sha256::digest(&bytes)) != manifest.output_sha256 {
        bail!("读取成品期间文件已变化，无法下载此历史版本");
    }
    Ok(bytes)
}

pub(crate) fn requires_inference(plan: &Plan, request: &GenerateRequest) -> Result<bool> {
    Ok(selected_groups(plan, request)?
        .iter()
        .flat_map(|g| &g.files)
        .any(|f| {
            f.kind == "image"
                && f.error.is_none()
                && !f.manual_rotation
                && !f.reviewed
                && f.orientation
                    .as_ref()
                    .is_none_or(|o| o.model != orientation::GENERAL_MODEL_ID)
        }))
}

pub(crate) fn execute(state: &AppState, job: &Job, cancel: &AtomicBool) -> Result<()> {
    match job.kind.as_str() {
        "batch_scan" => execute_scan(state, job, cancel),
        "batch_generate" => execute_generation(state, job, cancel),
        "batch_analyze" => execute_analysis(state, job, cancel),
        _ => bail!("未知批量合并任务类型"),
    }
}

type AnalysisResult = std::result::Result<Option<Orientation>, (bool, anyhow::Error)>;

fn finalize_analysis(plan: &Plan, file: &File, result: AnalysisResult) -> AnalysisResult {
    // A changed source is a hard error even if model inference also failed.
    // Optional-OCR handling must never conceal content/fingerprint failures.
    check_file(plan, file).map_err(|error| (false, error))?;
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnalysisOutcome {
    Changed(RotationChange),
    Unchanged,
    Skipped,
    Failed,
}
fn apply_analysis_result(file: &mut File, result: AnalysisResult) -> AnalysisOutcome {
    file.auto_rotation_change = None;
    match result {
        Ok(Some(direction)) => {
            let from = file.rotation_cw;
            let to = direction.rotation_cw;
            let resolved = !direction.needs_review;
            file.orientation = Some(direction);
            file.orientation_error = None;
            if resolved && from != to {
                let change = RotationChange {
                    from_cw: from,
                    to_cw: to,
                };
                file.rotation_cw = to;
                file.auto_rotation_change = Some(change);
                AnalysisOutcome::Changed(change)
            } else {
                AnalysisOutcome::Unchanged
            }
        }
        Ok(None) => AnalysisOutcome::Skipped,
        Err((optional, error)) => {
            let message = format!(
                "{}：{error:#}",
                if optional {
                    "方向识别失败，可手动修正或按当前方向合并"
                } else {
                    "来源文件校验失败，请重新扫描"
                }
            );
            if optional {
                file.orientation_error = Some(message);
            } else {
                file.error = Some(message);
            }
            AnalysisOutcome::Failed
        }
    }
}

fn execute_analysis(state: &AppState, job: &Job, cancel: &AtomicBool) -> Result<()> {
    let mut plan = state.batch_plan(&job.plan_id)?;
    validate_analysis(&plan, &job.request)?;
    let selected: HashSet<_> = selected_groups(&plan, &job.request)?
        .iter()
        .flat_map(|group| &group.files)
        .filter(|file| file.kind == "image")
        .map(|file| file.id.clone())
        .collect();
    {
        let mut data = state.data.lock().unwrap();
        let current = data
            .batch_plans
            .get_mut(&plan.id)
            .context("批量合并计划不存在")?;
        for file in current.groups.iter_mut().flat_map(|group| &mut group.files) {
            if selected.contains(&file.id) {
                file.auto_rotation_change = None;
            }
        }
        for group in &mut current.groups {
            refresh_group(group)?;
        }
        state.save_batch_plan(current)?;
        plan = current.clone();
    }

    let work: Vec<_> = selected_groups(&plan, &job.request)?
        .iter()
        .flat_map(|g| &g.files)
        .filter(|f| f.kind == "image" && f.error.is_none())
        .cloned()
        .collect();
    let report_dir = state.dir.join("reports").join(&job.id);
    fs::create_dir_all(&report_dir)?;
    pdf::atomic_json(&report_dir.join("batch-plan.json"), &plan)?;
    let count = job.threads.min(work.len());
    let mut engines: Vec<Option<Engine>> = Vec::with_capacity(count);
    if requires_inference(&plan, &job.request)? {
        let warm = state.engine.lock().unwrap().take();
        engines.push(Some(match warm {
            Some(engine) => engine,
            None => Engine::new(&state.model_path, state.runtime_path.as_deref())?,
        }));
        for _ in 1..count {
            engines.push(Some(Engine::new(
                &state.model_path,
                state.runtime_path.as_deref(),
            )?));
        }
    } else {
        engines.resize_with(count, || None);
    }
    let positions: HashMap<_, _> = work
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.clone(), i))
        .collect();
    let mut completed = 0;
    let mut summary = AnalysisSummary::default();
    let outcome = workers::run(
        &work,
        &mut engines,
        cancel,
        |engine, file, _| {
            let path = check_file(&plan, file).map_err(|e| (false, e))?;
            let result = (|| {
                if file.manual_rotation || file.reviewed {
                    return Ok(None);
                }
                if let Some(cached) = file
                    .orientation
                    .as_ref()
                    .filter(|o| o.model == orientation::GENERAL_MODEL_ID)
                {
                    return Ok(Some(cached.clone()));
                }
                let bytes = fs::read(path).map_err(|e| (false, e.into()))?;
                if hex::encode(sha2::Sha256::digest(&bytes)) != file.sha256 {
                    return Err((
                        false,
                        anyhow::anyhow!("读取识别来源期间内容已变化，请重新扫描"),
                    ));
                }
                let image = orientation::load_image_bytes(bytes).map_err(|e| (false, e))?;
                let direction = engine
                    .as_mut()
                    .context("方向模型未就绪")
                    .map_err(|e| (true, e))?
                    .predict_generic_image(&image)
                    .map_err(|e| (true, e))?;
                Ok(Some(direction))
            })();
            finalize_analysis(&plan, file, result)
        },
        |event| {
            match event {
                workers::Event::Started { .. } => state.update_job(&job.id, |job| {
                    job.current = format!("方向识别：{}/{}", job.done, job.total);
                })?,
                workers::Event::Finished { index, result } => {
                    let source = &work[index];
                    let classification = {
                        let mut data = state.data.lock().unwrap();
                        let current = data
                            .batch_plans
                            .get_mut(&plan.id)
                            .context("批量合并计划不存在")?;
                        let file = current
                            .groups
                            .iter_mut()
                            .flat_map(|g| &mut g.files)
                            .find(|f| f.id == source.id)
                            .context("图片不属于此计划")?;
                        let outcome = apply_analysis_result(file, result);
                        for group in &mut current.groups {
                            refresh_group(group)?;
                        }
                        completed += 1;
                        if completed % 10 == 0 {
                            state.save_batch_plan(current)?;
                        }
                        outcome
                    };
                    match classification {
                        AnalysisOutcome::Changed(_) => summary.changed += 1,
                        AnalysisOutcome::Unchanged => summary.unchanged += 1,
                        AnalysisOutcome::Skipped => summary.skipped += 1,
                        AnalysisOutcome::Failed => summary.failed += 1,
                    }
                    state.update_job(&job.id, |job| {
                        job.done = (job.done + 1).min(job.total);
                        job.orientation_summary = Some(summary);
                        job.current = format!("方向识别：{}/{}", job.done, job.total);
                        if let AnalysisOutcome::Changed(change) = classification {
                            job.results.push(JobResult {
                                voucher_id: source.id.clone(),
                                folder: source.relative_path.clone(),
                                status: "completed".into(),
                                output_path: None,
                                pages: 1,
                                error: None,
                                needs_review: true,
                                rotation_change: Some(change),
                            });
                            job.results
                                .sort_by_key(|result| positions[&result.voucher_id]);
                        }
                    })?;
                }
                workers::Event::Panicked => {
                    summary.failed += 1;
                    state.update_job(&job.id, |job| {
                        job.done = (job.done + 1).min(job.total);
                        job.orientation_summary = Some(summary);
                    })?;
                    bail!("方向识别工作线程异常，已完成结果保留；可重试或按当前方向合并");
                }
                workers::Event::Progress { .. } => {}
            }
            Ok(())
        },
    );
    if outcome.is_ok() {
        if let Some(engine) = engines.into_iter().flatten().next() {
            *state.engine.lock().unwrap() = Some(engine);
        }
    }
    {
        let mut data = state.data.lock().unwrap();
        let current = data
            .batch_plans
            .get_mut(&plan.id)
            .context("批量合并计划不存在")?;
        current.orientation_workflow.analysis_completed =
            outcome.is_ok() && !cancel.load(Ordering::Acquire);
        state.save_batch_plan(current)?;
        pdf::atomic_json(&report_dir.join("batch-plan.json"), current)?;
    }
    outcome?;
    if summary.failed > 0 {
        bail!(
            "{}个文件自动识别失败；当前角度未因失败改变，可检查来源或手动修正，详情保留在计划中",
            summary.failed
        );
    }
    Ok(())
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Initialize {
    root: String,
    #[serde(default)]
    level: u16,
    #[serde(default = "default_rules")]
    exclude_rules: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {
    path: String,
    sheet: Option<String>,
    header_row: Option<u32>,
}
#[derive(Deserialize)]
struct FileQuery {
    #[serde(default)]
    thumbnail: bool,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/initialize", post(api_initialize))
        .route("/preview", post(api_preview))
        .route("/scans", post(api_start_scan))
        .route("/scans/{id}", get(api_scan_status))
        .route("/index/inspect", post(api_inspect))
        .route("/scan", post(api_scan))
        .route("/plans", get(api_plans))
        .route("/plans/{id}", get(api_plan))
        .route("/plans/{id}/edit", post(api_edit))
        .route("/plans/{id}/analyze", post(api_analyze))
        .route("/plans/{id}/generate", post(api_generate))
        .route("/plans/{id}/files/{file_id}", get(api_file))
        .route("/plans/{id}/open-output", post(api_open_output))
        .route("/jobs", get(api_jobs))
        .route("/jobs/{id}/output-conflict", post(conflicts::api_resolve))
}
pub fn start_scan(state: &Arc<AppState>, request: ScanRequest) -> Result<Job> {
    validate_sort(&request.sort, false)?;
    if request.root.trim().is_empty() {
        bail!("请选择待处理文件夹");
    }
    let mut data = state.data.lock().unwrap();
    if state.shutting_down.load(Ordering::Acquire) {
        bail!("工作台正在退出，不能启动扫描");
    }
    if data.jobs.values().any(Job::active) {
        bail!("已有任务运行，请等待或取消后重试");
    }
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: uuid::Uuid::new_v4().to_string(),
        kind: "batch_scan".into(),
        status: "queued".into(),
        created_at: now(),
        updated_at: now(),
        total: 0,
        done: 0,
        current: "等待扫描".into(),
        error: None,
        results: vec![],
        request: GenerateRequest::default(),
        threads: DEFAULT_THREADS,
        output_conflict: None,
        orientation_summary: None,
    };
    let report_dir = state.dir.join("reports").join(&job.id);
    fs::create_dir_all(&report_dir)?;
    pdf::atomic_json(&report_dir.join("scan-request.json"), &request)?;
    let progress = ScanProgress {
        id: job.id.clone(),
        status: job.status.clone(),
        plan_id: None,
        phase: "enumerating".into(),
        done: 0,
        total: 0,
        elapsed_ms: 0,
        current_file: None,
        timings: ScanTimings::default(),
        counts: ScanCounts::default(),
        logs: vec![],
        error: None,
    };
    pdf::atomic_json(&report_dir.join("scan-progress.json"), &progress)?;
    state.save_job(&job)?;
    state
        .scan_progress
        .lock()
        .unwrap()
        .insert(job.id.clone(), progress);
    let cancel = Arc::new(AtomicBool::new(false));
    data.jobs.insert(job.id.clone(), job.clone());
    data.cancels.insert(job.id.clone(), cancel.clone());
    drop(data);
    state.launch_job(job.clone(), cancel);
    Ok(job)
}
pub fn scan_status(state: &AppState, id: &str) -> Result<ScanProgress> {
    let job = state.job(id)?;
    if job.kind != "batch_scan" {
        bail!("该任务不是扫描任务");
    }
    let cached = state.scan_progress.lock().unwrap().get(id).cloned();
    let mut progress = match cached {
        Some(value) => value,
        None => serde_json::from_slice(&fs::read(
            state
                .dir
                .join("reports")
                .join(id)
                .join("scan-progress.json"),
        )?)?,
    };
    progress.status = job.status.clone();
    progress.error = job.error.clone();
    progress.done = job.done;
    progress.total = job.total;
    if job.active() {
        if let Ok(started) = chrono::DateTime::parse_from_rfc3339(&job.created_at) {
            progress.elapsed_ms = progress.elapsed_ms.max(
                chrono::Utc::now()
                    .signed_duration_since(started)
                    .num_milliseconds()
                    .max(0) as u64,
            );
        }
    }
    if job.status == "completed" {
        progress.plan_id = Some(job.plan_id);
        progress.phase = "completed".into();
    } else {
        progress.plan_id = None;
    }
    Ok(progress)
}
fn execute_scan(state: &AppState, job: &Job, cancel: &AtomicBool) -> Result<()> {
    let directory = state.dir.join("reports").join(&job.id);
    let request = serde_json::from_slice(&fs::read(directory.join("scan-request.json"))?)?;
    let result = scan_with_progress(request, Some(&state.dir), cancel, |status| {
        let mut current = status.clone();
        current.id = job.id.clone();
        state
            .scan_progress
            .lock()
            .unwrap()
            .insert(job.id.clone(), current);
        // Live progress is in memory; do not fsync a multi-KB job for every file.
        let mut data = state.data.lock().unwrap();
        let live = data.jobs.get_mut(&job.id).context("扫描任务不存在")?;
        live.done = status.done;
        live.total = status.total;
        live.current = format!("扫描资料：{}/{}", status.done, status.total);
        live.updated_at = now();
        Ok(())
    });
    let outcome = result.and_then(|mut plan| {
        plan.id = job.plan_id.clone();
        let mut data = state.data.lock().unwrap();
        if cancel.load(Ordering::Acquire) || state.shutting_down.load(Ordering::Acquire) {
            bail!("扫描已取消，原有计划保留");
        }
        state.save_batch_plan(&plan)?;
        data.batch_plans.insert(plan.id.clone(), plan);
        Ok(())
    });
    // Persist the last counters even when cancellation races with publication.
    let progress = scan_status(state, &job.id)?;
    pdf::atomic_json(&directory.join("scan-progress.json"), &progress)?;
    outcome
}

async fn api_preview(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScanRequest>,
) -> ApiResult<Json<Initialization>> {
    Ok(Json(
        tokio::task::spawn_blocking(move || preview(request, Some(&state.dir))).await??,
    ))
}
async fn api_start_scan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScanRequest>,
) -> ApiResult<Json<ScanProgress>> {
    let job = start_scan(&state, request)?;
    Ok(Json(scan_status(&state, &job.id)?))
}
async fn api_scan_status(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<ScanProgress>> {
    Ok(Json(scan_status(&state, &id)?))
}

pub fn resolve_output_conflict(
    state: &AppState,
    job_id: &str,
    conflict_id: &str,
    action: &str,
    apply_to_batch: bool,
) -> Result<Job> {
    let action = match action {
        "skip" => conflicts::Action::Skip,
        "overwrite" => conflicts::Action::Overwrite,
        _ => bail!("同名文件操作只能是skip或overwrite"),
    };
    let coordinator = state
        .output_conflicts
        .lock()
        .unwrap()
        .get(job_id)
        .cloned()
        .context("该任务当前没有等待处理的同名文件")?;
    coordinator.resolve(
        state,
        job_id,
        conflicts::ResolveRequest {
            conflict_id: conflict_id.into(),
            action,
            apply_to_batch,
        },
    )
}

async fn api_initialize(
    State(state): State<Arc<AppState>>,
    Json(request): Json<Initialize>,
) -> ApiResult<Json<Initialization>> {
    Ok(Json(
        tokio::task::spawn_blocking(move || {
            preview(
                ScanRequest {
                    root: request.root,
                    level: request.level,
                    exclude_rules: request.exclude_rules,
                    index: None,
                    sort: Sort::default(),
                    output_names: vec![],
                },
                Some(&state.dir),
            )
        })
        .await??,
    ))
}
async fn api_inspect(Json(request): Json<Inspect>) -> ApiResult<Json<rename_index::Inspection>> {
    Ok(Json(
        tokio::task::spawn_blocking(move || {
            rename_index::inspect(
                Path::new(&request.path),
                request.sheet.as_deref(),
                request.header_row,
            )
        })
        .await??,
    ))
}
async fn api_scan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScanRequest>,
) -> ApiResult<Json<Plan>> {
    let dir = state.dir.clone();
    let plan = tokio::task::spawn_blocking(move || scan(request, Some(&dir))).await??;
    Ok(Json(state.insert_batch_plan(plan)?))
}
#[derive(Deserialize)]
struct PlanQuery {
    #[serde(default)]
    summary: bool,
}
async fn api_plans(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PlanQuery>,
) -> Json<Value> {
    let data = state.data.lock().unwrap();
    let mut plans: Vec<_> = data.batch_plans.values().collect();
    plans.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if query.summary {
        Json(Value::Array(plans.iter().map(|p|json!({"id":p.id,"parent_plan_id":p.parent_plan_id,"created_at":p.created_at,"root":p.root,"output_dir":p.output_dir,"level":p.level,"group_count":p.groups.len(),"file_count":p.groups.iter().map(|g|g.files.len()).sum::<usize>(),"pages":p.groups.iter().map(|g|g.pages as u64).sum::<u64>(),"index":p.index,"orientation_workflow":p.orientation_workflow,"removed_file_count":p.removed_sources.len()})).collect()))
    } else {
        Json(serde_json::to_value(plans).expect("serializable saved batch plans"))
    }
}

async fn api_plan(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Plan>> {
    Ok(Json(state.batch_plan(&id)?))
}
async fn api_edit(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
    Json(request): Json<EditRequest>,
) -> ApiResult<Json<Plan>> {
    let original = state.batch_plan(&id)?;
    let dir = state.dir.clone();
    let plan = tokio::task::spawn_blocking(move || {
        let mut plan = edit(&original, request)?;
        inspect_existing_outputs(&dir, &mut plan)?;
        Ok::<_, anyhow::Error>(plan)
    })
    .await??;
    Ok(Json(state.insert_batch_plan(plan)?))
}
async fn api_analyze(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
    Json(request): Json<Analyze>,
) -> ApiResult<Json<Job>> {
    if !request.accept_disclaimer {
        return Err(anyhow::anyhow!("请先明确同意使用须知与免责声明").into());
    }
    let plan = analysis_revision(&state.batch_plan(&id)?, request.threads)?;
    let selection = GenerateRequest {
        voucher_ids: request.group_ids,
        allow_unreviewed: false,
    };
    validate_analysis(&plan, &selection)?;
    if requires_inference(&plan, &selection)? && state.model_error.is_some() {
        return Err(anyhow::anyhow!(
            "方向模型未就绪：{}；可跳过自动识别直接按当前方向合并",
            state.model_error.as_deref().unwrap_or_default()
        )
        .into());
    }
    let plan = state.insert_batch_plan(plan)?;
    Ok(Json(state.start_batch(
        &plan.id,
        "batch_analyze",
        selection,
        request.threads,
    )?))
}
async fn api_generate(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
    Json(request): Json<Generate>,
) -> ApiResult<Json<Job>> {
    if !(1..=10).contains(&request.threads) {
        return Err(anyhow::anyhow!("并行线程数必须为1到10的整数").into());
    }
    let mut plan = state.batch_plan(&id)?;
    if let Some(output_dir) = request.output_dir {
        plan = edit(
            &plan,
            EditRequest {
                output_dir: Some(output_dir),
                ..Default::default()
            },
        )?;
        inspect_existing_outputs(&state.dir, &mut plan)?;
        plan = state.insert_batch_plan(plan)?;
    }
    Ok(Json(state.start_batch(
        &plan.id,
        "batch_generate",
        GenerateRequest {
            voucher_ids: request.group_ids,
            allow_unreviewed: false,
        },
        request.threads,
    )?))
}
async fn api_jobs(State(state): State<Arc<AppState>>) -> Json<Vec<Job>> {
    let mut jobs: Vec<_> = state
        .data
        .lock()
        .unwrap()
        .jobs
        .values()
        .filter(|j| j.kind.starts_with("batch_"))
        .cloned()
        .collect();
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(jobs)
}
async fn api_file(
    State(state): State<Arc<AppState>>,
    ApiPath((id, file_id)): ApiPath<(String, String)>,
    Query(query): Query<FileQuery>,
) -> ApiResult<Response> {
    let plan = state.batch_plan(&id)?;
    let file = plan
        .groups
        .iter()
        .flat_map(|g| &g.files)
        .find(|f| f.id == file_id)
        .context("文件ID不属于此计划")?
        .clone();
    let (bytes, mime) = tokio::task::spawn_blocking(move || {
        let path = check_file(&plan, &file)?;
        let original = fs::read(&path)?;
        if hex::encode(sha2::Sha256::digest(&original)) != file.sha256 {
            bail!("来源文件读取期间已变化，请重新扫描");
        }
        let bytes = if query.thumbnail && file.kind == "image" {
            let image = orientation::load_image_bytes(original)?;
            let image = if image.width() > 1600 || image.height() > 1600 {
                image.thumbnail(1600, 1600)
            } else {
                image
            };
            let rgb = orientation::composite_white(&image);
            let mut bytes = vec![];
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 88)
                .encode_image(&image::DynamicImage::ImageRgb8(rgb))?;
            check_file(&plan, &file)?;
            bytes
        } else {
            original
        };
        let mime = if file.kind == "pdf" {
            "application/pdf"
        } else if query.thumbnail {
            "image/jpeg"
        } else {
            match Path::new(&file.name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "bmp" => "image/bmp",
                "webp" => "image/webp",
                _ => "image/tiff",
            }
        };
        Ok::<_, anyhow::Error>((bytes, mime))
    })
    .await??;
    Ok(([(header::CONTENT_TYPE, mime)], Body::from(bytes)).into_response())
}
async fn api_open_output(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Value>> {
    let plan = state.batch_plan(&id)?;
    let directory = absolute_checked(Path::new(&plan.output_dir), true)?;
    if !checked_metadata(&directory)?.is_dir() {
        return Err(anyhow::anyhow!("输出目录不存在，请先执行合并").into());
    }
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
mod tests {
    use super::*;

    #[test]
    fn source_change_wins_over_an_optional_ocr_failure() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("page.png");
        image::RgbImage::from_pixel(20, 30, image::Rgb([255, 255, 255]))
            .save(&source)
            .unwrap();
        let plan = scan(
            ScanRequest {
                root: root.to_str().unwrap().into(),
                level: 0,
                exclude_rules: String::new(),
                index: None,
                sort: Sort::default(),
                output_names: vec![],
            },
            None,
        )
        .unwrap();
        let file = &plan.groups[0].files[0];
        let unchanged = finalize_analysis(
            &plan,
            file,
            Err((true, anyhow::anyhow!("synthetic OCR failure"))),
        );
        assert!(unchanged.unwrap_err().0);
        fs::write(&source, b"changed during an unsuccessful inference").unwrap();
        let changed = finalize_analysis(
            &plan,
            file,
            Err((true, anyhow::anyhow!("synthetic OCR failure"))),
        );
        let (optional, error) = changed.unwrap_err();
        assert!(!optional);
        assert!(error.to_string().contains("来源文件"));
    }
    #[test]
    fn scanning_cancel_drains_workers_without_returning_a_partial_plan() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for i in 0..20 {
            image::RgbImage::from_pixel(20, 30, image::Rgb([255, 255, 255]))
                .save(root.join(format!("{i}.png")))
                .unwrap();
        }
        let cancel = AtomicBool::new(false);
        let mut done = 0;
        let result = scan_with_progress(
            ScanRequest {
                root: root.to_str().unwrap().into(),
                level: 0,
                exclude_rules: String::new(),
                index: None,
                sort: Sort::default(),
                output_names: vec![],
            },
            None,
            &cancel,
            |status| {
                assert!(status.done >= done);
                done = status.done;
                if status.done > 0 {
                    cancel.store(true, Ordering::Release);
                }
                Ok(())
            },
        );
        assert!(result.unwrap_err().to_string().contains("取消"));
        assert!(done > 0 && done < 20);
        assert_eq!(fs::read_dir(root).unwrap().count(), 20);
    }
    #[test]
    fn shutdown_at_publication_keeps_the_final_scan_counters_without_a_plan() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("input");
        fs::create_dir(&root).unwrap();
        image::RgbImage::from_pixel(20, 30, image::Rgb([255, 255, 255]))
            .save(root.join("page.png"))
            .unwrap();
        let state =
            AppState::load(base.join("records"), &base.join("missing-model"), None).unwrap();
        let job = Job {
            id: uuid::Uuid::new_v4().to_string(),
            plan_id: uuid::Uuid::new_v4().to_string(),
            kind: "batch_scan".into(),
            status: "running".into(),
            created_at: now(),
            updated_at: now(),
            total: 0,
            done: 0,
            current: String::new(),
            error: None,
            results: vec![],
            request: GenerateRequest::default(),
            threads: 4,
            output_conflict: None,
            orientation_summary: None,
        };
        let directory = state.dir.join("reports").join(&job.id);
        fs::create_dir_all(&directory).unwrap();
        pdf::atomic_json(
            &directory.join("scan-request.json"),
            &ScanRequest {
                root: root.to_str().unwrap().into(),
                level: 0,
                exclude_rules: String::new(),
                index: None,
                sort: Sort::default(),
                output_names: vec![],
            },
        )
        .unwrap();
        state
            .data
            .lock()
            .unwrap()
            .jobs
            .insert(job.id.clone(), job.clone());
        // Models a close between the shutdown guard and cancel-flag delivery.
        state.shutting_down.store(true, Ordering::Release);
        let result = execute_scan(&state, &job, &AtomicBool::new(false));
        assert!(result.unwrap_err().to_string().contains("取消"));
        let saved: ScanProgress =
            serde_json::from_slice(&fs::read(directory.join("scan-progress.json")).unwrap())
                .unwrap();
        assert_eq!((saved.done, saved.total), (1, 1));
        assert_eq!(saved.counts.hash_calls, 1);
        assert!(state.data.lock().unwrap().batch_plans.is_empty());
        assert_eq!(
            fs::read_dir(state.dir.join("batch-plans")).unwrap().count(),
            0
        );
    }
}
