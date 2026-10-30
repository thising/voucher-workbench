#!/bin/sh
set -eu
model_dir=$(CDPATH= cd -- "$(dirname -- "$0")/../models/title-ocr" && pwd)
temporary=""
trap '[ -z "$temporary" ] || rm -f "$temporary"' EXIT HUP INT TERM
verify() {
  if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$1" | cut -d ' ' -f 1)
  else actual=$(shasum -a 256 "$1" | cut -d ' ' -f 1); fi
  if [ "$actual" != "$2" ]; then echo "SHA256校验失败：$1" >&2; exit 1; fi
}
fetch() {
  temporary="$model_dir/$1.download"
  curl --fail --location --retry 2 --connect-timeout 20 "$2" --output "$temporary"
  verify "$temporary" "$3"
  mv "$temporary" "$model_dir/$1"
  temporary=""
}
fetch det.onnx 'https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/e6f4fa85f00e168c862bc462aebca69eef9b3d3d/inference.onnx' a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d
fetch rec.onnx 'https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_rec_onnx/resolve/ed152b8b495f84de93cda5709d768548a9127622/inference.onnx' da72dc72ca4dc220df0dfde68c1dedc31c58d3e76a25871122e5056227d50092
verify "$model_dir/dictionary.txt" d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b
echo '标题OCR检测、识别模型和字典校验完成。运行时不会联网。'
