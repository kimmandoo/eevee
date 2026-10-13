#!/usr/bin/env python3
"""Build a native verification archive, not a public release."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import tarfile
import tempfile
import zipfile

TARGETS = {("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x86_64",
           ("Linux", "arm64"): "linux-arm64", ("Linux", "x86_64"): "linux-x86_64",
           ("Windows", "x86_64"): "windows-x86_64"}


def digest_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as content:
        for block in iter(lambda: content.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def native_target():
    machine = platform.machine().lower()
    arch = {"aarch64": "arm64", "amd64": "x86_64"}.get(machine, machine)
    try:
        return TARGETS[platform.system(), arch]
    except KeyError:
        raise SystemExit(f"unqualified native package target: {platform.system()}/{machine}")


def verify_executable(path, target):
    with path.open("rb") as binary:
        header = binary.read(64)
        if target.startswith("darwin-"):
            expected = 0x0100000C if target.endswith("arm64") else 0x01000007
            assert header[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<I", header, 4)[0] == expected, "wrong Mach-O target"
        elif target.startswith("linux-"):
            expected = 183 if target.endswith("arm64") else 62
            assert header[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", header, 18)[0] == expected, "wrong ELF target"
        else:
            assert header[:2] == b"MZ", "not a Windows executable"
            binary.seek(struct.unpack_from("<I", header, 60)[0])
            pe = binary.read(6)
            assert pe[:4] == b"PE\0\0" and struct.unpack_from("<H", pe, 4)[0] == 0x8664, "wrong PE target"
    version = subprocess.run([str(path), "--version"], capture_output=True, text=True, timeout=10, check=True).stdout.strip()
    assert version.startswith("ev ") and len(version.splitlines()) == 1, "unexpected executable version"
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--notices", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    target = native_target()
    binary = Path(args.binary).resolve()
    if platform.system() == "Windows" and not binary.exists():
        binary = binary.with_suffix(".exe")
    notices = Path(args.notices).resolve()
    for source in (binary, root / "LICENSE", notices):
        assert source.is_file() and not source.is_symlink(), f"invalid package input: {source}"
        assert 0 < source.stat().st_size <= 64 * 1024 * 1024, f"package input exceeds bound: {source}"
    version = verify_executable(binary, target)
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    extension = ".zip" if platform.system() == "Windows" else ".tar.gz"
    name = f"ev-{target}{extension}"
    members = [("ev.exe" if extension == ".zip" else "ev", binary),
               ("LICENSE", root / "LICENSE"), ("THIRD_PARTY_NOTICES", notices)]
    assert sum(source.stat().st_size for _, source in members) <= 96 * 1024 * 1024, "expanded package exceeds installer bound"
    with tempfile.TemporaryDirectory(prefix="package-", dir=output) as temporary:
        archive = Path(temporary) / name
        if extension == ".zip":
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as package:
                for filename, source in members:
                    package.write(source, filename)
        else:
            with tarfile.open(archive, "w:gz", format=tarfile.USTAR_FORMAT) as package:
                for filename, source in members:
                    information = package.gettarinfo(str(source), filename)
                    information.uid = information.gid = 0
                    information.uname = information.gname = ""
                    information.mode = 0o755 if filename == "ev" else 0o644
                    with source.open("rb") as content:
                        package.addfile(information, content)
        assert archive.stat().st_size <= 64 * 1024 * 1024, "archive exceeds installer bound"
        digest = digest_file(archive)
        os.replace(archive, output / name)
        (output / "SHA256SUMS").write_text(f"{digest}  {name}\n", encoding="ascii")
    for installer in ("install.sh", "install.ps1"):
        shutil.copyfile(root / installer, output / installer)
    print(f"Packaged {version}: {output / name}")
    print("Package includes code/dependency notices, no artwork or provider credentials.")


if __name__ == "__main__":
    main()
