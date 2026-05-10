#!/usr/bin/env python3

import argparse
import hashlib
import re
import shutil
import tarfile
import tempfile
import zipfile
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def copy_tree(src: Path, dst: Path) -> None:
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)


def sanitize_segment(value: str) -> str:
    sanitized = re.sub(r"[^A-Za-z0-9._-]+", "-", value).strip(".-")
    if not sanitized:
        raise SystemExit(f"invalid archive name segment: {value!r}")
    return sanitized


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--name", default="codex-shim")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    binary = (repo_root / args.binary).resolve()
    if not binary.exists():
        raise SystemExit(f"binary not found: {binary}")

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    package_name = (
        f"{sanitize_segment(args.name)}-{sanitize_segment(args.version)}-"
        f"{sanitize_segment(args.target)}"
    )
    with tempfile.TemporaryDirectory() as temp_dir:
        stage_root = Path(temp_dir) / package_name
        stage_root.mkdir(parents=True, exist_ok=True)

        shutil.copy2(binary, stage_root / binary.name)
        shutil.copy2(repo_root / "README.md", stage_root / "README.md")
        shutil.copy2(repo_root / "LICENSE", stage_root / "LICENSE")
        copy_tree(repo_root / "examples", stage_root / "examples")

        if binary.suffix.lower() == ".exe" or "windows" in args.target:
            archive = output_dir / f"{package_name}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zf:
                for file_path in stage_root.rglob("*"):
                    if file_path.is_dir():
                        continue
                    zf.write(file_path, file_path.relative_to(stage_root.parent))
        else:
            archive = output_dir / f"{package_name}.tar.gz"
            with tarfile.open(archive, "w:gz") as tf:
                tf.add(stage_root, arcname=package_name)

    checksum_path = archive.with_suffix(archive.suffix + ".sha256")
    checksum_path.write_text(f"{sha256(archive)}  {archive.name}\n", encoding="utf-8")
    print(archive)
    print(checksum_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
