//! Read only the two fields that define voucher names. Row numbers remain the
//! original, one-based spreadsheet coordinates, including pre-header rows.
use crate::model::{Issue, SheetInfo};
use anyhow::{bail, Context, Result};
use calamine::{open_workbook_auto_from_rs, Data, Range, Reader};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Cursor, path::Path};

#[derive(Clone, Debug)]
pub struct LedgerEntry {
    pub subject: Option<String>,
    pub row: u32,
    /// Later occurrences only; the first occurrence is always `row`.
    pub duplicate_rows: Vec<u32>,
}

#[derive(Debug)]
pub struct Ledger {
    pub sha256: String,
    pub sheet: String,
    pub sheets: Vec<SheetInfo>,
    pub entries: BTreeMap<String, LedgerEntry>,
    pub issues: Vec<Issue>,
}

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{feff}')
        .collect()
}

/// Normalization deliberately does not parse a code as an integer: leading
/// zeroes are meaningful. We accept only a bare number or the documented (c)
/// prefix, so unrelated numeric-looking text is never accidentally matched.
pub fn normalize_code(value: &str) -> Option<String> {
    let value = compact(value).replace('（', "(").replace('）', ")");
    let digits = if value
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("(c)"))
    {
        &value[3..]
    } else {
        value.as_str()
    };
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.to_owned())
}

fn text(cell: Option<&Data>) -> String {
    match cell {
        None | Some(Data::Empty) => String::new(),
        Some(Data::String(value)) => value.trim().trim_matches('\u{feff}').to_owned(),
        Some(Data::Error(_)) => String::new(),
        Some(value) => value.to_string().trim().to_owned(),
    }
}

fn parse_sheet(range: &Range<Data>) -> Result<(u32, BTreeMap<String, LedgerEntry>, Vec<Issue>)> {
    let mut header = None;
    for (offset, row) in range.rows().enumerate() {
        let codes: Vec<_> = row
            .iter()
            .enumerate()
            .filter(|(_, c)| compact(&c.to_string()) == "凭证编号")
            .map(|(i, _)| i)
            .collect();
        let subjects: Vec<_> = row
            .iter()
            .enumerate()
            .filter(|(_, c)| compact(&c.to_string()) == "科目名称")
            .map(|(i, _)| i)
            .collect();
        if !codes.is_empty() && !subjects.is_empty() {
            if codes.len() != 1 || subjects.len() != 1 {
                bail!("表头包含重复的“凭证编号”或“科目名称”列，无法确定匹配列");
            }
            header = Some((offset, codes[0], subjects[0]));
            break;
        }
    }
    let (offset, code_col, subject_col) =
        header.context("缺少同一行的“凭证编号”和“科目名称”表头")?;
    let start_row = range.start().map(|(r, _)| r).unwrap_or(0);
    let header_row = start_row + offset as u32 + 1;
    let mut entries: BTreeMap<String, LedgerEntry> = BTreeMap::new();
    let mut issues = Vec::new();
    for (index, row) in range.rows().enumerate().skip(offset + 1) {
        let source_row = start_row + index as u32 + 1;
        let raw_code = text(row.get(code_col));
        if raw_code.is_empty() || compact(&raw_code) == "凭证编号" {
            continue;
        }
        // Totals and footer notes are not voucher rows. No other column is read.
        let Some(code) = normalize_code(&raw_code) else {
            continue;
        };
        if let Some(existing) = entries.get_mut(&code) {
            existing.duplicate_rows.push(source_row);
            continue;
        }
        let subject = text(row.get(subject_col));
        if subject.is_empty() {
            issues.push(Issue {
                severity: "warning".into(),
                message: format!("工作表第 {source_row} 行的首匹配科目名称为空；对应凭证不能生成"),
            });
        }
        entries.insert(
            code,
            LedgerEntry {
                subject: (!subject.is_empty()).then_some(subject),
                row: source_row,
                duplicate_rows: Vec::new(),
            },
        );
    }
    Ok((header_row, entries, issues))
}

pub fn read(path: &Path, requested_sheet: Option<&str>) -> Result<Ledger> {
    // Parse precisely the bytes that we hash, avoiding an inconsistent ledger
    // fingerprint if the user saves the workbook during scanning.
    let bytes = fs::read(path).with_context(|| format!("无法读取明细账 {}", path.display()))?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes))
        .context("无法解析明细账（支持 XLS / XLSX）")?;
    let names = workbook.sheet_names().to_vec();
    let selected = requested_sheet
        .map(str::to_owned)
        .or_else(|| names.first().cloned())
        .context("明细账中没有工作表")?;
    if !names.contains(&selected) {
        bail!("明细账中不存在工作表“{selected}”；请重新选择");
    }
    let mut sheets = Vec::with_capacity(names.len());
    let mut selected_entries = None;
    let mut selected_issues = Vec::new();
    let mut selected_error = None;
    for name in names {
        let result = workbook
            .worksheet_range(&name)
            .map_err(anyhow::Error::from)
            .and_then(|range| parse_sheet(&range));
        match result {
            Ok((header_row, entries, issues)) => {
                sheets.push(SheetInfo {
                    name: name.clone(),
                    valid: true,
                    error: None,
                    header_row: Some(header_row),
                    unique_codes: entries.len(),
                });
                if name == selected {
                    selected_entries = Some(entries);
                    selected_issues = issues;
                }
            }
            Err(error) => {
                let message = format!("{error:#}");
                sheets.push(SheetInfo {
                    name: name.clone(),
                    valid: false,
                    error: Some(message.clone()),
                    header_row: None,
                    unique_codes: 0,
                });
                if name == selected {
                    selected_error = Some(message);
                }
            }
        }
    }
    let entries = selected_entries.with_context(|| {
        format!(
            "工作表“{selected}”不可用于匹配：{}",
            selected_error.unwrap_or_default()
        )
    })?;
    Ok(Ledger {
        sha256,
        sheet: selected,
        sheets,
        entries,
        issues: selected_issues,
    })
}

/// Lists every worksheet even when the first sheet has no usable header. This
/// is deliberately separate from `read`, whose selected sheet must be valid.
pub fn inspect(path: &Path) -> Result<Vec<SheetInfo>> {
    let bytes = fs::read(path).with_context(|| format!("无法读取明细账 {}", path.display()))?;
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes))
        .context("无法解析明细账（支持 XLS / XLSX）")?;
    let names = workbook.sheet_names().to_vec();
    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let result = workbook
            .worksheet_range(&name)
            .map_err(anyhow::Error::from)
            .and_then(|range| parse_sheet(&range));
        sheets.push(match result {
            Ok((header_row, entries, _)) => SheetInfo {
                name,
                valid: true,
                error: None,
                header_row: Some(header_row),
                unique_codes: entries.len(),
            },
            Err(error) => SheetInfo {
                name,
                valid: false,
                error: Some(format!("{error:#}")),
                header_row: None,
                unique_codes: 0,
            },
        });
    }
    Ok(sheets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_normalization_keeps_zeroes_and_rejects_unrelated_numbers() {
        assert_eq!(normalize_code(" （ C ） 04067 "), Some("04067".into()));
        assert_eq!(normalize_code("04067"), Some("04067".into()));
        for value in [
            "",
            "c04067",
            "40.67",
            "(c)04067注",
            "二〇二三",
            "摘要",
            "(中)",
        ] {
            assert_eq!(normalize_code(value), None, "{value}");
        }
    }

    fn fixture() -> Range<Data> {
        let mut range = Range::new((5, 2), (9, 4));
        range.set_value((6, 2), Data::String("凭证编号".into()));
        range.set_value((6, 4), Data::String("科目名称".into()));
        range.set_value((7, 2), Data::String("(c)00001".into()));
        range.set_value((7, 4), Data::String("首个科目".into()));
        range.set_value((8, 2), Data::String("(c)00001".into()));
        range.set_value((8, 4), Data::String("后续科目".into()));
        range.set_value((9, 2), Data::String("00002".into()));
        range
    }

    #[test]
    fn header_offset_and_first_duplicate_row_are_preserved() {
        let (header_row, entries, issues) = parse_sheet(&fixture()).unwrap();
        assert_eq!(header_row, 7);
        assert_eq!(entries["00001"].row, 8);
        assert_eq!(entries["00001"].subject.as_deref(), Some("首个科目"));
        assert_eq!(entries["00001"].duplicate_rows, vec![9]);
        assert_eq!(entries["00002"].subject, None);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn blank_first_subject_does_not_fall_back() {
        let mut range = fixture();
        range.set_value((7, 4), Data::Empty);
        let (_, entries, _) = parse_sheet(&range).unwrap();
        assert_eq!(entries["00001"].subject, None);
        assert_eq!(entries["00001"].row, 8);
    }

    #[test]
    fn ambiguous_or_missing_headers_are_rejected() {
        let mut range = fixture();
        range.set_value((6, 3), Data::String("凭证编号".into()));
        assert!(parse_sheet(&range).is_err());
        assert!(parse_sheet(&Range::new((0, 0), (0, 0))).is_err());
    }

    #[test]
    fn invalid_first_sheet_can_be_inspected_and_explicit_second_selected() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.xlsx");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let files = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="说明" sheetId="1" r:id="rId1"/><sheet name="人工指定表" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>仅说明，不能匹配</t></is></c></row></sheetData></worksheet>"#,
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="3"><c r="A3" t="inlineStr"><is><t>凭证编号</t></is></c><c r="B3" t="inlineStr"><is><t>科目名称</t></is></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>(c)00001</t></is></c><c r="B4" t="inlineStr"><is><t>合成科目</t></is></c></row></sheetData></worksheet>"#,
            ),
        ];
        for (name, content) in files {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let sheets = inspect(&path).unwrap();
        assert_eq!(sheets.len(), 2);
        assert_eq!(sheets[0].name, "说明");
        assert!(!sheets[0].valid);
        assert!(sheets[1].valid);
        assert_eq!(sheets[1].header_row, Some(3));
        assert!(read(&path, None).is_err());
        let ledger = read(&path, Some("人工指定表")).unwrap();
        assert_eq!(ledger.entries["00001"].row, 4);
        assert_eq!(ledger.entries["00001"].subject.as_deref(), Some("合成科目"));
        assert!(read(&path, Some("不存在的表")).is_err());
    }
}
