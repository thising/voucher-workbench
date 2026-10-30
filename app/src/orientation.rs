//! Title text is the sole orientation evidence. Dimensions only select page layout.
use crate::{
    model::{Orientation, TitleCandidate},
    ocr::OcrEngine,
};
use anyhow::{anyhow, bail, Context, Result};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const MODEL_ID: &str = "title-ocr-v1.0.2";
const TITLE_MIN_CONFIDENCE: f32 = 0.82;
pub fn is_current(value: &Orientation) -> bool {
    value.model == MODEL_ID
}
/// Decode once, applying every EXIF transform (including mirrored variants).
/// Callers must apply `rotation_cw` only after this function, never before it.
pub fn load_image(path: &Path) -> Result<DynamicImage> {
    let reader = ImageReader::open(path)
        .with_context(|| format!("无法打开图片：{}", path.display()))?
        .with_guessed_format()
        .context("无法识别图片格式")?;
    let mut decoder = reader.into_decoder().context("图片解码器初始化失败")?;
    let orientation = decoder.orientation().context("无法读取图片方向元数据")?;
    // Bound allocation even when a crafted header advertises enormous dimensions.
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 100_000_000 {
        bail!("图片尺寸无效或超过 1 亿像素的单页上限");
    }
    let mut image = DynamicImage::from_decoder(decoder).context("图片损坏或无法解码")?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub struct Engine {
    ocr: OcrEngine,
}
impl Engine {
    pub fn new(model_path: &Path, runtime_path: Option<&Path>) -> Result<Self> {
        Ok(Self {
            ocr: OcrEngine::new(model_path, runtime_path)?,
        })
    }
    pub fn predict(&mut self, path: &Path) -> Result<Orientation> {
        self.predict_image(&load_image(path)?)
    }
    pub fn predict_image(&mut self, image: &DynamicImage) -> Result<Orientation> {
        let rgb = composite_white(image);
        let mut candidates = vec![];
        for angle in [0, 90, 180, 270] {
            let candidate = match angle {
                0 => rgb.clone(),
                90 => image::imageops::rotate90(&rgb),
                180 => image::imageops::rotate180(&rgb),
                _ => image::imageops::rotate270(&rgb),
            };
            let band_height = ((candidate.height() as f32 * 0.42).ceil() as u32)
                .max(96)
                .min(candidate.height());
            let header =
                image::imageops::crop_imm(&candidate, 0, 0, candidate.width(), band_height)
                    .to_image();
            let lines = self.ocr.read_lines(&header)?;
            for line in lines {
                if let Some(keyword) = title_keyword(&line.text) {
                    let [x, y, w, h] = line.bbox;
                    let center = x as f32 + w as f32 / 2.;
                    let positioned = center >= candidate.width() as f32 * 0.06
                        && center <= candidate.width() as f32 * 0.94;
                    let readable = line.confidence.is_finite()
                        && line.confidence >= TITLE_MIN_CONFIDENCE
                        && h >= 7
                        && w as f32 >= h as f32 * 1.5;
                    // y is relative to the rotated header, also relative to the full rotated image.
                    candidates.push(TitleCandidate {
                        rotation_cw: angle,
                        text: line.text,
                        confidence: line.confidence,
                        bbox: [x, y, w, h],
                        keyword: keyword.into(),
                        accepted: positioned && readable,
                    });
                }
            }
        }
        Ok(select_title_direction(candidates))
    }
}

// These are document-title terms, never voucher ids or source-specific paths.
// Body fields such as invoice number/code must not masquerade as a title.
fn title_keyword(text: &str) -> Option<&'static str> {
    // Parenthesized form variants, e.g. 转账回单（多人）, qualify a heading.
    let text = text.trim();
    let heading = if text.ends_with(['）', ')']) {
        text.rsplit_once(['（', '('])
            .map(|(head, _)| head)
            .unwrap_or(text)
    } else {
        text
    };
    let compact: String = heading
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .collect();
    for field in [
        "发票代码",
        "发票号码",
        "发票编号",
        "发票日期",
        "发票金额",
        "收据号码",
        "通知单号",
        "合同编号",
        "合同金额",
        "原始凭证张",
        "凭证编号",
        "凭证号码",
        // These are invoice side annotations, often perpendicular to the real
        // heading. Their text must never turn a sideways invoice another 90°.
        "购买方",
        "买方",
        "购方",
        "方记账凭证",
        "方记帐凭证",
        "销售方",
        "销方",
        "发票联",
        "抵扣联",
        "记账联",
        "本联",
        "此联",
        "请附",
        "业务类型",
    ] {
        if compact.contains(field) {
            return None;
        }
    }
    for title in [
        "记账凭证",
        "记帐凭证",
        "到款通知单",
        "到账通知单",
        "收款通知单",
        "付款通知单",
        "入账通知单",
        "银行回单",
        "电子回单",
        "收款收据",
        "资金往来结算票据",
        "财政票据",
        "转账凭证",
        "汇款凭证",
        "支付凭证",
        "付款凭证",
        "收款凭证",
        "收支凭证",
        "记账凭单",
        "报销凭证",
        "支出凭单",
        "差旅费报销",
        "费用报销",
        "报销单",
        "审批单",
        "审批表",
        "结算单",
        "缴款书",
        "缴费通知",
        "付款申请",
        "支付申请",
        "采购申请",
        "验收单",
        "送货单",
        "购销合同",
        "技术服务合同",
        "测试委托单",
        "交易明细",
        "银行流水",
        "购货清单",
        "费用清单",
        "结算清单",
        "付款回执",
        "支付回执",
        "行程单",
        "出库单",
        "入库单",
        "电子客票",
        "车票",
        "登机牌",
        "订票信息",
        "订单信息",
        "支付业务委托书",
        "转账回单",
        "服务清单",
        "销售清单",
        "签收表",
    ] {
        if compact.ends_with(title) {
            return Some(title);
        }
    }
    [
        "发票",
        "收据",
        "协议书",
        "合同",
        "申请表",
        "申请单",
        "登记表",
        "明细表",
    ]
    .into_iter()
    .find(|title| compact.ends_with(title) && compact.chars().count() >= 4)
}

/// A unique, clearly read title direction is accepted. A disagreement is not
/// resolved by the old page classifier, dimensions, adjacent pages, or a score vote.
pub fn select_title_direction(mut candidates: Vec<TitleCandidate>) -> Orientation {
    // Reapply title semantics to saved OCR evidence too. This allows the same
    // decision function to re-evaluate candidates after a stricter title rule.
    candidates.retain_mut(|candidate| {
        if let Some(keyword) = title_keyword(&candidate.text) {
            candidate.keyword = keyword.into();
            true
        } else {
            false
        }
    });
    candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let directions: BTreeSet<_> = candidates
        .iter()
        .filter(|c| c.accepted)
        .map(|c| c.rotation_cw)
        .collect();
    let best = candidates
        .iter()
        .find(|c| c.accepted)
        .or_else(|| candidates.first());
    let unique = directions.len() == 1;
    let reason = if unique {
        "标题文字清楚，阅读方向唯一"
    } else if directions.len() > 1 {
        "多个方向识别到标题，需检查标题方向冲突"
    } else if candidates.is_empty() {
        "未找到可识别的文档标题"
    } else {
        "标题文字不够清楚或位置不可靠"
    };
    Orientation {
        rotation_cw: if unique {
            *directions.first().unwrap()
        } else {
            0
        },
        confidence: best.map(|c| c.confidence).unwrap_or(0.),
        scores: vec![],
        model: MODEL_ID.into(),
        needs_review: !unique,
        title: best.map(|c| c.text.clone()),
        reason: reason.into(),
        candidates,
    }
}
/// ort owns one process-global runtime. Resolve an explicit file before calling
/// it, so a missing runtime becomes an actionable UI error instead of a panic.
pub(crate) fn initialize_runtime(explicit: Option<&Path>) -> Result<()> {
    static RUNTIME: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    let path = resolve_runtime(explicit)?;
    let loaded = RUNTIME.get_or_init(|| {
        std::panic::catch_unwind(|| {
            ort::init_from(path.to_string_lossy())
                .with_name("voucher-workbench")
                .with_telemetry(false)
                .commit()
                .map(|_| path.clone())
                .map_err(|err| format!("ONNX Runtime 初始化失败：{err}"))
        })
        .unwrap_or_else(|_| Err("ONNX Runtime 加载失败，请检查版本、CPU 架构及原生依赖".into()))
    });
    match loaded {
        Ok(previous) if previous == &path => Ok(()),
        Ok(_) => bail!("本进程已加载另一份 ONNX Runtime，请重启服务后切换运行库"),
        Err(message) => bail!("{message}"),
    }
}

fn resolve_runtime(explicit: Option<&Path>) -> Result<PathBuf> {
    let library = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    let mut candidates = vec![];
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    } else if let Some(path) = std::env::var_os("ORT_DYLIB_PATH") {
        candidates.push(PathBuf::from(path));
    } else if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.extend([parent.join(library), parent.join("runtime").join(library)]);
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow!(
                "未找到 ONNX Runtime 原生运行库；用 --runtime 指定 {} 的绝对路径",
                library
            )
        })?
        .canonicalize()
        .context("无法访问 ONNX Runtime 运行库")
}

pub(crate) fn composite_white(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let alpha = u32::from(p[3]);
        image::Rgb(
            [0, 1, 2].map(|c| ((u32::from(p[c]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn candidate(angle: u16, accepted: bool) -> TitleCandidate {
        TitleCandidate {
            rotation_cw: angle,
            text: "合成记账凭证".into(),
            confidence: 0.94,
            bbox: [10, 20, 220, 30],
            keyword: "记账凭证".into(),
            accepted,
        }
    }
    #[test]
    fn unique_title_is_the_only_automatic_evidence() {
        for angle in [0, 90, 180, 270] {
            let result = select_title_direction(vec![candidate(angle, true)]);
            assert_eq!(result.rotation_cw, angle);
            assert!(!result.needs_review);
        }
        assert!(select_title_direction(vec![]).needs_review);
        assert!(select_title_direction(vec![candidate(0, false)]).needs_review);
        assert!(
            select_title_direction(vec![candidate(0, true), candidate(180, true)]).needs_review
        );
    }
    #[test]
    fn body_fields_do_not_become_titles() {
        assert_eq!(title_keyword("华中某大学记账凭证"), Some("记账凭证"));
        assert_eq!(title_keyword("增值税普通发票"), Some("发票"));
        assert_eq!(title_keyword("电子发票（普通发票）"), Some("发票"));
        assert_eq!(
            title_keyword("某大学网银对私转账回单（多人）"),
            Some("转账回单")
        );
        assert_eq!(
            title_keyword("某大学电子支付业务委托书"),
            Some("支付业务委托书")
        );
        for body in [
            "发票代码1234",
            "发票号码1234",
            "合同编号1234",
            "项目名称",
            "123456789",
            "购买方记账凭证",
            "买方记账凭证",
            "方记账凭证",
            "方记帐凭证",
            "销售方记账凭证",
            "请附费用报销凭证",
            "业务类型银行回单",
            "记账凭证打印说明",
        ] {
            assert_eq!(title_keyword(body), None);
        }
        let mut side_note = candidate(180, true);
        side_note.text = "方记账凭证".into();
        let reclassified = select_title_direction(vec![side_note]);
        assert!(reclassified.needs_review);
        assert_eq!(reclassified.rotation_cw, 0);
        assert!(reclassified.candidates.is_empty());
    }
    #[test]
    fn transparent_pixels_are_white() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            3,
            7,
            image::Rgba([0, 100, 200, 0]),
        ));
        assert!(composite_white(&image)
            .pixels()
            .all(|p| p.0 == [255, 255, 255]));
    }
    #[test]
    fn image_decode_applies_exif_rotation() {
        use image::{codecs::jpeg::JpegEncoder, ExtendedColorType, ImageEncoder};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orientation.jpg");
        // EXIF TIFF little-endian, one SHORT orientation field, value=6 (90 CW).
        let exif = b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00";
        let file = fs::File::create(&path).unwrap();
        let mut encoder = JpegEncoder::new(file);
        encoder.set_exif_metadata(exif.to_vec()).unwrap();
        encoder
            .write_image(&[120; 3 * 2 * 3], 3, 2, ExtendedColorType::Rgb8)
            .unwrap();
        let image = load_image(&path).unwrap();
        assert_eq!((image.width(), image.height()), (2, 3));
    }
}
