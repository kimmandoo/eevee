#!/usr/bin/env python3
"""Actual PTY/ConPTY source-basket acceptance; no provider requests are sent."""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for


ESC = "\x1b"
HOME = ESC + "[H"
DOWN = ESC + "[B"
RIGHT = ESC + "[C"
DELETE = ESC + "[3~"
SOURCE_TITLE = "Sources / task-owned context"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ev")
    parser.add_argument("--output", default="target/context-smoke")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == "nt" and not binary.exists():
        binary = binary.with_suffix(".exe")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "cases": [], "host": platform.platform(),
              "scope": "Native terminal source/context behavior only; no provider send or filesystem sandbox claim"}
    sessions = []

    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)

    try:
        # A fixture under target/ would discover the enclosing checkout's Git root.
        with tempfile.TemporaryDirectory(prefix="ev-context-空 格-") as temporary:
            root = Path(temporary).resolve()
            project = root / "project 漢字"
            project.mkdir()
            data = root / "data 空 格"
            database = data / "workbench.sqlite3"

            def git(*arguments):
                subprocess.run(["git", "-c", "core.autocrlf=false", "-c", "core.hooksPath=", *arguments],
                               cwd=project, check=True, capture_output=True, timeout=10)

            def link(path, target):
                try:
                    path.symlink_to(target)
                except OSError as error:
                    result["blocked"] = "Native file-link creation is required for this acceptance row: " + str(error)
                    raise AssertionError(result["blocked"]) from error

            git("init", "--quiet")
            secret = "context-opaque-value-123"
            original = ("first\r\npassword=" + secret + "\r\n東京 source\r\nlast\r\n").encode("utf-8")
            selected = "password=[REDACTED]\r\n東京 source\r\n"
            source = project / "資料.rs"
            source.write_bytes(original)
            (project / "other.rs").write_bytes(b"independent removable capture\n")
            replacement = project / "replace.rs"
            replacement_bytes = b"identical bytes do not authorize a link\n"
            replacement.write_bytes(replacement_bytes)
            missing = project / "missing.rs"
            missing_bytes = b"missing source has no substituted evidence\n"
            missing.write_bytes(missing_bytes)
            (project / ".gitignore").write_text("ignored.rs\n", encoding="utf-8")
            (project / "ignored.rs").write_bytes(b"ignored source must not enter the basket\n")
            (project / ".env.local").write_text("password=excluded-credential\n", encoding="utf-8")
            (project / "opaque.bin").write_bytes(b"binary\x00not context\n")
            (project / "excluded").mkdir()
            (project / "excluded" / "never.rs").write_bytes(b"configured exclusion\n")
            (project / "keys").mkdir()
            (project / "keys" / "signing.pem").write_bytes(b"excluded private key path\n")
            outside = root / "outside.txt"
            outside.write_bytes(replacement_bytes)
            link(project / "outside.rs", outside)
            git("add", "--", "資料.rs", "other.rs", "replace.rs", "missing.rs", ".gitignore")
            git("-c", "user.name=Eevee smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "source fixture")
            index_before = hashlib.sha256((project / ".git" / "index").read_bytes()).hexdigest()

            def launch(name, size=(120, 36)):
                session = Session([str(binary), "--continue", "--data-dir", str(data), "--images", "off", "--color", "mono",
                                   "--exclude-context", "EXCLUDED", "--exclude-context", "unused-prefix"], project, terminal_environment("mono"), size, output / name)
                sessions.append(session)
                session.context_query = ""
                session.context_mode = "closed"
                session.pump(0.5)
                wait_for(session, "Storage: saved")
                return session

            def read_rows():
                # Export is task-row-only. Read actual durable basket records without
                # acquiring a second application writer or copying source payloads.
                with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=10)) as db:
                    tasks = db.execute("SELECT id,goal FROM tasks ORDER BY id").fetchall()
                    records = [json.loads(row[0]) for row in db.execute("SELECT record FROM excerpts ORDER BY id")]
                    active = db.execute("SELECT active_id FROM selections").fetchone()
                return tasks, records, None if active is None else active[0]

            def basket(task_id):
                return [row for row in read_rows()[1] if row["task_id"] == task_id]

            def settled(session, predicate, explanation):
                deadline = time.monotonic() + 10
                while True:
                    rows = read_rows()
                    if predicate(rows):
                        return rows
                    assert session.alive(), "application exited while waiting for " + explanation
                    assert time.monotonic() < deadline, explanation + "\n" + session.display()
                    session.pump(0.05)

            def create_task(session, goal, first=False):
                assert session.context_mode == "closed"
                if not first:
                    session.command("task-new")
                session.command("goal")
                session.paste(goal)
                session.send("\r")
                rows = settled(session, lambda rows: any(text == goal for _, text in rows[0]), "task was not committed")
                wait_for(session, "Storage: saved")
                session.context_query = ""
                return next(task_id for task_id, text in rows[0] if text == goal)

            def switch_task(session, index, task_id, goal):
                close_sources(session)
                session.command("tasks")
                wait_for(session, "Saved sessions / committed tasks")
                wait_for(session, "Storage: saved")
                session.send(HOME + DOWN * index)
                wait_for(session, "Storage: saved")
                session.send("\r")
                settled(session, lambda rows: rows[2] == task_id, "task selection did not commit")
                session.command("goal")
                wait_for(session, goal)
                wait_for(session, "Storage: saved")
                session.send("\x13")
                session.context_query = ""

            def open_sources(session):
                assert session.context_mode == "closed"
                session.command("files")
                wait_for(session, SOURCE_TITLE)
                wait_for(session, "eligible files /", timeout=35)
                session.context_mode = "files"

            def close_sources(session):
                if session.context_mode != "closed":
                    session.command("close")
                    session.context_mode = "closed"
                wait_for(session, "Storage: saved")

            def quit_app(session):
                close_sources(session)
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()

            def files(session):
                if session.context_mode in ("payload", "preview", "basket"):
                    session.send("\t")
                    session.context_mode = "files"
                assert session.context_mode == "files"

            def search(session, value, count):
                files(session)
                session.send("s")
                wait_for(session, "Filename filter")
                session.send(HOME + DELETE * len(session.context_query))
                session.paste(value)
                session.send("\r")
                session.context_query = value
                wait_for(session, str(count) + " eligible files /", timeout=35)

            def pin(session, task_id, filename, lines=None):
                before = len(basket(task_id))
                search(session, filename, 1)
                session.send("\r")
                wait_for(session, "Review local capture /")
                session.context_mode = "preview"
                if lines is not None:
                    session.send("r")
                    wait_for(session, "Source range / inclusive")
                    session.send(HOME + DELETE * 3)
                    session.paste(lines)
                    session.send("\r")
                    wait_for(session, "Lines " + lines + " inclusive")
                session.send("\r")
                settled(session, lambda rows: sum(row["task_id"] == task_id for row in rows[1]) == before + 1,
                        "attachment was not committed")
                session.pump(0.1)
                if SOURCE_TITLE not in session.display():
                    session.context_mode = "closed"
                    open_sources(session)
                    session.send("\t")
                session.context_mode = "basket"
                return basket(task_id)[-1]

            def show_basket(session, task_id):
                files(session)
                session.send("\t")
                session.send(HOME)
                session.context_mode = "basket"
                rows = basket(task_id)
                if rows:
                    wait_for(session, "#" + str(rows[0]["id"]) + (" stored" if rows[0]["available"] else " UNAVAILABLE"))
                else:
                    wait_for(session, "No saved attachments")

            def panel_text(session):
                # Read actual terminal cells inside the source overlay. Do not mix
                # underlying rail text into wrapped payload lines or count wide CJK
                # glyphs as two Python characters.
                screen = session.screen
                for y in range(screen.lines):
                    cells = screen.buffer[y]
                    for x in range(screen.columns - len(SOURCE_TITLE) + 1):
                        if "".join(cells[column].data for column in range(x, x + len(SOURCE_TITLE))) != SOURCE_TITLE:
                            continue
                        # Frame style is presentation, not the payload contract.
                        left = max(column for column in range(x) if cells[column].data in ("╭", "┌", "╔", "+"))
                        right = min(column for column in range(x + len(SOURCE_TITLE), screen.columns) if cells[column].data in ("╮", "┐", "╗", "+"))
                        bottom = next(row for row in range(y + 1, screen.lines)
                                      if screen.buffer[row][left].data in ("╰", "└", "╚", "+")
                                      and screen.buffer[row][right].data in ("╯", "┘", "╝", "+"))
                        return "\n".join("".join(screen.buffer[row][column].data for column in range(left + 1, right))
                                         for row in range(y + 1, bottom))
                raise AssertionError("source overlay cell boundaries are unavailable\n" + session.display())

            def exact_preview(session, task_id, dispatch=False):
                session.command("dispatch-preview" if dispatch else "assemble")
                wait_for(session, "Dispatch boundary revalidated / no provider send" if dispatch
                         else "Source and stored references validated / preview only", timeout=35)
                session.context_mode = "payload"
                session.send(HOME)
                records = basket(task_id)
                # This fixture uses distinct source paths; overlapping capture
                # representation is covered by the context admission regressions.
                assert len({record["path"] for record in records}) == len(records)
                assert all(record["root"] == records[0]["root"] for record in records)
                payload = json.dumps({
                    "source_root": records[0]["root"] if records else "",
                    "captures": [{
                        **{key: record[key] for key in ("id", "path", "start_line", "end_line", "text")},
                        "source_sha256": bytes(record["source_sha256"]).hex(),
                    } for record in records],
                }, ensure_ascii=False, separators=(",", ":"))
                count = len(payload.encode("utf-8"))
                wait_for(session, str(count) + " UTF-8 bytes /")
                text = panel_text(session)
                # Compare the complete visible envelope semantically, not JSON
                # key order. Terminal wrapping loses whitespace boundaries; raw
                # CRLF/spacing is independently checked against committed records.
                compact = lambda value: "".join(value.split())
                visible = compact(text)
                escaped = visible[visible.index('"{'):visible.rindex('}"') + 2]
                observed = json.loads(json.loads(escaped))
                assert observed == json.loads(compact(payload)), "outgoing envelope differed\n" + text
                assert secret not in payload and secret not in session.text
                session.capture("dispatch-checked" if dispatch else "exact-outgoing-preview")
                return payload

            def fail_dispatch(session, layer="source"):
                session.command("dispatch-preview")
                wait_for(session, "Context storage/validation failed" if layer == "storage"
                         else "Context unavailable or stale", timeout=35)
                assert "Dispatch boundary revalidated" not in session.display()
                assert "EXACT BASKET ENVELOPE" not in session.display()

            def refresh(session):
                session.command("refresh")
                wait_for(session, "eligible files /", timeout=35)
                session.context_mode = "files"

            def choose_basket(session, task_id, excerpt_id):
                show_basket(session, task_id)
                ids = [row["id"] for row in basket(task_id)]
                session.send(HOME + DOWN * ids.index(excerpt_id))
                wait_for(session, "#" + str(excerpt_id))

            def remove(session, task_id, excerpt_id, neutral_check=False):
                choose_basket(session, task_id, excerpt_id)
                before = basket(task_id)
                session.send(DELETE)
                wait_for(session, "Review local excerpt removal")
                if neutral_check:
                    session.capture("neutral-remove")
                    session.send("\r")
                    assert basket(task_id) == before, "neutral Enter removed an excerpt"
                    session.send(DELETE)
                    wait_for(session, "Review local excerpt removal")
                session.send(RIGHT + "\r")
                settled(session, lambda rows: all(row["id"] != excerpt_id for row in rows[1]), "reference was not removed")
                wait_for(session, str(len(before) - 1) + " task-owned basket references / stored")

            session = launch("source-selection")
            first_id = create_task(session, "context first 漢字", first=True)
            open_sources(session)
            for filename in (".env.local", "signing.pem", "opaque.bin", "outside.rs", "ignored.rs", "never.rs"):
                search(session, filename, 0)
                session.capture("omitted-" + filename.replace(".", "_"))
                assert not basket(first_id)
            passed("credential-binary-ignore-outside-link-and-case-folded-prefix-omissions")

            first_capture = pin(session, first_id, "資料.rs", "2:3")
            assert first_capture["path"] == "資料.rs"
            assert Path(first_capture["root"]).is_absolute() and os.path.samefile(first_capture["root"], project)
            assert (first_capture["start_line"], first_capture["end_line"]) == (2, 3)
            assert first_capture["text"] == selected and first_capture["available"]
            assert first_capture["source_sha256"] == list(hashlib.sha256(original).digest())
            assert secret.encode() not in database.read_bytes()
            exact_preview(session, first_id)
            exact_preview(session, first_id, dispatch=True)
            passed("exact-CJK-CRLF-range-source-identity-before-storage-masking-and-payload")

            removable = pin(session, first_id, "other.rs")
            remove(session, first_id, removable["id"], neutral_check=True)
            assert basket(first_id) == [first_capture]
            assert (project / "other.rs").read_bytes() == b"independent removable capture\n"
            passed("neutral-remove-and-source-preservation")

            close_sources(session)
            second_id = create_task(session, "context second retained")
            open_sources(session)
            second_capture = pin(session, second_id, "資料.rs", "2:3")
            assert second_capture["id"] != first_capture["id"]
            assert second_capture["text"] == selected and basket(first_id) == [first_capture]
            quit_app(session)
            session = launch("independent-capture-restart")
            session.command("goal")
            wait_for(session, "context second retained")
            session.send("\x13")
            open_sources(session)
            show_basket(session, second_id)
            assert basket(second_id) == [second_capture]
            exact_preview(session, second_id)
            passed("independent-task-captures-survive-exit-and-resume")

            switch_task(session, 0, first_id, "context first 漢字")
            open_sources(session)
            show_basket(session, first_id)
            session.resize((60, 18))
            session.command("purge")
            wait_for(session, "Review local excerpt removal")
            session.capture("compact-neutral-purge")
            session.send("\r")
            assert basket(first_id) == [first_capture], "neutral Enter purged source bytes"
            session.command("purge")
            wait_for(session, "Review local excerpt removal")
            session.send(RIGHT + "\r")
            settled(session, lambda rows: any(row["id"] == first_capture["id"] and not row["available"] for row in rows[1]), "purge did not persist unavailable metadata")
            wait_for(session, "UNAVAILABLE")
            unavailable = basket(first_id)[0]
            assert unavailable["text"] == "" and unavailable["source_sha256"] == first_capture["source_sha256"]
            assert basket(second_id) == [second_capture] and source.read_bytes() == original
            fail_dispatch(session)
            session.capture("compact-purged-refusal")
            quit_app(session)
            session = launch("purged-metadata-restart")
            open_sources(session)
            show_basket(session, first_id)
            wait_for(session, "UNAVAILABLE")
            assert basket(first_id) == [unavailable]
            fail_dispatch(session)
            passed("neutral-compact-purge-retains-unavailable-metadata-and-other-task-source")

            switch_task(session, 1, second_id, "context second retained")
            open_sources(session)
            show_basket(session, second_id)
            exact_preview(session, second_id, dispatch=True)
            source.write_bytes(original + b"changed outside the selected range\r\n")
            fail_dispatch(session)
            session.capture("full-source-stale-dispatch-refused")
            assert basket(second_id) == [second_capture], "stale validation silently substituted captured text"
            source.write_bytes(original)
            refresh(session)
            show_basket(session, second_id)
            exact_preview(session, second_id, dispatch=True)
            passed("full-source-change-after-preview-refuses-dispatch-without-substitution")

            swap_capture = pin(session, second_id, "replace.rs")
            exact_preview(session, second_id, dispatch=True)
            replacement.unlink()
            link(replacement, outside)
            fail_dispatch(session)
            session.capture("identical-byte-link-swap-refused")
            assert outside.read_bytes() == replacement_bytes
            assert next(row for row in basket(second_id) if row["id"] == swap_capture["id"])["text"] == replacement_bytes.decode()
            replacement.unlink()
            replacement.write_bytes(replacement_bytes)
            refresh(session)
            remove(session, second_id, swap_capture["id"])
            passed("identical-byte-outside-link-replacement-refuses-dispatch")

            missing_capture = pin(session, second_id, "missing.rs")
            missing.unlink()
            fail_dispatch(session)
            session.capture("missing-source-refused")
            assert basket(first_id) == [unavailable] and source.read_bytes() == original
            missing.write_bytes(missing_bytes)
            refresh(session)
            remove(session, second_id, missing_capture["id"])
            passed("missing-source-is-unavailable-not-empty-or-successful-evidence")

            close_sources(session)
            third_id = create_task(session, "context third independent")
            open_sources(session)
            third_capture = pin(session, third_id, "資料.rs", "2:3")
            switch_task(session, 1, second_id, "context second retained")
            open_sources(session)
            show_basket(session, second_id)
            exact_preview(session, second_id)
            # Deliberate external fault injection in disposable SQLite, not a second
            # ev writer: the TUI retains a now-missing reference until admission.
            with closing(sqlite3.connect(database, timeout=3)) as db:
                assert db.execute("DELETE FROM excerpts WHERE task_id=? AND id=?", (second_id, second_capture["id"])).rowcount == 1
                db.commit()
            fail_dispatch(session, layer="storage")
            session.capture("missing-stored-artifact-dispatch-refused")
            assert basket(third_id) == [third_capture]
            assert basket(first_id) == [unavailable]
            assert source.read_bytes() == original and outside.read_bytes() == replacement_bytes
            passed("missing-stored-artifact-refuses-admission-with-other-task-capture-intact")

            quit_app(session)
            assert hashlib.sha256((project / ".git" / "index").read_bytes()).hexdigest() == index_before
            assert source.read_bytes() == original and replacement.read_bytes() == replacement_bytes
            assert missing.read_bytes() == missing_bytes
            assert all(secret not in item.text for item in sessions)
            (output / "committed-baskets.json").write_text(json.dumps(read_rows()[1], indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            result["source_sha256"] = hashlib.sha256(original).hexdigest()
            result["expected_redacted_range"] = selected
            passed("repository-index-source-and-external-bytes-preserved")
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        for session in sessions:
            if not session.closed:
                session.capture("failure")
        raise
    finally:
        for session in sessions:
            session.close()
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
