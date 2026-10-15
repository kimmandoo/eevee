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

from terminal_smoke import Session, confirm_exit, terminal_environment


def lead_drafts(task):
    return task["partner_drafts"].get(str(task["lead_partner"]), {}).get("drafts", [])


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
            def launch(name, directory=data, cwd=project, size=(120,36), extra=(), environment=None):
                session = Session([str(binary), "--data-dir", str(directory), "--images", "off", "--color", "mono", *extra],
                                  cwd, terminal_environment("mono") if environment is None else environment, size, output / name)
                sessions.append(session)
                session.pump(0.5)
                return session
            def quit_app(session):
                session.command("quit")
                confirm_exit(session)
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
            # Local-only fixtures explicitly choose task facts instead of submitting a request.
            empty_path = root / "no-provider-programs"
            empty_path.mkdir()
            isolated_home = root / "first-use-home"
            isolated_home.mkdir()
            first_use_environment = terminal_environment("mono")
            for key in list(first_use_environment):
                if key.startswith(("OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GEMINI_", "GOOGLE_", "AWS_", "OPENCODE_")):
                    first_use_environment.pop(key, None)
            first_use_environment.update(
                PATH=str(empty_path), HOME=str(isolated_home), USERPROFILE=str(isolated_home),
                XDG_CONFIG_HOME=str(isolated_home / "config"), XDG_DATA_HOME=str(isolated_home / "data"),
                XDG_CACHE_HOME=str(isolated_home / "cache"), APPDATA=str(isolated_home / "config"),
                LOCALAPPDATA=str(isolated_home / "cache"),
            )
            for width in (120, 60):
                first_use_data = root / ("first-use-" + str(width))
                assets = root / ("first-use-artwork-" + str(width))
                size = (width, 36 if width == 120 else 18)
                first_use_args = dict(directory=first_use_data, size=size,
                                      extra=("--assets-dir", str(assets), "--images", "cells"), environment=first_use_environment)
                fresh = launch("first-use-" + str(width), **first_use_args)
                wait_for(fresh, "Storage: saved")
                fresh.command("goal")
                assert not fresh.screen.cursor.hidden, "first-run task input has no visible caret"
                fresh.capture("visible-first-run-input")
                typed = "qgo? direct typing "
                pasted = "漢字\nlocal-only pasted goal"
                fresh.send(typed)
                fresh.paste(pasted)
                assert "qgo? direct typing" in fresh.display()
                assert "local-only pasted goal" in fresh.display()
                assert fresh.alive(), "typing a navigation letter executed a shortcut"
                fresh.capture("direct-typing-and-paste")
                fresh.send("\r")
                saved_goal = typed + pasted
                stored_when(first_use_data, lambda rows: len(rows) == 1 and rows[0]["goal"] == saved_goal)
                wait_for(fresh, "Storage: saved")
                quit_app(fresh)
                fresh = launch("first-use-restart-" + str(width), **first_use_args)
                fresh.command("goal")
                wait_for(fresh, "qgo? direct typing")
                assert inspect_store(first_use_data)[0]["goal"] == saved_goal
                fresh.capture("local-apply-restart")
                passed("first-run-visible-input-direct-type-paste-local-apply-restart-" + str(width))

                fresh.command("goal")
                fresh.paste(" retained shortcut draft")
                fresh.command("connect")
                wait_for(fresh, "Connections")
                wait_for(fresh, "Not installed")
                fresh.capture("command-provider-list")
                fresh.send("\r")  # Provider -> visible actions.
                wait_for(fresh, "Connect / credential setup")
                fresh.capture("command-provider-actions")
                fresh.send("\r")  # Missing provider: setup guidance, never auth.
                wait_for(fresh, "npm install")
                assert "Review connection" not in fresh.display()
                fresh.capture("command-missing-provider-guidance")
                fresh.send("\x1b")
                wait_for(fresh, "PROVIDERS")
                fresh.send("\x1b")
                wait_for(fresh, "Storage: saved")
                fresh.command("goal")
                assert "retained shortcut draft" in fresh.display(), "/connect discarded the task draft"
                fresh.send("\x13")
                wait_for(fresh, "Storage: saved")
                retained = inspect_store(first_use_data)[0]
                assert retained["goal"] == saved_goal
                assert lead_drafts(retained)[0]["text"] == saved_goal + " retained shortcut draft"
                passed("command-discoverable-provider-actions-back-preserves-draft-without-auth-" + str(width))
                for command, label in (("help", "command-help"),
                                       (None, "ctrl-k-actions")):
                    fresh.command("goal")
                    editor_view = fresh.display()
                    fresh.command(command) if command else fresh.send("\x0b")
                    assert fresh.display() != editor_view, label + " did not open"
                    fresh.capture(label)
                    fresh.send("\x1b")
                    fresh.command("goal")
                    assert "retained shortcut draft" in fresh.display(), label + " discarded the draft"
                    fresh.send("\x13")
                    wait_for(fresh, "Storage: saved")
                    assert lead_drafts(inspect_store(first_use_data)[0]) == lead_drafts(retained)
                passed("global-help-and-actions-preserve-inline-draft-" + str(width))

                fresh.command("forms")
                wait_for(fresh, "Choose evolution")
                fresh.send("\x1b[H" + "\x1b[C" * 8)
                wait_for(fresh, "Sylveon")
                fresh.capture("command-evolution-preview")
                fresh.send("\x1b")
                assert inspect_store(first_use_data)[0]["form"] == 0, "preview/Back applied an evolution"
                fresh.command("forms")
                fresh.command("role-sylveon")
                stored_when(first_use_data, lambda rows: rows[0]["form"] == 8)
                assert lead_drafts(inspect_store(first_use_data)[0]) == lead_drafts(retained)
                passed("command-evolution-preview-back-and-explicit-select-" + str(width))
                fresh.command("goal")
                fresh.command("images")  # Retain the draft and open only a review.
                wait_for(fresh, "Images / Eevee and evolutions")
                wait_for(fresh, "0/9")
                wait_for(fresh, "> Back")
                wait_for(fresh, "Download missing / Retry")
                wait_for(fresh, "Reload local")
                fresh.capture("command-image-review-neutral")
                fresh.send("\x1b[F")
                wait_for(fresh, "Enter Select")
                fresh.send("\r")  # Even a fully read review defaults to Back.
                wait_for(fresh, "Storage: saved")
                assert not assets.exists(), "neutral image review downloaded artwork"
                fresh.command("images")
                wait_for(fresh, "> Back")
                fresh.send("\x1b[F\t")
                wait_for(fresh, "> Download missing / Retry")
                fresh.capture("command-download-selected-not-approved")
                fresh.send("\x1b")  # Highlighting Download is not consent.
                wait_for(fresh, "Storage: saved")
                assert "Downloading pinned artwork" not in fresh.text, "neutral image navigation launched a download"
                assert not assets.exists(), "Esc from image review downloaded artwork"
                fresh.command("goal")
                assert "retained shortcut draft" in fresh.display(), "/images discarded the task draft"
                fresh.send("\x13")
                wait_for(fresh, "Storage: saved")
                assert lead_drafts(inspect_store(first_use_data)[0]) == lead_drafts(retained)
                passed("command-image-review-neutral-enter-and-escape-no-download-" + str(width))
                with closing(sqlite3.connect((first_use_data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                    assert db.execute("SELECT count(*) FROM provider_journal").fetchone()[0] == 0
                    assert db.execute("SELECT count(*) FROM executions").fetchone()[0] == 0
                assert not assets.exists(), "first-use navigation created an unapproved artwork cache"
                assert not any(path.is_file() for path in isolated_home.rglob("*")), "navigation wrote provider-owned configuration or credentials"
                quit_app(fresh)

            goal = "durable 漢字 cafe\u0301\nsecond line"
            first = launch("save")
            wait_for(first, "Storage: saved")
            first.send("\x1b")  # Legacy navigation starts outside the clean composer.
            first.command("goal")
            first.paste(goal)
            first.send("\r")
            first.command("forms")
            first.command("role-sylveon")
            wait_for(first, "Storage: saved")
            first.capture("committed")
            stored = inspect_store(data)
            assert len(stored) == 1 and (stored[0]["goal"],stored[0]["form"]) == (goal,8)
            project_identity = stored[0]["project"]
            quit_app(first)
            second = launch("resume")
            second.command("goal")
            wait_for(second, "durable 漢字 café")
            assert "Sylveon" in second.display()
            passed("exact-goal-form-save-exit-resume")
            other = root / "other project"
            other.mkdir()
            conflict = launch("writer-conflict", cwd=other)
            wait_for(conflict, "Storage: save failed")
            conflict.send("\x1b")
            conflict.command("goal")
            conflict.paste("retained writer conflict")
            conflict.send("\r")
            assert "retained writer conflict" in conflict.display()
            conflict.command("quit")
            conflict.send("\r")
            assert conflict.alive(), "neutral exit discarded a failed-save goal"
            quit_app(second)
            conflict.command("storage-retry")
            wait_for(conflict, "Storage: saved")
            quit_app(conflict)
            resumed = launch("other-project", cwd=other)
            resumed.command("goal")
            wait_for(resumed, "retained writer conflict")
            assert "durable 漢字" not in resumed.display()
            quit_app(resumed)
            passed("writer-conflict-retry-and-project-isolation")
            locked = launch("locked-sqlite")
            wait_for(locked, "Storage: saved")
            lock = sqlite3.connect(data / "workbench.sqlite3", timeout=1)
            lock.execute("BEGIN EXCLUSIVE")
            try:
                locked.command("goal")
                locked.paste(" pending locked edit")
                locked.send("\r")
                wait_for(locked, "Storage: save failed")
                assert "pending locked edit" in locked.display()
                locked.capture("save-failure")
            finally:
                lock.rollback()
                lock.close()
            locked.command("storage-retry")
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
                failed.send("\x1b")
                failed.command("goal")
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
            migrated.command("goal")
            wait_for(migrated, "migration retained")
            wait_for(migrated, "Storage: saved")
            quit_app(migrated)
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
            pending.send("\x1b")
            pending.command("goal")
            pending.paste("save before exit")
            pending.send("\r")
            pending.command("quit")
            confirm_exit(pending)
            assert pending.wait_exit() == 0
            pending.close()
            reopened = launch("quit-after-apply-resume", root / "pending")
            reopened.command("goal")
            wait_for(reopened, "save before exit")
            quit_app(reopened)
            passed("quit-after-apply-retains-committed-goal")
            for width in (120,60):
                cards = root / ("cards-" + str(width))
                size = (width,36 if width == 120 else 18)
                card = launch("cards-" + str(width), cards, size=size)
                wait_for(card, "Storage: saved")
                card.send("\x1b")
                card.command("goal")
                card.paste("card one 漢字")
                card.send("\r")
                wait_for(card, "Storage: saved")
                card.command("criteria")
                card.command("criteria-edit")
                card.paste("criterion alpha\ncriterion beta")
                card.send("\r\x1b")
                wait_for(card, "Storage: saved")
                card.command("next")
                card.paste("inspect first source")
                card.send("\r")
                card.command("forms")
                card.command("role-vaporeon")
                wait_for(card, "Storage: saved")
                card.command("goal")
                card.paste(" pending one")
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.command("task-new")
                card.command("goal")
                card.paste("card two 界")
                card.send("\r")
                wait_for(card, "Storage: saved")
                card.command("criteria")
                card.command("criteria-edit")
                card.paste("criterion gamma")
                card.send("\r\x1b")
                card.command("next")
                card.paste("inspect second source")
                card.send("\r")
                card.command("forms")
                card.command("role-jolteon")
                wait_for(card, "Storage: saved")
                card.command("goal")
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
                assert lead_drafts(one)[0]["text"].endswith("pending one")
                assert lead_drafts(two)[0]["text"].endswith("pending two")
                card.command("tasks")
                card.send("\x1b[H\r")
                card.command("goal")
                wait_for(card, "card one 漢字")
                wait_for(card, "Storage: saved")
                card.command("goal")
                assert "pending one" in card.display() and "pending two" not in card.display()
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.send("\t")
                card.command("layout")
                wait_for(card, "Storage: saved")
                remembered = next(t for t in inspect_store(cards) if t["id"] == one["id"])
                assert remembered["navigation"]["linear"]
                assert remembered["navigation"]["focus"] == (2 if width == 120 else 1)
                card.capture("first-resumed")
                quit_app(card)
                card = launch("cards-restart-" + str(width), cards, size=size)
                card.command("goal")
                wait_for(card, "card one 漢字")
                card.command("goal")
                assert "pending one" in card.display()
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.command("tasks")
                card.send("\x1b[F\r")
                card.command("goal")
                wait_for(card, "card two 界")
                wait_for(card, "Storage: saved")
                card.command("goal")
                assert "pending two" in card.display() and "pending one" not in card.display()
                card.send("\x1b")
                card.send("\x1b[C\r")
                wait_for(card, "Storage: saved")
                card.command("complete")
                assert not next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("criteria")
                card.command("criterion-toggle")
                card.send("\x1b")
                wait_for(card, "Storage: saved")
                card.command("complete")
                card.send("\r")
                assert not next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("complete")
                card.send("\x1b[C\r")
                wait_for(card, "Storage: saved")
                assert next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("goal")
                card.send("\r")
                assert next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("goal")
                card.send("\x1b[C\r")
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
            life.send("\x1b")
            life.command("goal")
            life.paste("private 漢字 api_key=" + secret)
            life.send("\r")
            wait_for(life, "Storage: saved")
            first_rows = inspect_store(lifecycle)
            private_id = first_rows[0]["id"]
            assert secret not in json.dumps(first_rows) and first_rows[0]["redacted"]
            assert secret.encode() not in (lifecycle / "workbench.sqlite3").read_bytes()
            life.command("task-new")
            life.command("goal")
            life.paste("retained second task")
            life.send("\r")
            wait_for(life, "Storage: saved")
            life.command("goal")
            life.paste(" password=" + secret)
            life.send("\x13")
            wait_for(life, "Storage: saved")
            rows = inspect_store(lifecycle)
            other_id = next(row["id"] for row in rows if row["id"] != private_id)
            assert secret not in json.dumps(rows)
            assert lead_drafts(next(row for row in rows if row["id"] == other_id))
            export_copy = root / "user-export.json"
            export_copy.write_text(json.dumps(rows))
            export_before = export_copy.read_bytes()
            life.command("tasks")
            life.send("\x1b[H")
            life.capture("task-cards")
            life.command("archive")
            stored_when(lifecycle, lambda rows: next(row for row in rows if row["id"] == private_id)["archived"])
            life.command("archived")
            life.capture("archived")
            life.command("restore")
            stored_when(lifecycle, lambda rows: not next(row for row in rows if row["id"] == private_id)["archived"])
            life.command("archived")
            life.send("\x1b[H")
            life.command("delete")
            wait_for(life, "Review local task deletion")
            life.capture("neutral-delete-preview")
            life.send("\r")
            assert len(inspect_store(lifecycle)) == 2, "neutral confirmation deleted data"
            life.command("delete")
            life.send("\x1b[C\r")
            rows = stored_when(lifecycle, lambda rows: len(rows) == 1)
            assert rows[0]["id"] == other_id and lead_drafts(rows[0])
            assert export_copy.read_bytes() == export_before
            assert untouched.read_bytes() == b"user bytes\r\nnot managed by ev\n"
            life.command("archive")
            stored_when(lifecycle, lambda rows: rows[0]["archived"])
            quit_app(life)
            life = launch("lifecycle-restart", lifecycle, size=(60,18), extra=("--retention-days","0"))
            wait_for(life, "Storage: saved")
            assert "retained second task" not in life.display()
            life.send("\x1b")  # All cards are archived, so this is a fresh composer.
            life.command("tasks")
            life.command("archived")
            wait_for(life, "retained second task")
            life.command("prune")
            wait_for(life, "Review local task deletion")
            life.send("\x1b[C\r")
            stored_when(lifecycle, lambda rows: rows == [])
            assert export_copy.read_bytes() == export_before and untouched.exists()
            quit_app(life)
            passed("archive-restore-neutral-delete-retention-redaction-and-scope")

            quota_data = root / "quota"
            seed = launch("quota-schema", quota_data)
            wait_for(seed, "Storage: saved")
            seed.send("\x1b")
            quit_app(seed)
            with closing(sqlite3.connect(quota_data / "workbench.sqlite3")) as db:
                schema_bytes = db.execute("PRAGMA page_count").fetchone()[0] * db.execute("PRAGMA page_size").fetchone()[0]
            # Leave a fixed payload budget above actual schema cost. A fixed
            # total conflates quota behavior with newly added schema pages and
            # can leave no room for the explicit recovery/navigation update.
            quota_kib = max(64, (schema_bytes + 1023) // 1024 + 32)
            result["quota_kib"] = quota_kib
            quota = launch("quota", quota_data, extra=("--quota-kib", str(quota_kib)))
            wait_for(quota, "Storage: saved")
            quota.send("\x1b")
            failed_goal = None
            for index in range(16):
                if index:
                    quota.command("task-new")
                quota.command("goal")
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
            quota.command("goal")
            quota.paste(" retained")
            quota.send("\x13")
            quota.command("inspect")
            quota.capture("quota-failure")
            quota.send("\x1b")
            quota.command("tasks")
            quota.send("\x1b[H")
            for remaining in range(len(before)-1, 0, -1):
                quota.command("delete")
                wait_for(quota, "Review local task deletion")
                quota.send("\x1b[C\r")
                stored_when(quota_data, lambda rows: len(rows) == remaining)
            quota.command("storage-retry")
            wait_for(quota, "Storage: saved")
            rows = inspect_store(quota_data)
            recovered = next(row for row in rows if row["goal"] == failed_goal)
            assert lead_drafts(recovered)[0]["text"] == failed_goal + " retained"
            assert {row["id"] for row in rows} == {before[-1]["id"], recovered["id"]}
            quota.send("\x1b")
            quota.command("goal")
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
