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
            def launch(name, directory=data, cwd=project, size=(120,36), extra=()):
                session = Session([str(binary), "--data-dir", str(directory), "--images", "off", "--color", "mono", *extra],
                                  cwd, terminal_environment("mono"), size, output / name)
                sessions.append(session)
                session.pump(0.5)
                return session
            def quit_app(session):
                session.send("q")
                if session.alive():
                    session.send("\x1b[C\r")
                assert session.wait_exit() == 0
                session.close()
            export_counter = 0
            def inspect_store(directory):
                nonlocal export_counter
                export_counter += 1
                destination = root / ("inspect-" + str(export_counter) + ".json")
                command = [str(binary), "--data-dir", str(directory), "--export-data", str(destination)]
                process = subprocess.run(command, capture_output=True, text=True, timeout=10)
                assert process.returncode == 0, process.stderr
                return json.loads(destination.read_text())
            def stored_when(directory, predicate):
                deadline = time.monotonic() + 8
                while True:
                    rows = inspect_store(directory)
                    if predicate(rows):
                        return rows
                    assert time.monotonic() < deadline, "stored state did not settle"
                    time.sleep(0.05)
            goal = "durable 漢字 cafe\u0301\nsecond line"
            first = launch("save")
            wait_for(first, "Storage: saved")
            first.send("g")
            first.paste(goal)
            first.send("\r")
            first.send("f9\r")
            wait_for(first, "Storage: saved")
            first.capture("committed")
            stored = inspect_store(data)
            assert len(stored) == 1 and (stored[0]["goal"],stored[0]["form"]) == (goal,8)
            project_identity = stored[0]["project"]
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
                assert db.execute("PRAGMA user_version").fetchone()[0] == 3
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
            for width in (120,60):
                cards = root / ("cards-" + str(width))
                size = (width,36 if width == 120 else 18)
                card = launch("cards-" + str(width), cards, size=size)
                wait_for(card, "Storage: saved")
                card.send("g")
                card.paste("card one 漢字")
                card.send("\r")
                wait_for(card, "Storage: saved")
                card.send("ce")
                card.paste("criterion alpha\ncriterion beta")
                card.send("\r\x1b")
                wait_for(card, "Storage: saved")
                card.send("a")
                card.paste("inspect first source")
                card.send("\r")
                card.send("f2\r")
                wait_for(card, "Storage: saved")
                card.send("g")
                card.paste(" pending one")
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.send("n")
                card.paste("card two 界")
                card.send("\r")
                wait_for(card, "Storage: saved")
                card.send("ce")
                card.paste("criterion gamma")
                card.send("\r\x1b")
                card.send("a")
                card.paste("inspect second source")
                card.send("\r")
                card.send("f3\r")
                wait_for(card, "Storage: saved")
                card.send("g")
                card.paste(" pending two")
                card.send("\x13")
                wait_for(card, "Storage: saved")
                rows = inspect_store(cards)
                assert len(rows) == 2
                one,two = sorted(rows,key=lambda task:task["id"])
                assert one["goal"] == "card one 漢字" and one["form"] == 1
                assert two["goal"] == "card two 界" and two["form"] == 2
                assert one["next_action"] == "inspect first source"
                assert two["next_action"] == "inspect second source"
                assert [c["text"] for c in one["criteria"]] == ["criterion alpha","criterion beta"]
                assert [c["text"] for c in two["criteria"]] == ["criterion gamma"]
                assert one["drafts"][0]["text"].endswith("pending one")
                assert two["drafts"][0]["text"].endswith("pending two")
                card.send("t\x1b[H\r")
                wait_for(card, "card one 漢字")
                wait_for(card, "Storage: saved")
                card.send("g")
                assert "pending one" in card.display() and "pending two" not in card.display()
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.send("\tl")
                wait_for(card, "Storage: saved")
                remembered = next(t for t in inspect_store(cards) if t["id"] == one["id"])
                assert remembered["navigation"]["linear"]
                assert remembered["navigation"]["focus"] == (2 if width == 120 else 1)
                card.capture("first-resumed")
                quit_app(card)
                card = launch("cards-restart-" + str(width), cards, size=size)
                wait_for(card, "card one 漢字")
                card.send("g")
                assert "pending one" in card.display()
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.send("t\x1b[F\r")
                wait_for(card, "card two 界")
                wait_for(card, "Storage: saved")
                card.send("g")
                assert "pending two" in card.display() and "pending one" not in card.display()
                card.send("\x1b")
                card.send("\x1b[C\r")
                wait_for(card, "Storage: saved")
                card.send("d")
                assert not next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.send("c ")
                card.send("\x1b")
                wait_for(card, "Storage: saved")
                card.send("d\r")
                assert not next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.send("d\x1b[C\r")
                wait_for(card, "Storage: saved")
                assert next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.send("g\r")
                assert next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.send("g\x1b[C\r")
                card.paste(" revised")
                card.send("\r")
                wait_for(card, "Storage: saved")
                current = next(t for t in inspect_store(cards) if t["id"] == two["id"])
                assert not current["complete"] and current["goal"] == "card two 界 revised"
                card.capture("reopened")
                quit_app(card)
                passed("two-task-draft-resume-complete-reopen-" + str(width))
            lifecycle = root / "lifecycle"
            secret = "ghp_" + "Ab9" * 12
            untouched = project / "user-owned.txt"
            untouched.write_bytes(b"user bytes\r\nnot managed by ev\n")
            life = launch("lifecycle", lifecycle, size=(60,18), extra=("--retention-days","0"))
            wait_for(life, "Storage: saved")
            life.send("g")
            life.paste("private 漢字 api_key=" + secret)
            life.send("\r")
            wait_for(life, "Storage: saved")
            first_rows = inspect_store(lifecycle)
            private_id = first_rows[0]["id"]
            assert secret not in json.dumps(first_rows) and first_rows[0]["redacted"]
            assert secret.encode() not in (lifecycle / "workbench.sqlite3").read_bytes()
            life.send("n")
            life.paste("retained second task")
            life.send("\r")
            wait_for(life, "Storage: saved")
            life.send("g")
            life.paste(" password=" + secret)
            life.send("\x13")
            wait_for(life, "Storage: saved")
            rows = inspect_store(lifecycle)
            other_id = next(row["id"] for row in rows if row["id"] != private_id)
            assert secret not in json.dumps(rows)
            assert next(row for row in rows if row["id"] == other_id)["drafts"]
            export_copy = root / "user-export.json"
            export_copy.write_text(json.dumps(rows))
            export_before = export_copy.read_bytes()
            life.send("t\x1b[H")
            life.capture("task-cards")
            life.send("a")
            stored_when(lifecycle, lambda rows: next(row for row in rows if row["id"] == private_id)["archived"])
            life.send("h")
            life.capture("archived")
            life.send("r")
            stored_when(lifecycle, lambda rows: not next(row for row in rows if row["id"] == private_id)["archived"])
            life.send("h\x1b[H\x1b[3~")
            wait_for(life, "Review local task deletion")
            life.capture("neutral-delete-preview")
            life.send("\r")
            assert len(inspect_store(lifecycle)) == 2, "neutral confirmation deleted data"
            life.send("\x1b[3~")
            life.send("\x1b[C\r")
            rows = stored_when(lifecycle, lambda rows: len(rows) == 1)
            assert rows[0]["id"] == other_id and rows[0]["drafts"]
            assert export_copy.read_bytes() == export_before
            assert untouched.read_bytes() == b"user bytes\r\nnot managed by ev\n"
            life.send("a")
            stored_when(lifecycle, lambda rows: rows[0]["archived"])
            quit_app(life)
            life = launch("lifecycle-restart", lifecycle, size=(60,18), extra=("--retention-days","0"))
            wait_for(life, "Storage: saved")
            assert "retained second task" not in life.display()
            life.send("th")
            wait_for(life, "retained second task")
            life.send("x")
            wait_for(life, "Review local task deletion")
            life.send("\x1b[C\r")
            stored_when(lifecycle, lambda rows: rows == [])
            assert export_copy.read_bytes() == export_before and untouched.exists()
            quit_app(life)
            passed("archive-restore-neutral-delete-retention-redaction-and-scope")

            quota_data = root / "quota"
            quota = launch("quota", quota_data, extra=("--quota-kib","64"))
            wait_for(quota, "Storage: saved")
            failed_goal = None
            for index in range(16):
                quota.send("g" if index == 0 else "n")
                goal = "quota card " + str(index) + " 漢字 " + "x" * 3960
                quota.paste(goal)
                quota.send("\r")
                deadline = time.monotonic() + 8
                while "Storage: saved" not in quota.display() and "Storage: save failed" not in quota.display():
                    assert time.monotonic() < deadline, quota.display()
                    quota.pump(0.05)
                if "Storage: save failed" in quota.display():
                    failed_goal = goal
                    break
            assert failed_goal is not None, "configured task quota did not reject real data"
            before = inspect_store(quota_data)
            assert len(before) >= 2 and all(row["goal"] != failed_goal for row in before)
            quota.send("g")
            quota.paste(" retained")
            quota.send("\x13")
            quota.send("i")
            quota.capture("quota-failure")
            quota.send("\x1b")
            quota.send("t\x1b[H")
            for remaining in range(len(before)-1, 0, -1):
                quota.send("\x1b[3~")
                wait_for(quota, "Review local task deletion")
                quota.send("\x1b[C\r")
                stored_when(quota_data, lambda rows: len(rows) == remaining)
            quota.send("r")
            wait_for(quota, "Storage: saved")
            rows = inspect_store(quota_data)
            recovered = next(row for row in rows if row["goal"] == failed_goal)
            assert recovered["drafts"][0]["text"] == failed_goal + " retained"
            assert {row["id"] for row in rows} == {before[-1]["id"], recovered["id"]}
            quota.send("\x1b")
            quota.send("g")
            assert "retained" in quota.display()
            quota.send("\x13")
            wait_for(quota, "Storage: saved")
            quit_app(quota)
            passed("real-task-quota-retains-draft-and-explicit-delete-recovers")
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        for session in sessions: session.close()
        (output / "results.json").write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n")


if __name__ == "__main__":
    main()
