#!/usr/bin/env python3
"""Native TUI proof for immutable repository baselines and non-Git task retention."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from terminal_smoke import Session, terminal_environment
from workbench_smoke import wait_for


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ev")
    parser.add_argument("--output", default="target/repository-smoke")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == "nt" and not binary.exists():
        binary = binary.with_suffix(".exe")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "cases": []}
    sessions = []
    def launch(name, project, data, size):
        session = Session([str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"],
                          project, terminal_environment("mono"), size, output / name)
        sessions.append(session)
        session.pump(0.5)
        return session
    def quit_app(session):
        session.command("quit")
        if session.alive():
            session.send("\x1b[C\r")
        assert session.wait_exit() == 0
        session.close()
    def find_text(session, value):
        session.send("\x1b[H")
        for _ in range(50):
            if value in session.display():
                return
            session.send("\x1b[6~")
        raise AssertionError("missing repository evidence: " + value + "\n" + session.display())
    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)
    try:
        # Outside this checkout: a nested temporary directory would itself discover our Git root.
        with tempfile.TemporaryDirectory(prefix="ev-repo-空 格-") as temporary:
            root = Path(temporary).resolve()
            for width in (120, 60):
                project = root / ("repo 漢字 " + str(width))
                project.mkdir()
                def git(*arguments):
                    subprocess.run(["git", "-c", "core.autocrlf=false", "-c", "core.hooksPath=", *arguments],
                                   cwd=project, check=True, capture_output=True, timeout=10)
                git("init", "--quiet")
                tracked = project / "tracked 漢字.txt"
                tracked.write_bytes(b"committed baseline\r\n")
                git("add", "--", tracked.name)
                git("-c", "user.name=Eevee smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "fixture")
                tracked.write_bytes(b"existing_user_edit\r\n")
                untracked = project / "new 界.txt"
                untracked.write_bytes(b"existing_untracked_bytes\n")
                index_before = hashlib.sha256((project / ".git" / "index").read_bytes()).hexdigest()
                session = launch("repo-" + str(width), project, root / ("data-" + str(width)), (width,36 if width == 120 else 18))
                wait_for(session, "Storage: saved")
                session.send("\x1b")  # Leave the clean first-run Goal composer.
                session.command("goal")
                session.paste("repository task 漢字")
                session.send("\r")
                wait_for(session, "Storage: saved")
                session.command("diff")
                wait_for(session, "Repository: baseline captured", timeout=35)
                find_text(session, "pre-existing")
                session.capture("dirty-baseline")
                assert tracked.read_bytes() == b"existing_user_edit\r\n"
                assert untracked.read_bytes() == b"existing_untracked_bytes\n"
                tracked.write_bytes(b"external_edit_marker\r\n")
                (project / "external.txt").write_bytes(b"external_new_file\n")
                session.command("refresh")
                wait_for(session, "Repository: refreshed", timeout=35)
                find_text(session, "unattributed")
                session.capture("unattributed-changes")
                find_text(session, "external_edit_marker")
                session.capture("raw-diff")
                assert tracked.read_bytes() == b"external_edit_marker\r\n"
                assert untracked.read_bytes() == b"existing_untracked_bytes\n"
                assert hashlib.sha256((project / ".git" / "index").read_bytes()).hexdigest() == index_before
                session.send("\x1b")
                assert "repository task 漢字" in session.display()
                wait_for(session, "Storage: saved")
                quit_app(session)
                passed("dirty-baseline-external-diff-and-byte-preservation-" + str(width))
            plain = root / "plain project"
            plain.mkdir()
            data = root / "plain-data"
            session = launch("non-git", plain, data, (60,18))
            wait_for(session, "Storage: saved")
            session.send("\x1b")  # Leave the clean first-run Goal composer.
            session.command("goal")
            session.paste("nonGit retained task")
            session.send("\r")
            wait_for(session, "Storage: saved")
            session.command("diff")
            wait_for(session, "Repository: unsupported", timeout=35)
            session.capture("unsupported-with-task-retained")
            session.send("\x1b")
            wait_for(session, "Storage: saved")
            quit_app(session)
            session = launch("non-git-resume", plain, data, (60,18))
            wait_for(session, "nonGit retained task")
            quit_app(session)
            passed("non-git-unsupported-retains-durable-task")
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        for session in sessions:
            session.close()
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
