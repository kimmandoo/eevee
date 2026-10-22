#!/usr/bin/env python3
"""Native cooperation acceptance with real local processes and a deterministic JSONL peer.

No live-provider conformance claim. All task/source/home/control data is disposable;
controlled writes are performed by Eevee, never by the peer. Requires pyte/wcwidth.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC = "\x1b"
UP, RIGHT, LEFT, HOME, DELETE = (ESC + key for key in ("[A", "[C", "[D", "[H", "[3~"))
PARTNERS = ((1, "Mizu", "vaporeon"), (2, "Bolt", "jolteon"), (3, "Mind", "espeon"),
            (4, "Ember", "flareon"), (5, "Blaze", "flareon"), (6, "Ice", "glaceon"))


class Journey:
    def __init__(self, binary, output, size, name):
        self.temporary = tempfile.TemporaryDirectory(prefix="ev-cooperation-漢字-")
        self.root = Path(self.temporary.name).resolve()
        self.project, self.data = self.root / "project", self.root / "data"
        self.project.mkdir()
        self.source = self.project / "calc.py"
        self.original = "def total(a, b):\n    return a + b\n"
        self.replacement = self.original.replace("a + b", "(a + b)")
        self.source.write_text(self.original)
        self.control, runtime, home = (self.root / part for part in ("control", "runtime", "home"))
        for directory in (self.control, runtime, home):
            directory.mkdir()
        self.log = self.root / "peer.jsonl"
        self.log.touch()
        self.database = self.data / "workbench.sqlite3"
        self.process_script = self.root / "owned.py"
        self.process_script.write_text("import pathlib,sys,time\np=pathlib.Path(sys.argv[1])\np.with_suffix('.started').write_text('owned process started')\nprint('OWNED-READY',flush=True)\nwhile not p.with_suffix('.finish').exists(): time.sleep(.03)\nprint('OWNED-DONE',flush=True)\n")
        for argv in (("init", "-q"), ("add", "."), ("-c", "user.name=Smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "fixture")):
            subprocess.run(["git", "-c", "core.hooksPath=", *argv], cwd=self.project, check=True, capture_output=True, timeout=15)
        environment = terminal_environment("mono")
        for key in list(environment):
            if key.startswith(("EV_", "OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GOOGLE_", "GEMINI_")) or key.lower().endswith("_proxy"):
                environment.pop(key, None)
        peer = Path(__file__).with_name("controlled_peer_fixture.py").resolve()
        wrapper = runtime / "codex"
        wrapper.write_text("#!/bin/sh\nexec " + shlex.quote(sys.executable) + " " + shlex.quote(str(peer)) + ' --fake-peer "$@"\n')
        wrapper.chmod(0o700)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / "config"),
                           XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"),
                           PATH=str(runtime) + os.pathsep + environment.get("PATH", ""),
                           CODEX_API_KEY="cooperation-fixture-not-real", EV_COOP_CONTROL=str(self.control),
                           EV_COOP_LOG=str(self.log), EV_COOP_EDIT_PATH="calc.py", EV_COOP_EDIT_CONTENT=self.replacement,
                           EV_PROVIDER="compatible", EV_API_BASE_URL="http://127.0.0.1:9/v1", EV_API_MODEL="unused", EV_API_KEYLESS="1")
        self.argv = [str(binary), "--data-dir", str(self.data), "--images", "off", "--color", "mono"]
        if size[0] == 60:
            self.argv.append("--ascii")
        self.size, self.environment, self.output = size, environment, output / f"{size[0]}-{name}"
        self.session = self.launch("initial")
        self.sessions = [self.session]

    def launch(self, suffix):
        session = Session(self.argv, self.project, self.environment, self.size, self.output / suffix)
        wait_for(session, "Storage: saved", timeout=40)
        return session

    def snapshot(self):
        with closing(sqlite3.connect(self.database.as_uri() + "?mode=ro", uri=True, timeout=1)) as db:
            db.row_factory = sqlite3.Row
            task = dict(db.execute("SELECT * FROM tasks ORDER BY id LIMIT 1").fetchone())
            partners = [dict(row) for row in db.execute("SELECT * FROM partners ORDER BY id")]
            claims = [dict(row) for row in db.execute("SELECT * FROM active_work ORDER BY partner")]
            runs = [json.loads(row[0]) for row in db.execute("SELECT record FROM executions ORDER BY rowid")]
            journals = [json.loads(row[0]) for row in db.execute("SELECT record FROM provider_journal ORDER BY rowid")]
        for key in ("criteria", "partner_drafts", "workflows"):
            task[key] = json.loads(task[key])
        return task, partners, claims, runs, journals

    def peers(self, kind=None, marker=None):
        return [row for line in self.log.read_text().splitlines() if line.strip()
                for row in [json.loads(line)] if (kind is None or row["kind"] == kind) and (marker is None or row.get("marker") == marker)]

    def until(self, predicate, reason, timeout=40, approve_preview=False):
        deadline, approved = time.monotonic() + timeout, False
        while not predicate():
            assert self.session.alive() and time.monotonic() < deadline, reason + "\n" + self.session.display()
            if approve_preview and "y Run" in self.session.display() and not approved:
                self.session.send("y")
                approved = True
            elif approve_preview and "Alt+A" in self.session.display():
                self.session.send(ESC + "a")
            else:
                self.session.pump(.05)

    def workspace(self):
        for _ in range(20):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if 'Alt+A' in footer or footer.startswith(("Type Request /", "Enter Send /", "Draft only /")):
                return footer
            self.session.send("\x13" if "Ctrl+S Save draft/back" in self.session.display() else ESC)
            self.session.pump(.12)
        raise AssertionError("workspace not reachable\n" + self.session.display())

    def lead(self, partner):
        self.workspace()
        self.session.command(f"lead {partner}")
        self.until(lambda: self.snapshot()[0]["lead_partner"] == partner, "lead did not commit")
        self.workspace()

    def draft(self, partner):
        return next((row for row in self.snapshot()[0]["partner_drafts"].get(str(partner), {}).get("drafts", []) if row["field"] == "Request"), None)

    def request(self, partner, text, started=True, review_future=False):
        self.lead(partner)
        if self.workspace().startswith("Type Request /"):
            self.session.send("\r")
        self.session.send(UP * 100 + HOME + DELETE * 512)
        self.session.paste(text)
        self.session.send("\x13")
        self.until(lambda: self.draft(partner) is not None and self.draft(partner)["text"] == text, "literal Request draft was not committed")
        if review_future:
            self.session.send("\r")
            if "y Run" not in self.session.display():
                self.until(lambda: "Alt+A" in self.session.display(), "write review was not queued")
                self.session.send(ESC + "a")
            wait_for(self.session, "y Run", timeout=40)
            self.session.send(ESC)
            if self.workspace().startswith("Type Request /"):
                self.session.send("\r")
            self.session.send(UP * 100 + HOME + DELETE * 512)
            future = "future draft 保持"
            self.session.paste(future)
            self.session.send("\x13")
            self.until(lambda: self.draft(partner) is not None and self.draft(partner)["text"] == future, "future draft did not commit")
            self.session.send(ESC + "a")
            self.until(lambda: any("Request:" in line and text in line for line in self.session.display().splitlines()), "consent did not show the immutable submitted request")
            self.session.capture("immutable-preview-with-newer-draft")
            self.session.send("y")
        else:
            self.session.send("\r")
        if started:
            self.until(lambda: bool(self.peers("started", text)), "provider turn did not start: " + text, approve_preview=True)
            if review_future:
                assert self.draft(partner)["text"] == future
                assert all(text in row["text"] and future not in row["text"] for row in self.peers("started", text)), "peer received the future draft instead of the reviewed immutable request"

    def setup(self, count=6):
        self.session.command("goal")
        self.session.paste("Exercise independent cooperation without live provider calls")
        self.session.send("\r")
        wait_for(self.session, "Storage: saved", timeout=40)
        self.session.command("party")
        for number in range(2, count + 1):
            self.session.send("a")
            self.until(lambda: len(self.snapshot()[1]) == number, "party addition did not commit")
            wait_for(self.session, f"Eevee #{number}")
        self.workspace()
        for partner, nickname, form in PARTNERS[:count]:
            self.lead(partner)
            self.session.command(form)
            self.session.command("nickname")
            self.session.paste(nickname)
            self.session.send("\r")
            self.until(lambda: next(row for row in self.snapshot()[1] if row["id"] == partner)["nickname"] == nickname, "nickname did not commit")
            self.session.command("provider-codex")
            self.workspace()
        self.lead(1)

    def claims(self):
        return {row["partner"] for row in self.snapshot()[2]}

    def finish(self, marker):
        (self.control / (marker + ".finish")).touch()
        self.until(lambda: bool(self.peers("finished", marker)), "peer did not reach terminal completion")

    def publish(self, name, text):
        target = self.control / name
        pending = target.with_name(target.name + ".pending")
        pending.write_text(text)
        pending.replace(target)

    def prompt(self, marker, mode):
        self.publish(marker + ".prompt", mode)
        self.until(lambda: bool(self.peers("prompt", marker)), "peer prompt did not arrive")

    def approve_command(self, token, partner=2):
        self.lead(partner)
        self.session.command("jolteon")
        self.session.command("command-edit")
        wait_for(self.session, "Command / explicit JSON argv or shell")
        self.session.send(UP * 100 + HOME + DELETE * 4096)
        command = json.dumps([str(Path(sys.executable).resolve()), "-B", str(self.process_script), str(token)])
        self.session.paste(command)
        self.session.send("\r")
        self.until(lambda: self.snapshot()[0]["command"] == command and "Storage: saved" in self.session.display(),
                   "exact owned command did not commit before execution review")
        self.session.command("execute")
        wait_for(self.session, "Review one-shot execution permission")
        self.session.send(RIGHT + "\r")
        self.until(lambda: token.with_suffix(".started").exists(), "actual owned command did not start")
        self.workspace()

    def implementation(self, partner):
        self.lead(partner)
        self.session.command("flareon")
        self.session.command("role-implement")
        self.session.command("paths")
        wait_for(self.session, "JSON")
        self.session.send(UP * 100 + HOME + DELETE * 512)
        self.session.paste('["calc.py"]')
        self.session.send("\r")
        self.workspace()

    def release(self, partner, approve=True):
        self.lead(partner)
        self.session.command("release-partner")
        if not approve:
            return
        wait_for(self.session, "Stay")
        self.session.capture("release-preview-" + str(partner))
        self.session.send("y")
        self.until(lambda: partner not in {row["id"] for row in self.snapshot()[1]}, "idle partner release did not commit")
        self.workspace()

    def quit(self):
        self.workspace()
        self.session.command("quit")
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()

    def close(self):
        for token in self.control.glob("local-*"):
            token.with_suffix(".finish").touch()
        for session in self.sessions:
            if not session.closed:
                session.capture("final")
                session.close()
        if self.database.exists():
            (self.output / "evidence.json").write_text(json.dumps(self.snapshot(), indent=2, ensure_ascii=False) + "\n")
        (self.output / "peer.jsonl").write_text(self.log.read_text())
        self.temporary.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    assert os.name == "posix", "This controlled-write fixture requires a native POSIX host"
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "cases": [], "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "scope": "Actual native text UI and owned execution/controlled writes; deterministic protocol peer, not live-provider conformance"}
    active = None
    def passed(message):
        result["cases"].append(message)
        print("passed:", message, flush=True)
    try:
        for size in ((120, 36), (60, 18)):
            active = j = Journey(binary, output, size, "queue")
            j.setup()
            # Only the permission owner needs Implement; readers cannot authorize commands.
            j.implementation(2)
            for partner, marker in ((1, "COOP_A"), (2, "COOP_B"), (3, "COOP_C")):
                j.request(partner, marker, review_future=partner == 2)
            j.until(lambda: j.claims() == {1, 2, 3}, "three independent claims did not coexist")
            assert len({row["pid"] for row in j.peers("started")}) == 3, "same-agent requests shared a transport"
            j.session.command("party-board")
            j.session.capture("three-owner-board")
            for label in ("Mizu", "Bolt", "Mind"):
                assert label in j.session.display(), "board omitted owner " + label
            j.workspace()
            rejected = "COOP_FOURTH_REJECTED 漢字"
            j.request(6, rejected, started=False)
            j.until(lambda: any(word in j.session.display().lower() for word in ("limit", "three", "3 working", "capacity")), "fourth participant had no refusal reason")
            assert j.claims() == {1, 2, 3} and len(j.peers("started")) == 3 and j.draft(6)["text"] == rejected
            passed(f"{size}: three independent same-agent transports and fourth draft-preserving refusal")
            j.release(4)
            assert j.claims() == {1, 2, 3} and not j.peers("finished"), "idle release disturbed another owner's work"
            assert j.draft(6)["text"] == rejected
            passed(f"{size}: idle partner release preserves all other live transports and unrelated drafts")
            j.lead(1)
            if j.workspace().startswith("Type Request /"):
                j.session.send("\r")
            j.session.paste("follow-up draft 保持")
            j.session.send(LEFT * 3 + "\x13")
            j.until(lambda: j.draft(1) is not None and "follow-up" in j.draft(1)["text"], "follow-up draft was not retained")
            saved = j.draft(1).copy()
            j.prompt("COOP_A", "question")
            j.prompt("COOP_B", "edit")
            assert not j.peers("answer"), "arrival auto-answered a prompt"
            j.session.command("party-board")
            j.until(lambda: sum("waiting" in line.lower() for line in j.session.display().splitlines()) >= 2, "board did not show both independently waiting owners")
            board_lines = j.session.display().splitlines()
            for name, status in (("Mizu", "waiting"), ("Bolt", "waiting"), ("Mind", "working")):
                assert any(name in line and index + 1 < len(board_lines) and status in board_lines[index + 1].lower() for index, line in enumerate(board_lines)), "board mixed owner/status: " + name
            j.session.capture("independent-owner-board-events")
            j.session.send(ESC)
            j.session.command("approvals")
            wait_for(j.session, "1 of 2")
            j.session.capture("two-prompt-owner-queue")
            j.session.send(ESC)
            assert not j.peers("answer") and j.claims() == {1, 2, 3}
            j.session.command("approvals")
            j.session.command("answer")
            wait_for(j.session, "Literal answer")
            j.session.paste("only answer A 日本語")
            j.session.send("\r")
            j.session.command("answers-review")
            wait_for(j.session, "y Send")
            j.session.send("y")
            j.until(lambda: bool(j.peers("answer", "COOP_A")), "question answer was not correlated")
            assert "only answer A 日本語" in json.dumps(j.peers("answer", "COOP_A"), ensure_ascii=False)
            assert not j.peers("answer", "COOP_B") and j.draft(1) == saved
            j.session.command("approvals")
            j.session.send("n")  # File review requires explicit denial, not neutral Enter.
            j.until(lambda: bool(j.peers("answer", "COOP_B")), "second owner's denial was not delivered")
            assert j.peers("answer", "COOP_B")[-1]["result"]["success"] is False
            assert j.source.read_text() == j.original, "denied permission changed the source"
            passed(f"{size}: FIFO question/file-review deferral and owner-specific answers preserve follow-up caret")
            j.lead(1)
            j.session.command("cancel")
            j.until(lambda: 1 not in j.claims(), "lead-only cancellation did not settle")
            assert j.draft(1) == saved, "cancellation overwrote the sender's later follow-up draft"
            assert j.claims() == {2, 3} and not j.peers("finished", "COOP_B") and not j.peers("finished", "COOP_C")
            j.session.command("cancel-all")
            wait_for(j.session, "Stay")
            j.session.capture("neutral-cancel-all")
            j.session.send("\r")
            assert j.claims() == {2, 3}
            j.session.command("quit")
            wait_for(j.session, "Stay")
            for label in ("Bolt", "Mind"):
                assert label in j.session.display(), "quit omitted " + label
            j.session.capture("quit-lists-every-owner")
            j.session.send(ESC)
            j.session.command("cancel-all")
            j.session.send("y")
            j.until(lambda: not j.claims(), "cancel-all did not wait for every owner")
            assert all(j.peers("finished", marker) for marker in ("COOP_A", "COOP_B", "COOP_C"))
            journals_before_first_release = j.snapshot()[4]
            j.release(1)
            assert {row["id"] for row in j.snapshot()[1]} == {2, 3, 5, 6}
            assert j.snapshot()[0]["lead_partner"] != 1 and "1" not in j.snapshot()[0]["partner_drafts"]
            assert j.snapshot()[4] == journals_before_first_release
            j.quit()
            passed(f"{size}: lead-only stop, neutral all-stop/quit, complete independent settlement")
            j.session = j.launch("first-partner-release-restart")
            j.sessions.append(j.session)
            assert {row["id"] for row in j.snapshot()[1]} == {2, 3, 5, 6}
            assert j.snapshot()[4] == journals_before_first_release and not j.claims()
            assert len(j.peers("started")) == 3
            j.quit()
            passed(f"{size}: idle first partner released with repaired lead, retained evidence and no restart replay")
            j.close(); active = None

            active = j = Journey(binary, output, size, "idle-first")
            j.setup(count=4)
            j.implementation(4)
            for partner in (2, 3, 4):
                j.request(partner, f"COOP_IDLE_FIRST_{partner}")
            j.until(lambda: j.claims() == {2, 3, 4}, "three other owners did not coexist before first-partner release")
            j.until(lambda: len(j.snapshot()[4]) == 3, "live-owner observations did not commit before release")
            historical = j.snapshot()[4]
            j.release(1)
            assert j.claims() == {2, 3, 4} and not j.peers("finished")
            assert j.snapshot()[4] == historical and j.snapshot()[0]["lead_partner"] in {2, 3, 4}
            j.session.command("party-board")
            for name in ("Bolt", "Mind", "Ember"):
                assert name in j.session.display(), "first-partner release lost live owner " + name
            j.session.capture("first-partner-released-during-three-owned-turns")
            j.workspace()
            j.session.command("quit")
            wait_for(j.session, "Stay")
            j.session.send("\r")
            assert j.session.alive() and j.claims() == {2, 3, 4}
            j.session.command("quit")
            wait_for(j.session, "Stay")
            j.session.send("y")
            confirm_exit(j.session)
            assert j.session.wait_exit() == 0
            j.session.close()
            assert not j.claims() and j.snapshot()[4] == historical
            assert all(j.peers("finished", f"COOP_IDLE_FIRST_{partner}") for partner in (2, 3, 4))
            j.session = j.launch("restart"); j.sessions.append(j.session)
            assert {row["id"] for row in j.snapshot()[1]} == {2, 3, 4}
            assert not j.claims() and j.snapshot()[4] == historical and len(j.peers("started")) == 3
            assert {record["partner"]["name"] for record in historical} == {"Bolt", "Mind", "Ember"}
            j.quit()
            passed(f"{size}: idle first/current lead release during three other owned turns; neutral quit commits journals and ownership before exit, restart never replays")
            j.close(); active = None

            active = j = Journey(binary, output, size, "lease")
            j.setup()
            j.request(1, "COOP_READER_FIRST")
            j.finish("COOP_READER_FIRST")
            j.until(lambda: not j.claims(), "first readonly turn did not settle")
            j.request(1, "COOP_READER")
            assert len(j.peers("started", "COOP_READER_FIRST")) == len(j.peers("started", "COOP_READER")) == 1
            sessions = j.peers("session")
            assert [row["method"] for row in sessions] == ["thread/start", "thread/resume"], "consecutive owned turn did not resume the retained native session"
            assert sessions[0]["thread"] == sessions[1]["thread"]
            passed(f"{size}: completed provider session resumes through retained owner registry without replay")
            j.implementation(4)
            j.request(4, "COOP_WRITER")
            j.until(lambda: any(row["partner"] == 4 and row["writing"] for row in j.snapshot()[2]), "writer lease was not durable")
            j.implementation(5)
            j.request(5, "COOP_SECOND_WRITER", started=False)
            j.until(lambda: "editing" in j.session.display().lower(), "second writer lacked owning-writer refusal")
            assert not j.peers("started", "COOP_SECOND_WRITER") and j.source.read_text() == j.original
            token = j.control / "local-1"
            j.approve_command(token)
            j.until(lambda: j.claims() == {1, 2, 4}, "provider/read/write/local command ownership was not independent")
            j.release(2, approve=False)
            j.until(lambda: "Refused:" in j.session.display() and "owns work" in j.session.display(), "working owner release lacked its explicit refusal")
            assert len(j.snapshot()[1]) == 6
            j.workspace()
            j.prompt("COOP_WRITER", "edit")
            j.session.command("approvals")
            wait_for(j.session, "Allow once")
            j.session.capture("writer-patch-own-permission")
            j.session.send("a")
            j.until(lambda: j.source.read_text() == j.replacement, "actual reviewed write callback was not applied")
            token.with_suffix(".finish").touch()
            j.until(lambda: len(j.snapshot()[3]) == 1 and j.snapshot()[3][0]["state"] == "Succeeded", "overlapping real command did not finish")
            moving = j.snapshot()[3][0]
            assert moving["revision_moving"] is True and moving["prepared"]["partner"]["partner"] == 2
            j.lead(6)
            j.session.command("workflow-refactor")
            j.session.command("refactor-baseline")
            j.until(lambda: "revision moving" in j.session.display().lower(), "moving evidence was not visibly excluded")
            j.session.capture("moving-run-ineligible")
            j.workspace()
            j.finish("COOP_WRITER")
            j.finish("COOP_READER")
            j.until(lambda: not j.claims(), "writer lease did not release after terminal settlement")
            assert j.snapshot()[3][0]["revision_moving"] is True
            passed(f"{size}: one writer, real simultaneous provider/command, moving-run baseline refusal and exact controlled write")
            token = j.control / "local-2"
            j.approve_command(token)
            token.with_suffix(".finish").touch()
            j.until(lambda: len(j.snapshot()[3]) == 2 and j.snapshot()[3][-1]["state"] == "Succeeded", "fresh actual command did not finish")
            assert not j.snapshot()[3][-1].get("revision_moving", False)
            j.until(lambda: not j.claims(), "fresh command ownership did not settle")
            evidence = j.snapshot()[3]
            j.release(2)
            assert j.snapshot()[3] == evidence and evidence[-1]["prepared"]["partner"]["name"] == "Bolt"
            assert j.snapshot()[0]["lead_partner"] != 2
            for partner in (3, 4, 5, 6):
                j.release(partner)
            j.release(1, approve=False)
            j.until(lambda: any(word in j.session.display().lower() for word in ("last", "remain", "one partner")), "last partner was not protected")
            assert len(j.snapshot()[1]) == 1 and j.snapshot()[3] == evidence
            j.quit()
            j.session = j.launch("restart"); j.sessions.append(j.session)
            assert len(j.snapshot()[1]) == 1 and j.snapshot()[3] == evidence and not j.claims()
            j.session.command("runs")
            wait_for(j.session, "Bolt")
            j.session.capture("released-owner-snapshot-retained")
            j.quit()
            passed(f"{size}: idle release repairs leads and survives restart; evidence name snapshots and last-partner protection remain")
            j.close(); active = None

            active = j = Journey(binary, output, size, "local-slots")
            j.setup(count=4)
            for partner in (1, 2, 3):
                j.approve_command(j.control / f"local-{partner}", partner)
            j.until(lambda: j.claims() == {1, 2, 3}, "three real local commands did not coexist")
            assert {r["prepared"]["partner"]["partner"] for r in j.snapshot()[3]} == {1, 2, 3}
            j.session.command("party-board")
            j.session.capture("three-independent-owned-commands")
            j.lead(4)
            j.session.command("jolteon")
            j.session.command("command-edit")
            wait_for(j.session, "Command / explicit JSON argv or shell")
            j.session.send(UP * 100 + HOME + DELETE * 4096)
            fourth = j.control / "local-4"
            fourth_command = json.dumps([str(Path(sys.executable).resolve()), "-B", str(j.process_script), str(fourth)])
            j.session.paste(fourth_command)
            j.session.send("\r")
            j.until(lambda: any(word in j.session.display().lower() for word in ("limit", "three", "3 working", "capacity")), "fourth local request had no capacity refusal")
            assert not fourth.with_suffix(".started").exists() and j.claims() == {1, 2, 3}
            j.session.capture("fourth-local-capacity-refusal-draft-retained")
            j.workspace()
            j.until(lambda: any(row["field"] == "Command" and row["text"] == fourth_command
                                for row in j.snapshot()[0]["partner_drafts"].get("4", {}).get("drafts", [])),
                    "capacity refusal lost the fourth local command draft")
            fourth_drafts = j.snapshot()[0]["partner_drafts"]["4"]["drafts"]
            j.session.command("command-edit")
            wait_for(j.session, "Command / explicit JSON argv or shell")
            assert "local-4" in "".join(line.strip(" |║") for line in j.session.display().splitlines()), "refused local command was not retained in the editor"
            j.workspace()
            assert j.snapshot()[0]["partner_drafts"]["4"]["drafts"] == fourth_drafts
            j.lead(2)
            j.session.command("run")
            j.session.command("cancel")
            j.until(lambda: 2 not in j.claims(), "one local owner could not cancel independently")
            assert j.claims() == {1, 3}
            j.session.command("cancel-all")
            wait_for(j.session, "Stay")
            j.session.send("y")
            j.until(lambda: not j.claims(), "all local workers did not settle")
            assert len(j.snapshot()[3]) == 3 and all(r["state"] == "Cancelled" for r in j.snapshot()[3])
            assert not j.peers("started"), "local command work dispatched a provider"
            j.quit()
            passed(f"{size}: three actual local command workers, fourth refusal and isolated/all-owner cancellation")
            j.close(); active = None

            active = j = Journey(binary, output, size, "journal-limit")
            j.setup(count=1)
            # Stay below the transport's per-turn 128-item decision-retention bound.
            # Three genuine resumed turns still hit the task's real 257th journal Put.
            for batch, count in enumerate((100, 100, 57), 1):
                quota_marker = f"COOP_QUOTA_{batch}"
                baseline = len(j.snapshot()[4])
                j.publish(quota_marker + ".observations", str(count))
                j.request(1, quota_marker)
                for emitted in range(1, count):
                    j.until(lambda: len(j.snapshot()[4]) == baseline + emitted, "paced provider observations did not persist")
                    j.publish(quota_marker + ".observe-next", str(emitted + 1))
                j.until(lambda: bool(j.peers("observed_batch", quota_marker)), "public observation fixture batch did not finish")
                if batch < 3:
                    j.until(lambda: len(j.snapshot()[4]) == baseline + count, "completed quota batch did not commit")
                    j.finish(quota_marker)
                    j.until(lambda: not j.claims(), "completed quota batch did not settle")
            j.until(lambda: len(j.snapshot()[4]) == 256 and any(word in j.session.display().lower() for word in ("limit", "full", "failed")), "257th native observation did not exhaust the actual journal quota")
            j.session.command("journal-discard")
            j.until(lambda: "terminal" in j.session.display().lower(), "running provider did not refuse journal disposal")
            assert j.claims() == {1} and len(j.snapshot()[4]) == 256 and not j.peers("finished", quota_marker)
            j.session.capture("journal-discard-refused-before-terminal")
            j.workspace()
            j.finish(quota_marker)
            j.session.command("response")
            j.session.command("history")
            j.until(lambda: len(j.snapshot()[4]) == 256 and any(word in j.session.display().lower() for word in ("limit", "full", "failed")), "real journal capacity failure was not visible")
            assert j.claims() == {1}, "unsaved observation silently freed ownership"
            j.session.command("journal-discard")
            wait_for(j.session, "Stay")
            j.session.capture("journal-gap-neutral-recovery")
            j.session.send("\r")
            assert j.claims() == {1} and len(j.snapshot()[4]) == 256
            j.session.command("journal-discard")
            wait_for(j.session, "Stay")
            j.session.send("d")
            wait_for(j.session, "Exact failed")
            j.session.capture("journal-exact-failed-attempt")
            j.session.send(ESC + "[F")
            j.until(lambda: "coverage gap" in j.session.display().lower(), "discard details omitted the unverified coverage gap")
            j.session.capture("journal-exact-failure-error-and-gap")
            j.session.send("d")
            wait_for(j.session, "y Confirm")
            j.session.send("y")
            j.until(lambda: not j.claims(), "explicit terminal-only journal disposal did not settle ownership")
            j.until(lambda: "coverage gap" in j.session.display().lower(), "discard did not expose the unverified journal coverage gap")
            j.session.capture("journal-gap-after-explicit-discard")
            records = j.snapshot()[4]
            turns = list(dict.fromkeys(record["turn_id"] for record in records))
            assert len(turns) == 3 and [sum(record["turn_id"] == turn for record in records) for turn in turns] == [100, 100, 56]
            assert len({(record["turn_id"], record["item_id"]) for record in records}) == 256
            assert len(records) == 256 and not j.snapshot()[3]
            assert len(j.peers("started")) == 3 and all(len(j.peers("started", f"COOP_QUOTA_{batch}")) == 1 for batch in (1, 2, 3))
            quota_sessions = j.peers("session")
            assert [row["method"] for row in quota_sessions] == ["thread/start", "thread/resume", "thread/resume"]
            assert len({row["thread"] for row in quota_sessions}) == 1
            j.quit()
            passed(f"{size}: real journal quota failure has explicit owner-bound disposal, retained history and no provider replay")
            j.close(); active = None

            active = j = Journey(binary, output, size, "crash")
            j.setup(count=1)
            j.implementation(1)
            j.request(1, "COOP_CRASH")
            j.until(lambda: bool(j.snapshot()[2]) and j.snapshot()[2][0]["writing"], "crash fixture never held a write lease")
            j.session.capture("before-actual-hard-kill")
            j.session.process.kill(); j.session.process.wait(timeout=5); j.session.close()
            assert j.snapshot()[2], "hard kill was replaced with a graceful release"
            j.session = j.launch("recovered"); j.sessions.append(j.session)
            j.until(lambda: not j.claims(), "startup left an orphan write lease")
            assert len(j.peers("started", "COOP_CRASH")) == 1 and j.source.read_text() == j.original
            j.implementation(1)
            j.session.command("provider-codex")
            j.workspace()
            j.request(1, "COOP_CRASH_NEW")
            j.finish("COOP_CRASH_NEW")
            j.until(lambda: not j.claims(), "new writer could not settle after crash recovery")
            j.quit()
            passed(f"{size}: real hard-kill recovery clears stale lease without replay and permits a fresh request")
            j.close(); active = None
        result["status"] = "passed"
    finally:
        if active:
            active.close()
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
