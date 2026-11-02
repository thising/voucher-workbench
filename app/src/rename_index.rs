//! Optional, explicitly selected spreadsheet columns for batch output names.
//! Only the bytes inspected here are fingerprinted; no spreadsheet application
//! or workbook formula execution is involved.
use anyhow::{bail, Context, Result};
use calamine::{open_workbook_auto_from_rs, Data, Range, Reader};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io::Cursor, path::Path};

fn auto() -> String {
    "auto".into()
}
fn index_name() -> String {
    "index".into()
}
fn fallback() -> String {
    "fallback".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub path: String,
    pub sheet: String,
    pub index_column: usize,
    pub name_column: usize,
    pub header_row: u32,
    #[serde(default = "auto")]
    pub match_rule: String,
    #[serde(default = "index_name")]
    pub naming: String,
    #[serde(default = "fallback")]
    pub duplicates: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    pub index: usize,
    pub letter: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sheet {
    pub name: String,
    pub rows: u32,
    pub columns: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreviewRow {
    pub row: u32,
    pub values: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inspection {
    pub path: String,
    pub sha256: String,
    pub sheets: Vec<Sheet>,
    pub sheet: String,
    pub sheet_error: Option<String>,
    pub header_row: u32,
    pub columns: Vec<Column>,
    pub preview_rows: Vec<PreviewRow>,
    pub suggested_index_column: Option<usize>,
    pub suggested_name_column: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub config: Config,
    pub sha256: String,
    pub rows: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Evidence {
    pub status: String,
    pub rule: Option<String>,
    pub key: Option<String>,
    pub rows: Vec<u32>,
    pub value: Option<String>,
    pub message: String,
}
impl Evidence {
    pub fn folder() -> Self {
        Self {
            status: "folder".into(),
            rule: None,
            key: None,
            rows: vec![],
            value: None,
            message: "未使用重命名索引，按目标文件夹命名".into(),
        }
    }
}
#[derive(Clone, Debug)]
struct Entry {
    key: String,
    value: String,
    row: u32,
}
pub struct Index {
    pub snapshot: Snapshot,
    entries: Vec<Entry>,
}

fn cell(cell: Option<&Data>) -> String {
    match cell {
        None | Some(Data::Empty) => String::new(),
        Some(Data::Error(_)) => String::new(),
        Some(Data::String(s)) => s.trim().trim_matches('\u{feff}').to_owned(),
        Some(c) => c.to_string().trim().to_owned(),
    }
}
fn key(value: &str) -> String {
    value
        .trim()
        .trim_matches('\u{feff}')
        .replace('\\', "/")
        .to_lowercase()
}
fn header_key(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-' | '\u{feff}'))
        .flat_map(char::to_lowercase)
        .collect()
}
fn is_index_header(value: &str) -> bool {
    matches!(
        header_key(value).as_str(),
        "文件夹"
            | "文件夹名"
            | "文件夹名称"
            | "目录"
            | "目录名"
            | "文件夹索引"
            | "索引"
            | "编号"
            | "凭证编号"
            | "index"
            | "folder"
            | "foldername"
            | "id"
            | "code"
    )
}
fn is_name_header(value: &str) -> bool {
    matches!(
        header_key(value).as_str(),
        "名称"
            | "文件名"
            | "文件名称"
            | "重命名"
            | "重命名文件名"
            | "pdf名称"
            | "pdf文件名"
            | "科目名称"
            | "name"
            | "filename"
            | "pdfname"
            | "title"
    )
}
fn letter(mut index: usize) -> String {
    let mut result = String::new();
    loop {
        result.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    result
}
type Workbook = calamine::Sheets<Cursor<Vec<u8>>>;
fn load(path: &Path) -> Result<(Vec<u8>, Workbook)> {
    crate::generic::absolute_checked(path, true)?;
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("xls") || s.eq_ignore_ascii_case("xlsx"))
    {
        bail!("重命名索引支持XLS或XLSX文件");
    }
    let bytes = fs::read(path).context("无法读取重命名索引文件")?;
    let book = open_workbook_auto_from_rs(Cursor::new(bytes.clone()))
        .context("重命名索引无法解析，请选择有效的XLS/XLSX文件")?;
    Ok((bytes, book))
}
fn header(range: &Range<Data>, requested: Option<u32>) -> Result<u32> {
    let start = range.start().map(|(r, _)| r).unwrap_or(0);
    if let Some(row) = requested {
        if row == 0 || row <= start || row > start + range.height() as u32 {
            bail!("表头行不在所选工作表内容范围内");
        }
        return Ok(row);
    }
    let first = range
        .rows()
        .enumerate()
        .find(|(_, r)| r.iter().any(|v| !cell(Some(v)).is_empty()))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let best = range
        .rows()
        .take(50)
        .enumerate()
        .filter_map(|(i, row)| {
            let values: Vec<_> = row.iter().map(|c| cell(Some(c))).collect();
            let has_index = values.iter().any(|v| is_index_header(v));
            let has_name = values.iter().any(|v| is_name_header(v));
            (has_index && has_name).then_some(i)
        })
        .next()
        .unwrap_or(first);
    Ok(start + best as u32 + 1)
}
pub fn inspect(
    path: &Path,
    selected: Option<&str>,
    requested_header: Option<u32>,
) -> Result<Inspection> {
    let (bytes, mut book) = load(path)?;
    let names = book.sheet_names().to_vec();
    let selected = selected
        .map(str::to_owned)
        .or_else(|| names.first().cloned())
        .context("索引文件没有工作表")?;
    if !names.contains(&selected) {
        bail!("索引文件中不存在指定工作表，请重新选择");
    }
    let mut sheets = vec![];
    let mut range = None;
    let mut selected_error = None;
    for name in names {
        let r = match book.worksheet_range(&name) {
            Ok(range) => range,
            Err(error) => {
                if name == selected {
                    selected_error =
                        Some(format!("无法读取工作表{name}：{error}；请选择其他工作表"));
                }
                sheets.push(Sheet {
                    name,
                    rows: 0,
                    columns: 0,
                });
                continue;
            }
        };
        sheets.push(Sheet {
            name: name.clone(),
            rows: r.end().map(|(r, _)| r + 1).unwrap_or(0),
            columns: r.end().map(|(_, c)| c as usize + 1).unwrap_or(0),
        });
        if name == selected {
            range = Some(r);
        }
    }
    use sha2::Digest;
    let sha256 = hex::encode(sha2::Sha256::digest(&bytes));
    let path = crate::generic::absolute_checked(path, true)?
        .to_string_lossy()
        .into_owned();
    if range.as_ref().is_none_or(Range::is_empty) {
        return Ok(Inspection {
            path,
            sha256,
            sheets,
            sheet: selected,
            sheet_error: Some(
                selected_error.unwrap_or_else(|| "所选工作表为空，请选择其他工作表".into()),
            ),
            header_row: requested_header.unwrap_or(1),
            columns: vec![],
            preview_rows: vec![],
            suggested_index_column: None,
            suggested_name_column: None,
        });
    }
    let range = range.context("工作表不可读取")?;
    let header_row = header(&range, requested_header)?;
    let start_col = range.start().map(|(_, c)| c as usize).unwrap_or(0);
    let start_row = range.start().map(|(r, _)| r).unwrap_or(0);
    let row = range
        .rows()
        .nth((header_row - 1 - start_row) as usize)
        .context("表头行不存在")?;
    let columns: Vec<_> = row
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let name = cell(Some(c));
            let index = start_col + i;
            Column {
                index,
                letter: letter(index),
                name: if name.is_empty() {
                    format!("{}列（无标题）", letter(index))
                } else {
                    name
                },
            }
        })
        .collect();
    let suggested_index_column = columns
        .iter()
        .find(|c| is_index_header(&c.name))
        .map(|c| c.index);
    let suggested_name_column = columns
        .iter()
        .find(|c| is_name_header(&c.name) && Some(c.index) != suggested_index_column)
        .map(|c| c.index);
    let preview_rows = range
        .rows()
        .enumerate()
        .skip((header_row - start_row) as usize)
        .take(12)
        .map(|(i, r)| PreviewRow {
            row: start_row + i as u32 + 1,
            values: std::iter::repeat_n(String::new(), start_col)
                .chain(r.iter().map(|c| cell(Some(c))))
                .collect(),
        })
        .collect();
    Ok(Inspection {
        path,
        sha256,
        sheets,
        sheet: selected,
        sheet_error: None,
        header_row,
        columns,
        preview_rows,
        suggested_index_column,
        suggested_name_column,
    })
}
impl Index {
    pub fn read(mut config: Config) -> Result<Self> {
        if !matches!(
            config.match_rule.as_str(),
            "auto" | "exact" | "voucher_code"
        ) || !matches!(config.naming.as_str(), "index" | "folder_index")
            || !matches!(config.duplicates.as_str(), "fallback" | "first")
        {
            bail!("索引匹配、命名或重复项策略无效");
        }
        if config.index_column == config.name_column {
            bail!("文件夹索引列和重命名列不能相同");
        }
        let path = crate::generic::absolute_checked(Path::new(&config.path), true)?;
        config.path = path.to_string_lossy().into_owned();
        let (bytes, mut book) = load(&path)?;
        let range = book
            .worksheet_range(&config.sheet)
            .context("无法读取所选索引工作表")?;
        header(&range, Some(config.header_row))?;
        let start_row = range.start().map(|(r, _)| r).unwrap_or(0);
        let start_col = range.start().map(|(_, c)| c as usize).unwrap_or(0);
        let width = range.width();
        if config.index_column < start_col
            || config.name_column < start_col
            || config.index_column >= start_col + width
            || config.name_column >= start_col + width
        {
            bail!("所选索引列不在工作表范围内");
        }
        let mut entries = vec![];
        for (offset, row) in range
            .rows()
            .enumerate()
            .skip((config.header_row - start_row) as usize)
        {
            let value = cell(row.get(config.index_column - start_col));
            if value.is_empty() {
                continue;
            }
            entries.push(Entry {
                key: value,
                value: cell(row.get(config.name_column - start_col)),
                row: start_row + offset as u32 + 1,
            });
        }
        use sha2::Digest;
        Ok(Self {
            snapshot: Snapshot {
                config,
                sha256: hex::encode(sha2::Sha256::digest(&bytes)),
                rows: entries.len(),
            },
            entries,
        })
    }
    pub fn resolve(&self, folder: &str, relative: &str) -> Evidence {
        let config = &self.snapshot.config;
        let mut rule = "exact";
        let mut matches = vec![];
        if config.match_rule != "voucher_code" {
            for query in [folder, relative] {
                matches = self
                    .entries
                    .iter()
                    .filter(|e| key(&e.key) == key(query))
                    .collect();
                if !matches.is_empty() {
                    break;
                }
            }
        }
        if matches.is_empty() && config.match_rule != "exact" {
            let code = crate::ledger::normalize_code(folder).or_else(|| {
                folder
                    .split('-')
                    .nth(1)
                    .and_then(crate::ledger::normalize_code)
            });
            if let Some(code) = code {
                matches = self
                    .entries
                    .iter()
                    .filter(|e| {
                        crate::ledger::normalize_code(&e.key).as_deref() == Some(code.as_str())
                    })
                    .collect();
                rule = "voucher_code";
            }
        }
        if matches.is_empty() {
            return Evidence {
                status: "unmatched".into(),
                rule: None,
                key: Some(folder.into()),
                rows: vec![],
                value: None,
                message: "未匹配索引，已回退为文件夹名称".into(),
            };
        }
        let rows = matches.iter().map(|e| e.row).collect();
        let values: BTreeSet<_> = matches.iter().map(|e| e.value.as_str()).collect();
        let first = matches[0];
        if config.duplicates == "fallback" && values.len() > 1 {
            return Evidence {
                status: "conflict".into(),
                rule: Some(rule.into()),
                key: Some(first.key.clone()),
                rows,
                value: None,
                message: "同一索引对应不同名称，已回退为文件夹名称；可改用首条策略或手动命名"
                    .into(),
            };
        }
        if first.value.is_empty() {
            return Evidence {
                status: "empty".into(),
                rule: Some(rule.into()),
                key: Some(first.key.clone()),
                rows,
                value: None,
                message: "索引名称为空，已回退为文件夹名称".into(),
            };
        }
        Evidence {
            status: "matched".into(),
            rule: Some(rule.into()),
            key: Some(first.key.clone()),
            rows,
            value: Some(first.value.clone()),
            message: if matches.len() > 1 {
                "匹配多行，使用所选重复项策略".into()
            } else {
                "已按索引重命名".into()
            },
        }
    }
}
pub fn verify(snapshot: &Snapshot) -> Result<()> {
    let path = crate::generic::absolute_checked(Path::new(&snapshot.config.path), true)?;
    if crate::pdf::sha256_file(&path)? != snapshot.sha256 {
        bail!("重命名索引已变化，请重新扫描以核对输出名称");
    }
    Ok(())
}
