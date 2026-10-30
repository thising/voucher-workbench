#!/bin/sh
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$project_root"
if [ -z "${ORT_DYLIB_PATH:-}" ]; then
  case "$(uname -s)" in
    Darwin) local_library="$project_root/.local/onnxruntime/libonnxruntime.dylib" ;;
    *) local_library="$project_root/.local/onnxruntime/libonnxruntime.so" ;;
  esac
  if [ -f "$local_library" ]; then export ORT_DYLIB_PATH="$local_library"; fi
fi
sh "$project_root/app/tools/cargo-local.sh" build --release --locked
exec "$project_root/app/target/release/voucher-workbench" --model "$project_root/app/models/title-ocr" --state-dir "$project_root/.voucher-state" "$@"
