use serde::{Deserialize, Serialize};

pub const ORIENTATION_DISCLAIMER_VERSION: &str = "orientation-disclaimer-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrientationWorkflow {
    pub enabled: bool,
    pub consent_version: Option<String>,
    pub consented_at: Option<String>,
    pub analysis_completed: bool,
}
impl Default for OrientationWorkflow {
    fn default() -> Self {
        // A legacy plan may contain automatic rotations. It must not silently
        // become an opted-out plan or bypass the new consent/review stage.
        Self {
            enabled: true,
            consent_version: None,
            consented_at: None,
            analysis_completed: false,
        }
    }
}
impl OrientationWorkflow {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
    pub fn has_current_consent(&self) -> bool {
        self.consent_version.as_deref() == Some(ORIENTATION_DISCLAIMER_VERSION)
            && self.consented_at.as_ref().is_some_and(|at| !at.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SheetInfo {
    pub name: String,
    pub valid: bool,
    pub error: Option<String>,
    pub header_row: Option<u32>,
    pub unique_codes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub severity: String, // error | warning
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Orientation {
    pub rotation_cw: u16, // correction, clockwise, after EXIF normalization
    pub confidence: f32,
    pub scores: Vec<f32>,
    pub model: String,
    #[serde(default = "default_review")]
    pub needs_review: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub candidates: Vec<TitleCandidate>,
}

fn default_review() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TitleCandidate {
    pub rotation_cw: u16,
    pub text: String,
    pub confidence: f32,
    pub bbox: [u32; 4], // x,y,width,height in the candidate-rotated normalized image
    pub keyword: String,
    pub accepted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: String,
    pub name: String,
    pub relative_path: String, // relative to plan root
    pub sequence: u32,
    pub kind: String, // image | pdf
    pub pages: u32,
    pub bytes: u64,
    pub modified_ms: u64,
    #[serde(default)]
    pub sha256: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<Orientation>,
    pub rotation_cw: u16, // effective image correction or extra PDF rotation
    pub reviewed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Voucher {
    pub id: String,
    pub item: String,
    pub folder: String,
    pub relative_path: String,
    pub code: String,
    pub subject: Option<String>,
    pub ledger_row: Option<u32>,
    pub duplicate_rows: Vec<u32>,
    pub output_relative_path: Option<String>,
    pub files: Vec<SourceFile>,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub root: String,
    #[serde(default)]
    pub requested_root: Option<String>,
    pub ledger_path: String,
    pub ledger_sha256: String,
    pub sheet: String,
    pub sheets: Vec<SheetInfo>,
    pub created_at: String,
    pub vouchers: Vec<Voucher>,
    pub issues: Vec<Issue>,
    pub ignored: usize,
    #[serde(default)]
    pub orientation_workflow: OrientationWorkflow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanRequest {
    pub root: String, // batch root containing item directories OR one item directory
    pub ledger: Option<String>, // autodetect first XLS/XLSX next to root or in root
    pub sheet: Option<String>, // None means first workbook sheet
}
