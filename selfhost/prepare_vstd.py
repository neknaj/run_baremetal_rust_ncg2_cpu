"""RV64IM 向けに、固定版 vstd のソースを所定のディレクトリへ用意する。"""

import argparse
import hashlib
import io
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile
from urllib.request import urlopen


VERSION = "0.0.0-2026-11-01-0251"
ARCHIVE_ROOT = f"vstd-{VERSION}"
SOURCE = f"https://static.crates.io/crates/vstd/{ARCHIVE_ROOT}.crate"
# Core ad268796f9d824bcfca0156e0c4dc7cad972c304 の Cargo.lock。
SOURCE_SHA256 = "7cfcc2fc379ffbcda31111f1a7da2a34d00bd984407cfa85d1edcf64da38dbd2"
PATCH = Path(__file__).resolve().with_name("vstd-target-has-atomic.patch")
PATCH_SHA256 = "15defcc2da8d927ff459c94aee92fd50e5c0c3f02f5ba4cb43ca91eac321e829"


def checked_bytes(content: bytes, expected: str, label: str) -> bytes:
    if hashlib.sha256(content).hexdigest() != expected:
        raise ValueError(f"SHA-256 が一致しない: {label}")
    return content


def get_archive(local: Path | None) -> bytes:
    if local is not None:
        return checked_bytes(local.read_bytes(), SOURCE_SHA256, str(local))
    with urlopen(SOURCE, timeout=30) as response:
        return checked_bytes(response.read(), SOURCE_SHA256, SOURCE)


def snapshot(directory: Path) -> dict[str, str | None]:
    if not directory.is_dir() or directory.is_symlink():
        raise ValueError(f"通常のディレクトリが必要: {directory}")
    entries = {}
    for entry in directory.rglob("*"):
        name = entry.relative_to(directory).as_posix()
        if entry.is_symlink():
            raise ValueError(f"シンボリックリンクを含む: {entry}")
        if entry.is_dir():
            entries[name] = None
        else:
            entries[name] = hashlib.sha256(entry.read_bytes()).hexdigest()
    return entries


def apply_patch(source: Path, staging: Path, patch: Path) -> None:
    # 出力先が別の Git リポジトリ内でも、その作業ツリーの設定を使わない。
    env = os.environ.copy()
    for name in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"):
        env.pop(name, None)
    env["GIT_CEILING_DIRECTORIES"] = str(staging)
    subprocess.run(
        ["git", "apply", "--whitespace=error", str(patch)],
        cwd=source, env=env, check=True,
    )


def prepare(output: Path, local: Path | None) -> None:
    archive = get_archive(local)
    # Windows の Git チェックアウトで改行が変換されても、同じパッチを適用する。
    patch_content = PATCH.read_bytes().replace(b"\r\n", b"\n")
    patch = checked_bytes(patch_content, PATCH_SHA256, str(PATCH))
    output.parent.mkdir(parents=True, exist_ok=True)
    # Cargo が参照する出力先には、展開とパッチ適用を終えたディレクトリだけを置く。
    with tempfile.TemporaryDirectory(prefix=".vstd-prepare-", dir=output.parent) as temporary:
        staging = Path(temporary)
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as package:
            members = package.getmembers()
            for entry in members:
                parts = PurePosixPath(entry.name).parts
                if not parts or parts[0] != ARCHIVE_ROOT or ".." in parts or not (entry.isfile() or entry.isdir()):
                    raise ValueError(f"想定外のアーカイブの項目: {entry.name}")
            package.extractall(staging, members=members, filter="data")
        source = staging / ARCHIVE_ROOT
        staged_patch = staging / PATCH.name
        staged_patch.write_bytes(patch)
        apply_patch(source, staging, staged_patch)
        if not output.exists() and not output.is_symlink():
            try:
                source.rename(output)
                return
            except OSError:
                if not output.exists():
                    raise
        if snapshot(source) != snapshot(output):
            raise ValueError(f"既存の出力が固定版と一致しない: {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="展開済み vstd の配置先")
    parser.add_argument("--archive", type=Path, help="事前に取得した公式 .crate ファイル")
    args = parser.parse_args()
    output = args.output.absolute()
    try:
        prepare(output, args.archive)
    except (OSError, ValueError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f"{error}\n")
    print(output)


if __name__ == "__main__":
    main()
