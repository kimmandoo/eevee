#!/usr/bin/env python3
"""PT10/EV03B actual keyboard proof for reference-only owning-partner redirects.

Exercises both supported sizes with disposable tasks and native party controls.
SQLite is read-only observation. A loopback fixture detects any accidental send;
no provider request, command preparation/execution or source write is authorized.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import NativeFixture, wait_for

ESC = "\x1b"
HOME, LEFT = ESC + "[H", ESC + "[D"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = {"status": "failed", "cases": [], "scope": "Native keyboard UI; references only, zero provider submissions/executions/writes", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    sessions = []
    try:
        for size in ((120, 36), (60, 18)):
            with tempfile.TemporaryDirectory(prefix="ev-owner-redirect-") as temporary, NativeFixture() as fixture:
                root = Path(temporary).resolve()
                project, data, home = root / "project", root / "data", root / "home"
                project.mkdir()
                home.mkdir()
                source = project / "note.txt"
                source.write_text("Owning-role reference fixture. No edit is authorized.\n")
                original = source.read_bytes()
                for argv in (("init", "-q"), ("add", "--", "note.txt"), ("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")):
                    subprocess.run(["git", "-c", "core.hooksPath=", *argv], cwd=project, check=True, capture_output=True, timeout=15)
                env = terminal_environment("mono")
                for key in list(env):
                    if key.startswith(("EV_", "OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GOOGLE_", "GEMINI_")) or key.lower().endswith("_proxy"):
                        env.pop(key, None)
                env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"), EV_PROVIDER="compatible", EV_API_BASE_URL=f"http://127.0.0.1:{fixture.server.server_port}/v1", EV_API_PROTOCOL="chat_completions", EV_API_MODEL="owner-reference-fixture", EV_API_KEYLESS="1", NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
                argv = [str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"]
                if size[0] == 60:
                    argv.append("--ascii")
                session = Session(argv, project, env, size, output / str(size[0]))
                sessions.append(session)
                database = data / "workbench.sqlite3"

                def rows():
                    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as connection:
                        connection.row_factory = sqlite3.Row
                        task = dict(connection.execute("SELECT * FROM tasks ORDER BY id LIMIT 1").fetchone())
                        partners = [dict(row) for row in connection.execute("SELECT * FROM partners ORDER BY id")]
                        runs = connection.execute("SELECT count(*) FROM executions").fetchone()[0]
                        journals = connection.execute("SELECT count(*) FROM provider_journal").fetchone()[0]
                    for field in ("criteria", "partner_drafts", "workflows"):
                        task[field] = json.loads(task[field])
                    return task, partners, runs, journals

                def captured_source():
                    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as connection:
                        return any(record["path"] == "note.txt" and record["text"].encode() == original
                                   for row in connection.execute("SELECT record FROM excerpts")
                                   for record in [json.loads(row[0])])

                def until(predicate, why, timeout=20):
                    deadline = time.monotonic() + timeout
                    while not predicate():
                        assert session.alive() and time.monotonic() < deadline, why + "\n" + session.display()
                        session.pump(.05)

                def saved():
                    wait_for(session, "Storage: saved", timeout=30)
                    session.pump(.6)

                def draft(partner):
                    value = rows()[0]["partner_drafts"].get(str(partner), {})
                    return next((item for item in value.get("drafts", []) if item["field"] == "Request"), None)

                def lead(partner):
                    session.command(f"lead {partner}")
                    saved()

                def back_to_workspace():
                    # Unwind actual frames without escaping into a retained text editor.
                    for _ in range(12):
                        footer = session.display().rstrip().splitlines()[-1].strip()
                        if footer.startswith(("Type Request |", "Enter Send |")):
                            return
                        session.send(ESC)
                        session.pump(.15)
                    raise AssertionError("workspace not reached\n" + session.display())

                def no_effects():
                    task, _, runs, journals = rows()
                    assert runs == journals == 0, "redirect started or fabricated work"
                    assert task["command"] == "", "redirect invented command text"
                    assert not fixture.snapshot(), "redirect made a provider/metadata request"
                    assert source.read_bytes() == original, "redirect wrote source"

                def offer(action, owner):
                    session.command(action)
                    wait_for(session, "Switch to " + owner)
                    no_effects()

                def choose_send(name):
                    wait_for(session, "Send to " + name)
                    session.send("Send to " + name + "\r")
                    wait_for(session, "[Stay]")  # Observe neutral consent, not an incidental caption.
                    session.capture("review-send-" + name.lower())

                wait_for(session, "Storage: saved", timeout=30)
                session.command("goal")
                session.paste("Review a role handoff without executing or editing anything")
                session.send("\r")
                saved()
                session.command("files")
                wait_for(session, "eligible files /", timeout=30)
                session.command("search")
                session.paste("note.txt")
                session.send("\r")
                wait_for(session, "1 eligible files /")
                session.send("\r")
                wait_for(session, "Review local capture /")
                session.command("attach")
                until(captured_source, "exact owning-role source capture did not commit")
                back_to_workspace()
                saved()
                session.command("party")
                for count in (2, 3, 4):
                    session.send("a")
                    until(lambda: len(rows()[1]) == count, "party addition did not commit")
                    wait_for(session, f"Eevee #{count}")
                session.send(ESC)
                for partner, name, form in ((2, "Bolt", "jolteon"), (3, "Ember", "flareon"), (4, "Ash", "flareon")):
                    lead(partner)
                    session.command(form)
                    session.command("nickname")
                    session.paste(name)
                    session.send("\r")
                    until(lambda: next(row for row in rows()[1] if row["id"] == partner)["nickname"] == name, "nickname not committed")
                    wait_for(session, "Partner nickname saved locally")
                    saved()
                lead(1)
                session.command("espeon")
                assert draft(1) is None or not draft(1)["text"], "setup must not submit an existing request"
                if session.display().rstrip().splitlines()[-1].strip().startswith("Type Request |"):
                    session.send("\r")
                session.paste("원래 요청 draft 保持 🧭 sender")
                session.send(LEFT * 4)
                session.send("\x13")
                saved()
                sender = draft(1)
                assert sender and sender["cursor"] < len(sender["text"].encode()), "fixture did not retain an interior caret"
                facts = {key: rows()[0][key] for key in ("id", "goal", "criteria", "next_action")}

                offer("command-edit", "Jolteon")
                wait_for(session, "Send to Bolt")
                session.capture("owning-command-options")
                session.send(ESC)
                assert draft(1) == sender and draft(2) is None
                no_effects()
                offer("command-edit", "Jolteon")
                session.send("\r")  # Existing first-row Switch, never command replay.
                until(lambda: rows()[0]["form"] == 2, "owning-form switch did not commit")
                assert draft(1) == sender and rows()[0]["lead_partner"] == 1
                assert "Command / explicit JSON argv or shell" not in session.display()
                no_effects()
                session.capture("switch-stays-nonreplaying")
                session.command("espeon")
                saved()
                offer("command-edit", "Jolteon")
                choose_send("Bolt")
                session.send("\r")  # Neutral Enter returns to target selection, never stages.
                assert draft(2) is None and draft(1) == sender
                back_to_workspace()
                offer("command-edit", "Jolteon")
                choose_send("Bolt")
                session.send("y")
                until(lambda: draft(2) is not None, "command-role references not staged")
                assert "command-edit" in draft(2)["text"]
                assert draft(1) == sender and rows()[0]["lead_partner"] == 1 and rows()[0]["form"] == 4
                no_effects()
                target = draft(2).copy()
                back_to_workspace()
                offer("command-edit", "Jolteon")
                choose_send("Bolt")
                session.send("y")
                wait_for(session, "nonempty")
                assert draft(2) == target and draft(1) == sender
                session.capture("nonempty-target-refused")
                back_to_workspace()

                session.command("glaceon")
                saved()
                for action, name, partner in (("paths", "Ember", 3), ("role-implement", "Ash", 4)):
                    offer(action, "Flareon")
                    wait_for(session, "Send to " + name)
                    session.capture("owning-" + action + "-options")
                    choose_send(name)
                    session.send("y")
                    until(lambda: draft(partner) is not None, "editing-role references not staged")
                    assert action in draft(partner)["text"]
                    assert draft(1) == sender and rows()[0]["lead_partner"] == 1 and rows()[0]["form"] == 7
                    relay = next(row for row in rows()[0]["workflows"]["draft_relays"] if row["partner"] == partner)
                    assert relay["paths"] == [], "reference-only redirect invented writable paths"
                    no_effects()
                    back_to_workspace()
                assert {key: rows()[0][key] for key in facts} == facts
                result["cases"].append({"size": list(size), "status": "passed", "actions": ["command-edit", "paths", "role-implement"], "proof": "named idle owning partners; neutral review/escape; nonempty refusal; exact sender draft/caret/facts and source preserved; no argv/paths/grants/provider or execution"})
                session.capture("sender-and-reference-only-results")
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()
        result["status"] = "passed"
    finally:
        for session in sessions:
            session.close()
        (output / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
