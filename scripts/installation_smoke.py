#!/usr/bin/env python3
"""Exercise actual verified native installation and failure preservation."""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

import package_binary
import terminal_smoke


def sha(path):
    return package_binary.digest_file(path)


def install(root, archive, checksums, destination):
    if os.name == "nt":
        command = ["powershell.exe", "-NoProfile", "-NonInteractive", "-File", str(root / "install.ps1"),
                   "-Archive", str(archive), "-Checksums", str(checksums), "-InstallDir", str(destination)]
    else:
        command = ["sh", str(root / "install.sh"), "--archive", str(archive),
                   "--checksums", str(checksums), "--install-dir", str(destination)]
    return terminal_smoke.run_owned(command, timeout=90, text=True)


def snapshot(destination):
    return {str(path.relative_to(destination)): sha(path) for path in destination.rglob("*") if path.is_file()}


def malformed_archive(original, destination, member, link=False, broken_binary=False):
    """Re-sign a deliberately invalid archive so validation reaches the member boundary."""
    if original.suffix == ".zip":
        with zipfile.ZipFile(original) as source, zipfile.ZipFile(destination, "w") as output:
            for entry in source.infolist():
                if link and entry.filename == member:
                    continue
                content = source.read(entry)
                if broken_binary and entry.filename == "ev.exe": content = b"not a runnable native executable\0"
                output.writestr(entry, content)
            if member:
                entry = zipfile.ZipInfo(member)
                if link:
                    entry.create_system = 3
                    entry.external_attr = (0o120777 << 16)
                output.writestr(entry, b"outside")
    else:
        with tarfile.open(original) as source, tarfile.open(destination, "w:gz", format=tarfile.USTAR_FORMAT) as output:
            for entry in source.getmembers():
                if link and entry.name == member:
                    continue
                content = source.extractfile(entry).read()
                if broken_binary and entry.name == "ev": content = b"\0not a runnable native executable\0"
                entry.size = len(content)
                output.addfile(entry, io.BytesIO(content))
            if member:
                entry = tarfile.TarInfo(member)
                if link:
                    entry.type = tarfile.SYMTYPE
                    entry.linkname = "outside"
                else: entry.size = 7
                output.addfile(entry, None if link else io.BytesIO(b"outside"))
    checksum = destination.parent / "SHA256SUMS"
    checksum.write_text(f"{sha(destination)}  {destination.name}\n", encoding="ascii")
    return checksum


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-dir", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    packages = Path(args.package_dir).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    extension = ".zip" if os.name == "nt" else ".tar.gz"
    archive = packages / f"ev-{package_binary.native_target()}{extension}"
    checksums = packages / "SHA256SUMS"
    binary_name = "ev.exe" if os.name == "nt" else "ev"
    destination = output / "installed 空 格" / "bin"
    evidence = {"status": "failed", "cases": [], "installed_binary": str(destination / binary_name)}
    def run_case(name, action):
        action()
        evidence["cases"].append(name)
        print("passed:", name, flush=True)
    def succeeds(a=archive, c=checksums, d=destination):
        result = install(root, a, c, d)
        assert result.returncode == 0, result.stdout + result.stderr
    def rejects(a, c, d=destination):
        before = snapshot(d) if d.exists() else {}
        result = install(root, a, c, d)
        assert result.returncode != 0, "invalid installation unexpectedly succeeded"
        assert (snapshot(d) if d.exists() else {}) == before, "failed install changed executable/metadata"
    try:
        run_case("fresh-spaces-CJK", succeeds)
        version = subprocess.check_output([str(destination / binary_name), "--version"], text=True).strip()
        assert re.fullmatch(r"ev [0-9][0-9A-Za-z.+_-]*", version)
        marker = (destination / ".ev-install/.native-install").read_text()
        assert marker == f"ev-native-install-v1\nversion={version}\nsha256={sha(destination / binary_name)}\n"
        run_case("owned-reinstall", succeeds)
        with tempfile.TemporaryDirectory(prefix="failures-", dir=output) as temporary:
            temporary = Path(temporary)
            bad_checksums = temporary / "bad-sums"
            bad_checksums.write_text(f"{'0' * 64}  {archive.name}\n")
            run_case("checksum-mismatch-preserves-install", lambda: rejects(archive, bad_checksums))
            bad_checksums.write_text(checksums.read_text() * 2)
            run_case("duplicate-checksum-preserves-install", lambda: rejects(archive, bad_checksums))
            for name, member, link in (("path-traversal", "../outside", False), ("link-member", binary_name, True),
                                       ("duplicate-member", binary_name, False)):
                folder = temporary / name
                folder.mkdir()
                malicious = folder / archive.name
                sums = malformed_archive(archive, malicious, member, link=link)
                run_case(name + "-preserves-install", lambda a=malicious,c=sums: rejects(a,c))
                assert not (temporary / "outside").exists(), "archive escaped staging"
            folder = temporary / "broken-version"
            folder.mkdir()
            malicious = folder / archive.name
            sums = malformed_archive(archive, malicious, None, broken_binary=True)
            run_case("failed-staged-version-preserves-install", lambda: rejects(malicious,sums))
            unowned = temporary / "unowned"
            unowned.mkdir()
            (unowned / binary_name).write_bytes(b"unrelated existing command")
            run_case("unowned-command-preserved", lambda: rejects(archive,checksums,unowned))
            saved = (destination / binary_name).read_bytes()
            (destination / binary_name).write_bytes(saved + b"modified")
            run_case("changed-owned-command-preserved", lambda: rejects(archive,checksums))
            (destination / binary_name).write_bytes(saved)
            if os.name == "nt":
                from ctypes import wintypes
                kernel = ctypes.WinDLL("kernel32", use_last_error=True)
                kernel.CreateFileW.argtypes = [wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,wintypes.LPVOID,wintypes.DWORD,wintypes.DWORD,wintypes.HANDLE]
                kernel.CreateFileW.restype = wintypes.HANDLE
                kernel.CloseHandle.argtypes = [wintypes.HANDLE]
                handle = kernel.CreateFileW(str(destination / binary_name),0x80000000,1,None,3,0,None)
                assert handle != ctypes.c_void_p(-1).value
                try: run_case("locked-executable-publication-rollback", lambda: rejects(archive,checksums))
                finally: kernel.CloseHandle(handle)
            else:
                link = temporary / "linked-destination"
                link.symlink_to(destination, target_is_directory=True)
                run_case("symlink-destination-preserved", lambda: rejects(archive,checksums,link))
        # Compile the real source under another package version; no fake version wrapper.
        with tempfile.TemporaryDirectory(prefix="upgrade-source-",dir=output) as temporary:
            package = Path(temporary)
            shutil.copytree(root / "src", package / "src")
            shutil.copyfile(root / "Cargo.lock", package / "Cargo.lock")
            manifest = (root / "Cargo.toml").read_text()
            current = version.split()[1]
            assert f'version = "{current}"' in manifest
            upgrade = current + "-install-smoke"
            (package / "Cargo.toml").write_text(manifest.replace(f'version = "{current}"',f'version = "{upgrade}"',1))
            result = terminal_smoke.run_owned(["cargo","build","--release","--manifest-path",str(package / "Cargo.toml"),
                                               "--target-dir",str(root / "target")],timeout=900,text=True)
            (output / "upgrade-build.log").write_text(result.stdout + result.stderr)
            assert result.returncode == 0, "real upgrade source build failed; see upgrade-build.log"
            upgrade_binary = root / "target/release" / binary_name
            with tempfile.TemporaryDirectory(prefix="upgrade-package-",dir=output) as upgrade_dir:
                upgrade_dir = Path(upgrade_dir)
                members = (binary_name,"LICENSE","THIRD_PARTY_NOTICES")
                if extension == ".zip":
                    with zipfile.ZipFile(archive) as source:
                        assert set(source.namelist()) == set(members)
                        for member in members:
                            (upgrade_dir / member).write_bytes(source.read(member))
                else:
                    with tarfile.open(archive) as source:
                        assert set(source.getnames()) == set(members)
                        for member in members:
                            entry = source.getmember(member)
                            assert entry.isfile() and entry.size <= 64 * 1024 * 1024
                            with source.extractfile(entry) as content, (upgrade_dir / member).open("wb") as target:
                                shutil.copyfileobj(content,target)
                    (upgrade_dir / binary_name).chmod(0o755)
                shutil.copyfile(upgrade_binary, upgrade_dir / binary_name)
                upgraded_archive = upgrade_dir / archive.name
                if extension == ".zip":
                    with zipfile.ZipFile(upgraded_archive,"w") as bundle:
                        for member in (binary_name,"LICENSE","THIRD_PARTY_NOTICES"): bundle.write(upgrade_dir / member,member)
                else:
                    with tarfile.open(upgraded_archive,"w:gz",format=tarfile.USTAR_FORMAT) as bundle:
                        for member in (binary_name,"LICENSE","THIRD_PARTY_NOTICES"): bundle.add(upgrade_dir / member,arcname=member,recursive=False)
                upgrade_sums = upgrade_dir / "SHA256SUMS"
                upgrade_sums.write_text(f"{sha(upgraded_archive)}  {archive.name}\n")
                run_case("real-version-upgrade", lambda: succeeds(upgraded_archive,upgrade_sums))
                assert subprocess.check_output([str(destination / binary_name),"--version"],text=True).strip() == "ev " + upgrade
                run_case("restore-current-version", succeeds)
        # Run the installed executable, not the source build, through the real native terminal harness.
        result = terminal_smoke.run_owned([sys.executable,str(root / "scripts/terminal_smoke.py"),"--binary",str(destination / binary_name),
                                          "--output",str(output / "terminal")],timeout=1100,text=True)
        (output / "terminal-smoke.log").write_text(result.stdout + result.stderr)
        assert result.returncode == 0, result.stdout + result.stderr
        evidence["cases"].append("installed-native-34-scenarios")
        evidence["status"] = "passed"
    except BaseException as error:
        evidence["error"] = str(error)
        raise
    finally:
        (output / "results.json").write_text(json.dumps(evidence,indent=2,ensure_ascii=False)+"\n")


if __name__ == "__main__":
    main()
