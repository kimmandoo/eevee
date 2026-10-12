#!/usr/bin/env python3
"""Native installed-TUI acceptance for durable local workbench behavior."""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
from contextlib import closing

from terminal_smoke import Session, terminal_environment


def wait_for(session, text, timeout=8):
    deadline = time.monotonic() + timeout
    while text not in session.display():
        assert session.alive(), "application exited while waiting for " + text
        assert time.monotonic() < deadline, "missing state: " + text + "\n" + session.display()
        session.pump(0.05)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ev")
    parser.add_argument("--output", default="target/workbench-smoke")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == "nt" and not binary.exists(): binary = binary.with_suffix(".exe")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "cases": []}
    sessions = []
    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="workbench-空 格-", dir=output) as temporary:
            root = Path(temporary).resolve()
            project = root / "project 漢字"
            project.mkdir()
            data = root / "data 空 格"
            def launch(name, directory=data, cwd=project):
                session = Session([str(binary), "--data-dir", str(directory), "--images", "off", "--color", "mono"],
                                  cwd, terminal_environment("mono"), (120,36), output / name)
                sessions.append(session)
                session.pump(0.5)
                return session
            def quit_app(session):
                session.send("q")
                if session.alive():
                    session.send("\x1b[C\r")
                assert session.wait_exit() == 0
                session.close()
            goal = "durable 漢字 cafe\u0301\nsecond line"
            first = launch("save")
            wait_for(first, "Storage: saved")
            first.send("g")
            first.paste(goal)
            first.send("\r")
            first.send("f9\r")
            wait_for(first, "Storage: saved")
            first.capture("committed")
            with closing(sqlite3.connect(data / "workbench.sqlite3")) as db:
                assert db.execute("SELECT goal,form FROM projects").fetchone() == (goal,8)
                project_identity = db.execute("SELECT project FROM projects").fetchone()[0]
            quit_app(first)
            second = launch("resume")
            wait_for(second, "durable 漢字 café")
            assert "Sylveon" in second.display()
            passed("exact-goal-form-save-exit-resume")
            other = root / "other project"
            other.mkdir()
            conflict = launch("writer-conflict", cwd=other)
            wait_for(conflict, "Storage: save failed")
            conflict.send("g")
            conflict.paste("retained writer conflict")
            conflict.send("\r")
            assert "retained writer conflict" in conflict.display()
            conflict.send("q\r")
            assert conflict.alive(), "neutral exit discarded a failed-save goal"
            quit_app(second)
            conflict.send("r")
            wait_for(conflict, "Storage: saved")
            quit_app(conflict)
            resumed = launch("other-project", cwd=other)
            wait_for(resumed, "retained writer conflict")
            assert "durable 漢字" not in resumed.display()
            quit_app(resumed)
            passed("writer-conflict-retry-and-project-isolation")
            locked = launch("locked-sqlite")
            wait_for(locked, "Storage: saved")
            lock = sqlite3.connect(data / "workbench.sqlite3", timeout=1)
            lock.execute("BEGIN EXCLUSIVE")
            try:
                locked.send("g")
                locked.paste(" pending locked edit")
                locked.send("\r")
                wait_for(locked, "Storage: save failed")
                assert "pending locked edit" in locked.display()
                locked.capture("save-failure")
            finally:
                lock.rollback()
                lock.close()
            locked.send("r")
            wait_for(locked, "Storage: saved")
            quit_app(locked)
            passed("sqlite-lock-failure-retains-edit-and-retries")
            for kind in ("corrupt", "newer"):
                folder = root / kind
                folder.mkdir()
                path = folder / "workbench.sqlite3"
                if kind == "corrupt": path.write_bytes(b"not a sqlite database\x00preserve me")
                else:
                    with closing(sqlite3.connect(path)) as db:
                        db.execute("CREATE TABLE future(value TEXT)")
                        db.execute("INSERT INTO future VALUES ('keep')")
                        db.execute("PRAGMA user_version=999")
                        db.commit()
                before = path.read_bytes()
                failed = launch(kind, folder)
                wait_for(failed, "Storage: save failed")
                failed.send("g")
                failed.paste("recoverable draft")
                failed.send("\r")
                quit_app(failed)
                assert path.read_bytes() == before
                passed(kind + "-database-untouched")
            legacy = root / "legacy"
            legacy.mkdir()
            with closing(sqlite3.connect(legacy / "workbench.sqlite3")) as db:
                db.execute("CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL)")
                db.execute("INSERT INTO goals VALUES (?,?,?)", (project_identity,"migration retained",1))
                db.commit()
            migrated = launch("migration", legacy)
            wait_for(migrated, "migration retained")
            wait_for(migrated, "Storage: saved")
            quit_app(migrated)
            with closing(sqlite3.connect(legacy / "workbench.sqlite3")) as db:
                assert db.execute("PRAGMA user_version").fetchone()[0] == 1
            exported = root / "recovery.json"
            command = [str(binary), "--data-dir",str(legacy),"--export-data",str(exported)]
            export = subprocess.run(command, capture_output=True, text=True, timeout=10)
            assert export.returncode == 0, export.stderr
            assert "migration retained" in exported.read_text()
            before = exported.read_bytes()
            assert subprocess.run(command, capture_output=True, timeout=10).returncode != 0
            assert exported.read_bytes() == before
            passed("transactional-migration-and-nondestructive-export")
            pending = launch("quit-after-apply", root / "pending")
            wait_for(pending, "Storage: saved")
            pending.send("g")
            pending.paste("save before exit")
            pending.send("\rq\x1b[C\r")
            assert pending.wait_exit() == 0
            pending.close()
            reopened = launch("quit-after-apply-resume", root / "pending")
            wait_for(reopened, "save before exit")
            quit_app(reopened)
            passed("quit-after-apply-retains-committed-goal")
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        for session in sessions: session.close()
        (output / "results.json").write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n")


if __name__ == "__main__":
    main()
