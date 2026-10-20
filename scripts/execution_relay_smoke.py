#!/usr/bin/env python3
"""Actual native-TUI execution relay acceptance with real failing cargo/pytest output.

Uses disposable projects and data only. No live model, credentials, release or
installed command is touched. Requires pytest, pyte and wcwidth in this Python.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC = "\x1b"
UP, DOWN, RIGHT, HOME, DELETE = (ESC + code for code in ("[A", "[B", "[C", "[H", "[3~"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "cases": [], "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "scope": "Native text PTY at both sizes; actual local cargo/pytest; no GUI/native-image/live-provider claim"}
    sessions = []

    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)

    try:
        cargo = Path(subprocess.check_output(["rustup", "which", "--toolchain", "1.90.0", "cargo"], text=True).strip())
        python = Path(sys.executable).resolve()
        # The runner executes canonical binaries. Use pytest's real entry-point
        # script so its venv shebang retains the isolated pytest installation.
        pytest_cli = Path(sys.executable).absolute().with_name("pytest")
        assert pytest_cli.is_file(), "pytest entry point missing from the smoke environment"
        for width, height in ((120, 36), (60, 18)):
            with tempfile.TemporaryDirectory(prefix="ev-relay-東京-") as temporary:
                root = Path(temporary).resolve()
                project, data = root / "project", root / "data"
                (project / "src").mkdir(parents=True)
                (project / "Cargo.toml").write_text('[package]\nname="relay_fixture"\nversion="0.0.0"\nedition="2021"\n')
                (project / "src/lib.rs").write_text('#[test]\nfn actual_failure() {\n    assert_eq!(1, 2);\n}\n')
                (project / "test_failure.py").write_text('def test_actual_failure():\n    actual = 1\n    assert actual == 2\n')
                (project / ".gitignore").write_text("target/\n.pytest_cache/\n__pycache__/\n.env\n")
                (project / ".env").write_text("PRIVATE=fixture\n")
                for argv in (("init", "--quiet"), ("add", "."), ("-c", "user.name=Smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "fixture")):
                    subprocess.run(["git", "-c", "core.hooksPath=", *argv], cwd=project, check=True, capture_output=True)
                database = data / "workbench.sqlite3"
                environment = terminal_environment("mono")
                for key in list(environment):
                    if key.startswith(("EV_", "OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GOOGLE_", "GEMINI_")) or key.lower().endswith("_proxy"):
                        environment.pop(key, None)
                home, runtime, control = root / "home", root / "runtime", root / "control"
                for directory in (home, runtime, control):
                    directory.mkdir()
                peer_log = root / "peer.jsonl"
                peer_log.touch()
                executable = runtime / "codex"
                peer_script = Path(__file__).with_name("controlled_peer_fixture.py").resolve()
                executable.write_text("#!/bin/sh\nexec " + shlex.quote(sys.executable) + " " + shlex.quote(str(peer_script)) + " --fake-peer \"$@\"\n")
                executable.chmod(0o700)
                environment.update(PATH=str(runtime) + os.pathsep + environment.get("PATH", ""),
                                   HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / "config"),
                                   CODEX_API_KEY="relay-fixture-not-real", EV_COOP_CONTROL=str(control), EV_COOP_LOG=str(peer_log))
                environment["RUSTUP_HOME"] = os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))
                environment["CARGO_HOME"] = os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
                environment["EV_PROVIDER"] = "compatible"
                environment["EV_API_BASE_URL"] = "http://127.0.0.1:9/v1"
                environment["EV_API_MODEL"] = "unused"
                environment["EV_API_KEYLESS"] = "1"
                argv = [str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"]
                if width == 60:
                    argv.append("--ascii")
                session = Session(argv, project, environment, (width, height), output / str(width))
                sessions.append(session)

                def rows():
                    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                        db.row_factory = sqlite3.Row
                        task = dict(db.execute("SELECT * FROM tasks ORDER BY id LIMIT 1").fetchone())
                        runs = [json.loads(row[0]) for row in db.execute("SELECT record FROM executions ORDER BY rowid")]
                        pins = [json.loads(row[0]) for row in db.execute("SELECT record FROM excerpts ORDER BY id")]
                        journal = db.execute("SELECT count(*) FROM provider_journal").fetchone()[0]
                    return task, runs, pins, journal

                def until(predicate, reason, timeout=60):
                    deadline = time.monotonic() + timeout
                    while not predicate():
                        assert session.alive(), reason + ": application exited"
                        assert time.monotonic() < deadline, reason + "\n" + session.display()
                        session.pump(0.05)

                def execute(argv, expected):
                    session.command("jolteon")
                    session.command("run")
                    session.command("command-edit")
                    wait_for(session, "Command / explicit JSON argv or shell", timeout=30)
                    current = rows()[0]["command"]
                    session.send(HOME + DELETE * len(current))
                    session.paste(json.dumps([str(value) for value in argv], ensure_ascii=False))
                    session.send("\r")
                    wait_for(session, "Approval pending", timeout=40)
                    session.command("run")
                    wait_for(session, "Review one-shot execution permission", timeout=30)
                    session.send(RIGHT + "\r")
                    until(lambda: len(rows()[1]) == expected and rows()[1][-1]["ended_at_ms"] is not None,
                          "owned command did not settle", timeout=120)
                    wait_for(session, "Local command evidence; no command running", timeout=40)
                    return rows()[1][-1]

                def locations():
                    session.command("history")
                    session.send(UP * 256 + "\r")
                    until(lambda: "Retained bytes" in session.display(), "retained output did not open")
                    session.command("locations")
                    wait_for(session, "Retained window/page only", timeout=30)

                wait_for(session, "Storage: saved", timeout=40)
                session.send(ESC)
                session.command("goal")
                session.paste("Exercise guarded execution relays 東京")
                session.send("\r")
                wait_for(session, "Storage: saved", timeout=40)
                session.command("criteria-edit")
                session.paste("Diagnostic locations are available")
                session.send("\r")
                wait_for(session, "Storage: saved", timeout=40)
                cargo_run = execute([cargo, "test", "--offline", "--color", "never"], 1)
                assert cargo_run["state"] == "Failed" and cargo_run["exit_code"] != 0
                locations()
                wait_for(session, "src/lib.rs:3", timeout=30)
                session.capture("cargo-locations")
                session.send("\r")
                wait_for(session, "From run " + cargo_run["prepared"]["run_id"], timeout=30)
                session.capture("capture-review-provenance")
                session.send("\r")
                until(lambda: len(rows()[2]) == 1, "location capture did not commit")
                pin = rows()[2][0]
                assert pin["origin_run"] == cargo_run["prepared"]["run_id"]
                assert pin["path"] == "src/lib.rs" and pin["start_line"] <= 3 <= pin["end_line"]
                assert "assert_eq!(1, 2)" in pin["text"]
                passed(f"{width}: actual cargo failure locations and reviewed pin provenance")
                session.send(ESC)
                pytest_run = execute([pytest_cli, "--tb=short", "-q", "-p", "no:cacheprovider", "test_failure.py"], 2)
                assert pytest_run["state"] == "Failed" and pytest_run["exit_code"] == 1
                locations()
                wait_for(session, "test_failure.py:3", timeout=30)
                session.capture("pytest-locations")
                passed(f"{width}: actual pytest failure location")
                session.send(ESC)
                session.command("party")
                session.send("a")
                def party_count():
                    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                        return db.execute("SELECT count(*) FROM partners").fetchone()[0]
                until(lambda: party_count() == 2, "second partner did not commit")
                session.send(ESC)
                session.command("lead #2")
                session.command("umbreon")
                session.command("lead #1")
                until(lambda: all(part in " ".join(session.display().split()) for part in ("Eevee #2", "(Umbreon)", "is free")),
                      "idle suggested partner not shown across the wrapped workspace")
                session.capture("idle-partner-suggestion")
                passed(f"{width}: suggested form names its idle party member")
                assert rows()[3] == 0, "local relay navigation created a provider journal"
                session.command("lead #2")
                session.command("provider-codex")
                session.command("new")
                session.paste("COOP_IDLE_BUSY")
                session.send("\r")
                deadline = time.monotonic() + 60
                while not any(json.loads(line).get("kind") == "started" for line in peer_log.read_text().splitlines()):
                    assert session.alive() and time.monotonic() < deadline, session.display()
                    if "y Run" in session.display():
                        session.send("y")
                    else:
                        session.pump(.05)
                session.command("lead #1")
                until(lambda: "is free" not in " ".join(session.display().split()),
                      "busy owning partner still advertised as available")
                session.capture("busy-partner-not-suggested-as-free")
                (control / "COOP_IDLE_BUSY.finish").touch()
                until(lambda: all(part in " ".join(session.display().split()) for part in ("Eevee #2", "(Umbreon)", "is free")),
                      "settled partner availability did not return")
                session.capture("settled-partner-suggestion")
                passed(f"{width}: idle hint disappears during actual owned provider work and returns after settlement")
                session.send(ESC)
                execute([python, "-B", "-c", 'print("no diagnostic locations")'], 3)
                locations()
                wait_for(session, "No parseable locations", timeout=30)
                session.capture("no-location")
                assert len(rows()[2]) == 1
                passed(f"{width}: unparseable output creates no location or pin")
                session.send(ESC)
                refs = 'print("/outside-root/absent.rs:1\\n.env:1\\nmissing.rs:1")'
                execute([python, "-B", "-c", refs], 4)
                locations()
                session.send("\r")
                wait_for(session, "outside", timeout=30)
                session.capture("outside-refused")
                session.send(DOWN + "\r")
                wait_for(session, "excluded", timeout=30)
                session.capture("excluded-refused")
                session.send(ESC)
                session.command("run")
                session.command("locations")
                session.send(DOWN * 2 + "\r")
                wait_for(session, "absent", timeout=30)
                session.capture("missing-refused")
                assert len(rows()[2]) == 1
                passed(f"{width}: out-of-root excluded and missing references refused")
                session.send(ESC)
                stable = execute([python, "-B", "-c", 'print("actual stable check")'], 5)
                assert stable["state"] == "Succeeded"
                stable_bytes = json.dumps(stable, sort_keys=True)
                session.send(ESC * 2)  # Use ordinary home refresh, not execution-specific refresh.
                with (project / "src/lib.rs").open("a") as changed:
                    changed.write("// deliberately changed after the passing run\n")
                session.command("refresh")
                session.command("run")
                session.command("history")
                session.send(UP * 256)
                until(lambda: "Stale" in session.display(), "source edit did not stale the retained check")
                session.command("rerun")
                wait_for(session, "Approval pending")
                session.command("approvals")
                wait_for(session, "Review one-shot execution permission", timeout=40)
                session.send(ESC)
                assert len(rows()[1]) == 5, "dismissed rerun review executed a command"
                session.command("approvals")
                session.command("deny")
                assert len(rows()[1]) == 5, "denied rerun executed a command"
                session.command("rerun")
                wait_for(session, "Approval pending")
                session.command("approvals")
                wait_for(session, "Review one-shot execution permission")
                session.send(RIGHT + "\r")
                until(lambda: len(rows()[1]) == 6 and rows()[1][-1]["ended_at_ms"] is not None,
                      "approved stale rerun did not finish")
                rerun = rows()[1][-1]
                assert rerun["prepared"]["rerun_of"] == stable["prepared"]["run_id"]
                assert rerun["prepared"]["spec"] == stable["prepared"]["spec"]
                assert rerun["prepared"]["cwd"] == stable["prepared"]["cwd"]
                assert rerun["prepared"]["before"]["fingerprint"] != stable["prepared"]["before"]["fingerprint"]
                assert json.dumps(rows()[1][-2], sort_keys=True) == stable_bytes
                session.capture("rerun-new-evidence-old-unchanged")
                passed(f"{width}: stale rerun cancellation denial new approval and immutable linked evidence")
                assert sum(json.loads(line).get("kind") == "started" for line in peer_log.read_text().splitlines()) == 1, "relay navigation dispatched an additional provider turn"
                session.send(ESC)
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()
        result["status"] = "passed"
    finally:
        for session in sessions:
            if not session.closed:
                session.capture("final")
                session.close()
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
