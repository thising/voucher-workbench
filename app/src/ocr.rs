//! Local text detection and recognition only. There is deliberately no angle
//! classifier, crop auto-rotation, or document-orientation model in this module.
use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use image::{imageops, RgbImage};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use sha2::{Digest, Sha256};

pub const DET_SHA256: &str = "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d";
pub const REC_SHA256: &str = "da72dc72ca4dc220df0dfde68c1dedc31c58d3e76a25871122e5056227d50092";
pub const DICTIONARY_SHA256: &str =
    "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b";
const MAX_LINES: usize = 12;
const MAX_JOINED_LINES: usize = 4;

#[derive(Clone, Debug)]
pub struct TextLine {
    pub text: String,
    pub confidence: f32,
    pub bbox: [u32; 4],
}

pub struct OcrEngine {
    detector: Session,
    recognizer: Session,
    dictionary: Vec<String>,
}

fn verified_bytes(path: &Path, expected: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "标题OCR模型文件不存在或不可读：{}；请先运行fetch-title-models.sh",
            path.display()
        )
    })?;
    if hex::encode(Sha256::digest(&bytes)) != expected {
        bail!("标题OCR模型文件SHA256校验失败：{}", path.display());
    }
    Ok(bytes)
}

fn session(bytes: &[u8]) -> Result<Session> {
    Ok(Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .with_log_level(ort::logging::LogLevel::Error)?
        .commit_from_memory(bytes)?)
}

impl OcrEngine {
    pub fn new(model_dir: &Path, runtime: Option<&Path>) -> Result<Self> {
        let det = verified_bytes(&model_dir.join("det.onnx"), DET_SHA256)?;
        let rec = verified_bytes(&model_dir.join("rec.onnx"), REC_SHA256)?;
        let dict = verified_bytes(&model_dir.join("dictionary.txt"), DICTIONARY_SHA256)?;
        let mut dictionary = vec![String::new()]; // CTC blank occupies class zero.
        dictionary.extend(std::str::from_utf8(&dict)?.lines().map(str::to_owned));
        dictionary.push(" ".into()); // Official CTCLabelDecode adds a literal space.
        if dictionary.len() != 18385 {
            bail!("标题OCR字典长度与模型不一致");
        }
        crate::orientation::initialize_runtime(runtime)?;
        Ok(Self {
            detector: session(&det)?,
            recognizer: session(&rec)?,
            dictionary,
        })
    }

    /// Returns twelve prominent horizontal lines and up to four adjacent-fragment
    /// candidates from a title region, preserving the original line evidence.
    /// Input/crop coordinates are preserved. Vertical lines are not turned into
    /// horizontal crops, since doing so would erase the evidence of page rotation.
    pub fn read_lines(&mut self, image: &RgbImage) -> Result<Vec<TextLine>> {
        let (w, h) = image.dimensions();
        if w < 8 || h < 8 {
            return Ok(Vec::new());
        }
        let (input, dw, dh) = detector_input(image);
        let input = Tensor::from_array(([1_usize, 3, dh as usize, dw as usize], input))?;
        let output = self
            .detector
            .run(ort::inputs![input])
            .context("标题文字检测失败")?;
        let (shape, probabilities) = output[0].try_extract_tensor::<f32>()?;
        if shape.len() != 4 || shape[0] != 1 || shape[1] != 1 {
            bail!("文字检测模型输出结构不符");
        }
        let (mh, mw) = (shape[2] as usize, shape[3] as usize);
        if mh.checked_mul(mw) != Some(probabilities.len()) {
            bail!("文字检测概率图尺寸不符");
        }
        let mut boxes = horizontal_boxes(probabilities, mw, mh, w, h);
        drop(output);
        // A large title may be split by an overprinted stamp. Join before the
        // twelve-line budget: its short final fragment may otherwise be omitted.
        // Each candidate joins only two immediate, similarly sized fragments;
        // no recursive joining or crop rotation is performed.
        let joined = adjacent_line_pairs(&boxes);
        // Headers tend to be larger than table cells. Size ranking is only used
        // to bound OCR work; final title acceptance belongs to the caller.
        boxes.sort_by(|a, b| {
            prominence(b)
                .total_cmp(&prominence(a))
                .then_with(|| a[1].cmp(&b[1]))
        });
        boxes.truncate(MAX_LINES);
        boxes.extend(
            joined
                .into_iter()
                .filter(|joined| !boxes.contains(joined))
                .collect::<Vec<_>>(),
        );
        let mut lines = Vec::with_capacity(boxes.len());
        for bbox in boxes {
            let crop = imageops::crop_imm(image, bbox[0], bbox[1], bbox[2], bbox[3]).to_image();
            let (text, confidence) = self.recognize(&crop)?;
            if !text.trim().is_empty() {
                lines.push(TextLine {
                    text,
                    confidence,
                    bbox,
                });
            }
        }
        lines.sort_by_key(|line| (line.bbox[1], line.bbox[0]));
        Ok(lines)
    }

    fn recognize(&mut self, crop: &RgbImage) -> Result<(String, f32)> {
        let (data, width) = recognizer_input(crop);
        let input = Tensor::from_array(([1_usize, 3, 48, width], data))?;
        let output = self
            .recognizer
            .run(ort::inputs![input])
            .context("标题文字识别失败")?;
        let (shape, probabilities) = output[0].try_extract_tensor::<f32>()?;
        if shape.len() != 3 || shape[0] != 1 || shape[2] as usize != self.dictionary.len() {
            bail!("文字识别模型输出与字典不一致");
        }
        decode_ctc(probabilities, shape[1] as usize, &self.dictionary)
    }
}

fn prominence(r: &[u32; 4]) -> f32 {
    r[3] as f32 * (r[2] as f32 / r[3].max(1) as f32).min(12.).sqrt()
}

fn adjacent_line_pairs(boxes: &[[u32; 4]]) -> Vec<[u32; 4]> {
    let mut joined = Vec::new();
    for left in boxes {
        let right = boxes
            .iter()
            .filter(|right| {
                if right[0] <= left[0] {
                    return false;
                }
                let small_h = left[3].min(right[3]) as f32;
                let large_h = left[3].max(right[3]) as f32;
                let gap = right[0] as i64 - (left[0] + left[2]) as i64;
                let left_bottom = left[1] + left[3];
                let right_bottom = right[1] + right[3];
                let overlap = left_bottom
                    .min(right_bottom)
                    .saturating_sub(left[1].max(right[1]));
                small_h >= 10.
                    && large_h <= small_h * 1.65
                    && gap as f32 >= -small_h * 0.4
                    && gap as f32 <= small_h * 0.6
                    && overlap as f32 >= small_h * 0.65
                    && left_bottom.abs_diff(right_bottom) as f32 <= large_h * 0.3
            })
            .min_by_key(|right| right[0]);
        if let Some(right) = right {
            let x = left[0];
            let y = left[1].min(right[1]);
            let w = (right[0] + right[2]).max(left[0] + left[2]) - x;
            let h = (left[1] + left[3]).max(right[1] + right[3]) - y;
            if w as f32 <= h as f32 * 18. {
                joined.push([x, y, w, h]);
            }
        }
    }
    joined.sort_by(|a, b| {
        prominence(b)
            .total_cmp(&prominence(a))
            .then_with(|| a[1].cmp(&b[1]))
    });
    joined.dedup();
    joined.truncate(MAX_JOINED_LINES);
    joined
}

fn detector_input(image: &RgbImage) -> (Vec<f32>, u32, u32) {
    let (w, h) = image.dimensions();
    let scale = (960.0 / w.max(h) as f32).min(1.0);
    let dw = (((w as f32 * scale / 32.).round() as u32) * 32).max(32);
    let dh = (((h as f32 * scale / 32.).round() as u32) * 32).max(32);
    let resized = imageops::resize(image, dw, dh, imageops::FilterType::Triangle);
    let plane = dw as usize * dh as usize;
    let mean = [0.485_f32, 0.456, 0.406];
    let std = [0.229_f32, 0.224, 0.225];
    let mut data = vec![0.; 3 * plane];
    for (i, pixel) in resized.pixels().enumerate() {
        for c in 0..3 {
            data[c * plane + i] = (pixel[2 - c] as f32 / 255. - mean[c]) / std[c];
        }
    }
    (data, dw, dh)
}

fn recognizer_input(crop: &RgbImage) -> (Vec<f32>, usize) {
    let resized_w = (48.0 * crop.width() as f64 / crop.height().max(1) as f64)
        .ceil()
        .clamp(1., 3200.) as u32;
    let width = resized_w.max(320) as usize;
    let resized = imageops::resize(crop, resized_w, 48, imageops::FilterType::Triangle);
    // Official normalization pads with zeros AFTER normalization, not white.
    let mut data = vec![0.; 3 * 48 * width];
    for y in 0..48 {
        for x in 0..resized_w {
            let pixel = resized.get_pixel(x, y);
            for c in 0..3 {
                data[c * 48 * width + y as usize * width + x as usize] =
                    pixel[2 - c] as f32 / 127.5 - 1.;
            }
        }
    }
    (data, width)
}

fn decode_ctc(probabilities: &[f32], steps: usize, dictionary: &[String]) -> Result<(String, f32)> {
    let classes = dictionary.len();
    if steps.checked_mul(classes) != Some(probabilities.len()) {
        bail!("CTC概率尺寸不符");
    }
    let mut text = String::new();
    let mut previous = usize::MAX;
    let mut confidence = 0.;
    let mut count = 0;
    for row in probabilities.chunks_exact(classes) {
        let (index, value) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .context("CTC无类别")?;
        if !value.is_finite() || *value < 0. || *value > 1.0001 {
            bail!("CTC返回非概率数值");
        }
        if index != 0 && index != previous {
            text.push_str(&dictionary[index]);
            confidence += value;
            count += 1;
        }
        previous = index;
    }
    Ok((
        text,
        if count == 0 {
            0.
        } else {
            confidence / count as f32
        },
    ))
}

fn horizontal_boxes(
    prob: &[f32],
    width: usize,
    height: usize,
    original_w: u32,
    original_h: u32,
) -> Vec<[u32; 4]> {
    let mut seen = vec![false; prob.len()];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..prob.len() {
        if seen[start] || prob[start] < 0.3 {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) =
            (start % width, start / width, start % width, start / width);
        let mut count = 0usize;
        let mut score = 0_f32;
        while let Some(index) = stack.pop() {
            let x = index % width;
            let y = index / width;
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            count += 1;
            score += prob[index];
            for yy in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let n = yy * width + xx;
                    if !seen[n] && prob[n] >= 0.3 {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        let bw = (x1 - x0 + 1) as f32;
        let bh = (y1 - y0 + 1) as f32;
        if count < 12 || score / (count as f32) < 0.6 || bh < 3. || bw < bh * 1.6 || bw < 12. {
            continue;
        }
        // Axis-aligned equivalent of DB unclip: A*r/perimeter, preserving the
        // rectangle's orientation rather than rotating it to a reading direction.
        let pad = (count as f32 * 1.5 / (2. * (bw + bh))).max(1.);
        let scale_x = original_w as f32 / width as f32;
        let scale_y = original_h as f32 / height as f32;
        let left = ((x0 as f32 - pad).max(0.) * scale_x).floor() as u32;
        let top = ((y0 as f32 - pad).max(0.) * scale_y).floor() as u32;
        let right = (((x1 + 1) as f32 + pad) * scale_x)
            .ceil()
            .min(original_w as f32) as u32;
        let bottom = (((y1 + 1) as f32 + pad) * scale_y)
            .ceil()
            .min(original_h as f32) as u32;
        if right > left && bottom > top {
            boxes.push([left, top, right - left, bottom - top]);
        }
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctc_removes_blank_and_only_adjacent_duplicates() {
        let dict = vec!["".into(), "发".into(), "票".into()];
        let data = [
            0., 0.9, 0.1, 0., 0.8, 0.2, 1., 0., 0., 0., 0.9, 0.1, 0., 0.1, 0.9,
        ];
        let (text, score) = decode_ctc(&data, 5, &dict).unwrap();
        assert_eq!(text, "发发票");
        assert!((score - 0.9).abs() < 1e-5);
    }
    #[test]
    fn detector_preserves_bgr_normalization_and_rec_padding() {
        let img = RgbImage::from_pixel(20, 10, image::Rgb([255, 100, 0]));
        let (data, w, h) = detector_input(&img);
        assert_eq!((w, h), (32, 32));
        assert!((data[0] + 0.485 / 0.229).abs() < 1e-5);
        let (rec, w) = recognizer_input(&img);
        assert_eq!(w, 320);
        assert_eq!(rec[0], -1.);
        assert_eq!(rec[100], 0.);
    }
    #[test]
    fn vertical_regions_do_not_get_auto_rotated() {
        let mut prob = vec![0.; 100 * 100];
        for y in 5..15 {
            for x in 5..70 {
                prob[y * 100 + x] = 0.95;
            }
        }
        for y in 25..85 {
            for x in 80..90 {
                prob[y * 100 + x] = 0.95;
            }
        }
        let boxes = horizontal_boxes(&prob, 100, 100, 200, 200);
        assert_eq!(boxes.len(), 1);
        assert!(boxes[0][2] > boxes[0][3]);
    }
    #[test]
    fn adjacent_fragments_join_only_on_same_baseline_and_similar_size() {
        // Real split-title geometry: 湖北增值税普 + 通发票, interrupted by a stamp.
        let title = [[356, 19, 199, 50], [552, 38, 84, 33]];
        assert_eq!(adjacent_line_pairs(&title), vec![[356, 19, 280, 52]]);
        for other in [[600, 38, 84, 33], [552, 80, 84, 33], [552, 55, 84, 16]] {
            assert!(adjacent_line_pairs(&[title[0], other]).is_empty());
        }
        // There is no recursive concatenation of an arbitrary row of table cells.
        let row = [[10, 10, 40, 20], [50, 10, 40, 20], [90, 10, 40, 20]];
        assert!(adjacent_line_pairs(&row).iter().all(|bbox| bbox[2] == 80));
    }
}
