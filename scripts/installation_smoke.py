#!/usr/bin/env python3
"""Exercise actual verified native installation and failure preservation."""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import io
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
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


class ReleaseFixture:
    """Serve actual native packages; transport rewrites exist only in throwaway source."""

    def __init__(self, archive, checksums, tag):
        self.requests = []
        self.corrupt_checksum = False
        self.throttle = False
        self.animation_observed = threading.Event()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_HEAD(self):
                self.respond(False)

            def do_GET(self):
                self.respond(True)

            def respond(self, body):
                fixture.requests.append(self.path)
                if self.path == "/releases/latest":
                    if fixture.throttle:
                        fixture.animation_observed.wait(timeout=10)
                    self.send_response(302)
                    self.send_header("Location", fixture.base + "/tag/" + tag)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if self.path == "/releases/tag/" + tag:
                    self.send_response(200)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                prefix = "/releases/download/" + tag + "/"
                if self.path == prefix + "SHA256SUMS":
                    content = (f"{'0' * 64}  {archive.name}\n" if fixture.corrupt_checksum
                               else checksums.read_text()).encode("ascii")
                    # Build artifacts omit installer entries; public manifests
                    # already include them and must remain duplicate-free.
                    names = {line.split()[-1] for line in content.splitlines()}
                    for name, digest in ((b"install.sh", b"1" * 64), (b"install.ps1", b"2" * 64)):
                        if name not in names:
                            content += digest + b"  " + name + b"\n"
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(content)))
                    self.end_headers()
                    if body:
                        self.wfile.write(content)
                    return
                if self.path == prefix + archive.name:
                    self.send_response(200)
                    self.send_header("Content-Length", str(archive.stat().st_size))
                    self.end_headers()
                    if body:
                        if fixture.throttle:
                            # Keep an actual download pending until the terminal
                            # observer has seen motion across both resize steps.
                            # Package size/cache speed must not set that window.
                            fixture.animation_observed.wait(timeout=10)
                        with archive.open("rb") as source:
                            while chunk := source.read(256 * 1024):
                                self.wfile.write(chunk)
                                self.wfile.flush()
                                if fixture.throttle:
                                    time.sleep(0.05)
                    return
                self.send_error(404)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.base = f"http://127.0.0.1:{self.server.server_port}/releases"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def configure_source(self, package):
        for relative in ("src/platform/distribution.rs", "install.sh", "install.ps1"):
            path = package / relative
            source = path.read_text()
            source = source.replace("https://github.com/kimmandoo/eevee/releases", self.base)
            source = source.replace(".https_only(true)", ".https_only(false)")
            source = source.replace("'=https'", "'=http'")
            source = source.replace("$Uri.Scheme -cne 'https'", "$Uri.Scheme -cne 'http'")
            source = source.replace(r"https://github\.com/kimmandoo/eevee/releases",
                                    re.escape(self.base))
            path.write_text(source)


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
    sprite_home = Path(tempfile.mkdtemp(prefix="sprite-user-", dir=output))
    sprite_cache = sprite_home / "cache" / "eevee" / "sprites"
    sprite_environment = {
        "HOME": str(sprite_home),
        "USERPROFILE": str(sprite_home),
        "XDG_CACHE_HOME": str(sprite_home / "cache"),
        "XDG_DATA_HOME": str(sprite_home / "data"),
        "LOCALAPPDATA": str(sprite_home / "cache"),
        "APPDATA": str(sprite_home / "config"),
    }
    artwork_source = (root / "src/ui/artwork.rs").read_text()
    hashes = re.findall(r'"([0-9a-f]{64})"', artwork_source.split("const HASHES:", 1)[1].split("];", 1)[0])
    species = (133, 134, 135, 136, 196, 197, 470, 471, 700)
    assert len(hashes) == len(species)
    revision = re.search(r'const REVISION: &str = "([0-9a-f]{40})"', artwork_source).group(1)

    def verified_sprites():
        manifest = json.loads((sprite_cache / "sources.json").read_text())
        assert manifest["revision"] == revision
        assert len(manifest["images"]) == 9
        assert {record["id"] for record in manifest["images"]} == set(species)
        records = {record["id"]: record for record in manifest["images"]}
        for species_id, expected in zip(species, hashes):
            path = sprite_cache / f"{species_id}.png"
            data = path.read_bytes()
            assert sha(path) == expected == records[species_id]["sha256"]
            assert records[species_id]["url"] == (
                f"https://raw.githubusercontent.com/PokeAPI/sprites/{revision}/sprites/pokemon/{species_id}.png"
            )
            assert data[:8] == b"\x89PNG\r\n\x1a\n"
            assert (int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")) == (96, 96)
        return {path.name: (sha(path), path.stat().st_mtime_ns) for path in sprite_cache.iterdir()}

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
        original_hash = sha(destination / binary_name)
        marker = (destination / ".ev-install/.native-install").read_text()
        assert marker == (
            f"ev-native-install-v2\nversion={version}\nsha256={sha(destination / binary_name)}\n"
            f"license-sha256={sha(destination / '.ev-install/LICENSE')}\n"
            f"notices-sha256={sha(destination / '.ev-install/THIRD_PARTY_NOTICES')}\n"
        )
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
        # Compile a real older binary whose release transport is isolated to localhost.
        current = version.split()[1]
        with ReleaseFixture(archive, checksums, "v" + current) as release, \
                tempfile.TemporaryDirectory(prefix="upgrade-source-", dir=output) as temporary:
            package = Path(temporary)
            shutil.copytree(root / "src", package / "src")
            shutil.copyfile(root / "build.rs", package / "build.rs")
            shutil.copytree(root / "locales", package / "locales")
            shutil.copyfile(root / "Cargo.lock", package / "Cargo.lock")
            for installer in ("install.sh", "install.ps1"):
                shutil.copyfile(root / installer, package / installer)
            manifest = (root / "Cargo.toml").read_text()
            assert f'version = "{current}"' in manifest
            upgrade = "0.0.0"
            assert current != upgrade
            fixture_manifest = manifest.replace(f'version = "{current}"', f'version = "{upgrade}"', 1)
            fixture_manifest = fixture_manifest.replace('name = "ev"', 'name = "ev-lifecycle-fixture"', 1)
            (package / "Cargo.toml").write_text(fixture_manifest)
            release.configure_source(package)
            cargo_target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve()
            result = terminal_smoke.run_owned(["cargo","build","--release","--manifest-path",str(package / "Cargo.toml"),
                                               "--target-dir",str(cargo_target)],timeout=900,text=True)
            (output / "upgrade-build.log").write_text(result.stdout + result.stderr)
            assert result.returncode == 0, "real upgrade source build failed; see upgrade-build.log"
            upgrade_binary = cargo_target / "release" / ("ev-lifecycle-fixture.exe" if os.name == "nt" else "ev-lifecycle-fixture")

            animation_drawings = {}

            def animated_install(command, environment, capture_name, retired_directory=None):
                release.throttle = True
                release.animation_observed.clear()
                if os.name != "nt":
                    # Both bootstrap and self-update execute a shell. Keep it
                    # below the session leader, as in an ordinary user shell,
                    # so macOS cannot revoke the PTY before restoration checks.
                    command = [sys.executable, "-c",
                               "import subprocess,sys; sys.exit(subprocess.call(sys.argv[1:]))",
                               *command]
                session = terminal_smoke.Session(command, root, environment, (120, 36), output / capture_name)
                try:
                    deadline = time.monotonic() + 360
                    columns = 120
                    frames = {}
                    narrow_frames = set()
                    restored_frames = set()
                    resized = False
                    restored = False
                    while session.alive() and time.monotonic() < deadline:
                        session.pump(0.1)
                        rows = [row.rstrip() for row in session.display().splitlines()]
                        # Compare visible drawings under the same phase, not a
                        # particular glyph, frame sequence, or screen position.
                        drawing = tuple(row for row in rows if row.strip()
                                        and not re.search(r"[A-NP-Za-np-z0-9]", row))
                        if len(drawing) < 3:
                            continue
                        phase = tuple(row for row in rows if row.strip() and row not in drawing)
                        frames.setdefault(phase, set()).add(drawing)
                        assert all(len(row) < columns for row in drawing), \
                            "animation reaches the wrap column"
                        if not resized and len(frames[phase]) >= 3:
                            session.capture("visible-motion")
                            session.resize((24, 20))
                            columns = 24
                            resized = True
                        elif resized and not restored:
                            narrow_frames.add(drawing)
                            if len(narrow_frames) >= 3:
                                session.capture("narrow-visible-motion")
                                session.resize((120, 36))
                                columns = 120
                                restored = True
                        elif restored:
                            restored_frames.add(drawing)
                            if len(restored_frames) >= 3:
                                if not release.animation_observed.is_set():
                                    session.capture("restored-visible-motion")
                                release.animation_observed.set()
                    assert not session.alive(), "interactive release installation timed out"
                    assert session.wait_exit(tui=False) == 0, session.display()
                    assert any(len(seen) >= 3 for seen in frames.values()), \
                        "no meaningful visible motion within a stable phase"
                    assert restored, "animation did not remain visible after a narrow resize"
                    assert len(restored_frames) >= 3, "animation did not resume at the restored width"
                    resolution_frames = set()
                    for phase, seen in frames.items():
                        if any("Resolving latest Eevee release" in line for line in phase):
                            resolution_frames.update(seen)
                    assert len(resolution_frames) >= 3, \
                        "release lookup did not show the install animation"
                    animation_drawings[capture_name] = set().union(*frames.values())
                    for parameters in re.findall(r"\x1b\[([0-9;:]*)m", session.text):
                        assert not any(30 <= int(code) <= 49 or 90 <= int(code) <= 107
                                       for code in re.split("[;:]", parameters) if code), \
                            "NO_COLOR animation emitted a color attribute"
                    if retired_directory is not None:
                        # Keep the owned Job alive until the post-exit cleanup
                        # helper finishes; closing Session would kill it first.
                        cleanup_deadline = time.monotonic() + 10
                        while list(retired_directory.glob(".ev-stage*")):
                            assert time.monotonic() < cleanup_deadline, "retired self-update files were not cleaned"
                            session.pump(0.05)
                finally:
                    release.animation_observed.set()
                    session.close()
                    release.throttle = False

            def static_install(command, environment, name, size):
                release.throttle = False
                if os.name != "nt":
                    command = [sys.executable, "-c",
                               "import subprocess,sys; sys.exit(subprocess.call(sys.argv[1:]))", *command]
                session = terminal_smoke.Session(command, root, environment, size, output / name)
                try:
                    deadline = time.monotonic() + 360
                    while session.alive() and time.monotonic() < deadline:
                        session.pump(.05)
                    assert not session.alive(), "static installation exceeded deadline"
                    assert session.wait_exit(tui=False) == 0, session.display()
                    assert "\x1b" not in session.text, "explicit opt-out emitted terminal animation/escapes"
                    assert "Publishing verified Eevee installation" in session.text
                    assert "Installed ev " in session.text
                    session.capture("truthful-static-phases")
                finally:
                    session.close()

            def bootstrap():
                downloaded = output / "downloaded-native" / "bin"
                command = (["powershell.exe", "-NoLogo", "-NoProfile", "-File", str(package / "install.ps1"),
                            "-InstallDir", str(downloaded)] if os.name == "nt" else
                           ["sh", str(package / "install.sh"), "--install-dir", str(downloaded)])
                environment = terminal_smoke.terminal_environment("mono")
                environment.update(sprite_environment)
                environment.pop("CI", None)
                environment.pop("EV_VERSION", None)
                environment["NO_COLOR"] = "1"
                animated_install(command, environment, "installer-animation")
                assert sha(downloaded / binary_name) == original_hash
                cached = verified_sprites()
                evidence["sprite_cache"] = str(sprite_cache)
                launch = terminal_smoke.Session(
                    [str(downloaded / binary_name), "--images", "cells"],
                    root, environment, (120, 36), output / "installed-sprites",
                )
                try:
                    deadline = time.monotonic() + 8
                    while "Storage: saved" not in launch.display() and time.monotonic() < deadline:
                        assert launch.alive(), "installed executable exited before first launch"
                        launch.pump(0.1)
                    launch.command("images")  # Inspect locally loaded assets; no download action.
                    deadline = time.monotonic() + 8
                    while not re.search(r"\b9/9\b", launch.display()) and time.monotonic() < deadline:
                        assert launch.alive(), "installed executable exited while inspecting sprites"
                        launch.pump(0.1)
                    assert re.search(r"\b9/9\b", launch.display()), "installed sprites were not all loaded"
                    assert verified_sprites() == cached, "first launch modified the installed sprite cache"
                    launch.capture("first-launch-nine-original-sprites")
                    launch.send("\x03")
                    assert launch.wait_exit() == 0
                finally:
                    launch.close()
                evidence["cases"].append("first-launch-loads-nine-installed-sprites")
                result = terminal_smoke.run_owned(command, timeout=360, text=True, env=environment)
                assert result.returncode == 0, result.stdout + result.stderr
                assert verified_sprites() == cached, "repeat installation rewrote matching cached sprites"
                evidence["cases"].append("nine-pinned-sprites-installed-and-reused")
                conflict = sprite_cache / "133.png"
                original = conflict.read_bytes()
                before = snapshot(downloaded)
                try:
                    conflict.write_bytes(b"preserve conflicting user image")
                    result = terminal_smoke.run_owned(command, timeout=90, text=True, env=environment)
                    assert result.returncode != 0, "conflicting sprite unexpectedly published an installation"
                    assert "sprite" in (result.stdout + result.stderr).lower()
                    assert snapshot(downloaded) == before
                    assert conflict.read_bytes() == b"preserve conflicting user image"
                finally:
                    conflict.write_bytes(original)
                evidence["cases"].append("sprite-failure-preserves-install-and-conflicting-file")
                result = terminal_smoke.run_owned([str(downloaded / binary_name), "uninstall"], timeout=180, text=True)
                assert result.returncode == 0, result.stdout + result.stderr
                assert not (downloaded / binary_name).exists()
                assert not list(downloaded.glob(".ev-stage*"))

            run_case("latest-bootstrap-and-monochrome-capture-ball", bootstrap)
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
                run_case("install-real-older-version", lambda: succeeds(upgraded_archive, upgrade_sums))
                assert subprocess.check_output([str(destination / binary_name), "--version"], text=True).strip() == "ev " + upgrade

                def lifecycle(arguments):
                    environment = os.environ.copy()
                    environment.update(sprite_environment)
                    environment.pop("EV_VERSION", None)
                    result = terminal_smoke.run_owned(
                        [str(destination / binary_name), *arguments],
                        timeout=360, text=True, env=environment,
                    )
                    assert "\x1b" not in result.stdout + result.stderr, "redirected lifecycle output contains terminal escapes"
                    (output / ("-".join(arg.strip("-") for arg in arguments) + ".log")).write_text(
                        result.stdout + result.stderr
                    )
                    return result

                def check_only():
                    before = snapshot(destination)
                    release.requests.clear()
                    result = lifecycle(["update", "--check"])
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert snapshot(destination) == before
                    assert release.requests == ["/releases/latest"], release.requests

                run_case("self-update-check-is-read-only", check_only)

                def corrupt_update():
                    before = snapshot(destination)
                    release.corrupt_checksum = True
                    try:
                        result = lifecycle(["update"])
                        assert result.returncode != 0, result.stdout + result.stderr
                        assert snapshot(destination) == before, "failed self-update changed owned files"
                    finally:
                        release.corrupt_checksum = False

                run_case("self-update-checksum-failure-preserves-install", corrupt_update)
                def reduced_motion_install_update():
                    environment = terminal_smoke.terminal_environment("mono")
                    environment.update(sprite_environment)
                    environment.pop("CI", None)
                    environment.pop("EV_VERSION", None)
                    for size in [(120, 36), (60, 18)]:
                        static_destination = output / f"static-install-{size[0]}" / "bin"
                        command = (["powershell.exe", "-NoLogo", "-NoProfile", "-File", str(package / "install.ps1"),
                                    "-NoAnimation", "-InstallDir", str(static_destination)] if os.name == "nt" else
                                   ["sh", str(package / "install.sh"), "--no-animation", "--install-dir", str(static_destination)])
                        static_install(command, environment, f"installer-optout-{size[0]}", size)
                        assert sha(static_destination / binary_name) == original_hash
                        succeeds(upgraded_archive, upgrade_sums)
                        static_install([str(destination / binary_name), "update", "--no-animation"],
                                       environment, f"updater-optout-{size[0]}", size)
                        assert sha(destination / binary_name) == original_hash
                    succeeds(upgraded_archive, upgrade_sums)
                    result = lifecycle(["update", "--no-animation"])
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert "Publishing verified Eevee installation" in result.stdout + result.stderr
                    succeeds(upgraded_archive, upgrade_sums)

                run_case("installer-and-self-update-explicit-motion-optout-both-sizes-and-redirected", reduced_motion_install_update)


                def self_update():
                    environment = terminal_smoke.terminal_environment("mono")
                    environment.update(sprite_environment)
                    environment.pop("CI", None)
                    environment.pop("EV_VERSION", None)
                    environment["NO_COLOR"] = "1"
                    animated_install([str(destination / binary_name), "update"],
                                     environment, "self-update-animation",
                                     retired_directory=destination if os.name == "nt" else None)
                    shared_frames = (animation_drawings["installer-animation"]
                                     & animation_drawings["self-update-animation"])
                    assert len(shared_frames) >= 3, \
                        "self-update did not preserve the install artwork and motion"
                    evidence["shared_install_update_frames"] = len(shared_frames)
                    assert subprocess.check_output([str(destination / binary_name), "--version"], text=True).strip() == version
                    assert sha(destination / binary_name) == original_hash
                    assert not list(destination.glob(".ev-stage*")), "retired self-update files were not cleaned"

                run_case("live-owned-binary-self-update", self_update)
                evidence["release_requests"] = list(release.requests)
        # Run the installed executable, not the source build, through the real native terminal harness.
        result = terminal_smoke.run_owned([sys.executable,str(root / "scripts/terminal_smoke.py"),"--binary",str(destination / binary_name),
                                          "--output",str(output / "terminal")],timeout=1100,text=True)
        (output / "terminal-smoke.log").write_text(result.stdout + result.stderr)
        assert result.returncode == 0, result.stdout + result.stderr
        evidence["cases"].append("installed-native-34-scenarios")
        unrelated = destination / "user-task-data"
        unrelated.mkdir()
        (unrelated / "keep.json").write_text('{"goal":"Keep user work through uninstall"}\n')
        kept = snapshot(unrelated)

        def unrecognized_metadata():
            extra = destination / ".ev-install/user-note"
            extra.write_text("Not owned by the installer\n")
            try:
                before = snapshot(destination)
                result = lifecycle(["uninstall"])
                assert result.returncode != 0, result.stdout + result.stderr
                assert snapshot(destination) == before
            finally:
                extra.unlink()

        run_case("uninstall-refuses-unrecognized-metadata", unrecognized_metadata)

        def uninstall_owned():
            result = lifecycle(["uninstall"])
            assert result.returncode == 0, result.stdout + result.stderr
            assert not (destination / binary_name).exists()
            assert not (destination / ".ev-install").exists()
            assert snapshot(unrelated) == kept
            assert not list(destination.glob(".ev-stage*")), "retired uninstall files were not cleaned"

        run_case("live-owned-uninstall-keeps-user-data", uninstall_owned)
        run_case("reinstall-after-uninstall", succeeds)
        evidence["status"] = "passed"
    except BaseException as error:
        evidence["error"] = str(error)
        raise
    finally:
        (output / "results.json").write_text(json.dumps(evidence,indent=2,ensure_ascii=False)+"\n")


if __name__ == "__main__":
    main()
