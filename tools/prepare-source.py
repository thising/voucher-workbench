#!/usr/bin/env python3
"""Audit/export the publishable source allowlist; never recursively copy the workspace."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "tools/public-source.json"
FORBIDDEN_PARTS = {".git", ".local", ".voucher-state", "input", "output", "output_v2", "target", "memory", "scripts", "archive", "__pycache__"}
FORBIDDEN_SUFFIXES = {".onnx", ".pdf", ".jpg", ".jpeg", ".png", ".xls", ".xlsx", ".log", ".pem", ".key", ".p12", ".exe", ".dll", ".zip"}


def source_files():
    paths = set()
    for pattern in json.loads(CONFIG.read_text())["patterns"]:
        matches = [p for p in ROOT.glob(pattern) if not p.is_dir()]
        if not matches:
            raise ValueError(f"Publication pattern has no files: {pattern}")
        for path in matches:
            relative = path.relative_to(ROOT)
            if any(part.startswith("._") for part in relative.parts):
                continue
            if any(part in FORBIDDEN_PARTS for part in relative.parts) or path.suffix.lower() in FORBIDDEN_SUFFIXES:
                raise ValueError(f"Forbidden publication path: {relative}")
            if any(p.is_symlink() for p in [path, *path.parents] if p != ROOT.parent):
                raise ValueError(f"Symlink publication path: {relative}")
            if not path.is_file() or not path.resolve().is_relative_to(ROOT):
                raise ValueError(f"Not a source file: {relative}")
            paths.add(relative.as_posix())
    return sorted(paths)


def audit_staged(paths):
    output = subprocess.check_output(["git", "ls-files", "--stage", "-z"], cwd=ROOT)
    index = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        metadata, name = record.split(b"\t", 1)
        mode, blob, stage = metadata.decode().split()
        path = name.decode()
        if mode not in {"100644", "100755"} or stage != "0":
            raise ValueError(f"Unexpected index entry: {path}")
        index[path] = blob
    if set(index) != set(paths):
        raise ValueError(f"Index differs from allowlist: extra={sorted(set(index)-set(paths))}, missing={sorted(set(paths)-set(index))}")
    for path in paths:
        data = (ROOT / path).read_bytes()
        expected = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        if index[path] != expected:
            raise ValueError(f"Index is not the reviewed working copy: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", type=Path, help="new, nonexistent directory for a source-only snapshot")
    parser.add_argument("--verify-staged", action="store_true", help="require exact index/allowlist/content equality")
    parser.add_argument("--write-gitignore", action="store_true", help="write deny-by-default ignore rules from the same allowlist")
    parser.add_argument("--list", action="store_true", help="print only source paths, one per line")
    args = parser.parse_args()
    if args.write_gitignore:
        patterns = json.loads(CONFIG.read_text())["patterns"]
        lines = ["# Current tool only. Historical specs and private audit files remain local.", "*", "!*/"]
        lines += ["!/" + pattern for pattern in patterns]
        lines += ["", "# Always exclude private/generated material, including accidental nested copies.", "**/._*", "**/.DS_Store", "**/__pycache__/", "**/*.onnx", "/.local/", "/.voucher-state/", "/input/", "/output/", "/output_v2/", "/memory/", "/scripts/", "/app/target/", "/openspec/changes/"]
        (ROOT / ".gitignore").write_text("\n".join(lines) + "\n")
    paths = source_files()
    if args.verify_staged:
        audit_staged(paths)
    records = [{"path": path, "bytes": (ROOT / path).stat().st_size,
                "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest()} for path in paths]
    if args.export:
        destination = args.export.resolve()
        if destination.exists() or ROOT == destination or ROOT.is_relative_to(destination):
            raise ValueError("Export requires a new directory, not the workspace or its parent")
        destination.mkdir(parents=True)
        for path in paths:
            target = destination / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / path, target)
        (destination / "SOURCE-MANIFEST.json").write_text(json.dumps({"schema": 1, "files": records}, indent=2) + "\n")
    if args.list:
        print("\n".join(paths))
    else:
        print(json.dumps({"source_files": len(paths), "bytes": sum(r["bytes"] for r in records),
                          "files": records, "staged_verified": args.verify_staged}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
