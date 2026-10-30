#!/usr/bin/env python3
"""Native Moves acceptance against one immutable, already-built executable.

Runs disposable real commands and a keyless loopback HTTP/SSE peer. This is local
TUI/transport evidence, NOT real-account/provider conformance. Never writes SQL,
builds the application, touches the user's home, or invokes an installed ev.
Requires the same pyte/wcwidth dependencies as terminal_smoke.py.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for
from execution_smoke import NativeProcess
from native_connection_smoke import Peer, MODEL, decoded_payload

ESC = "\x1b"
HOME, END = ESC + "[H", ESC + "[F"
UP, DOWN, RIGHT = ESC + "[A", ESC + "[B", ESC + "[C"
DELETE, PAGE = ESC + "[3~", ESC + "[6~"
RUN_REVIEW = "Review one-shot execution permission"
SOURCE_REVIEW = "Review local capture /"
FORM = {"eevee": 0, "vaporeon": 1, "jolteon": 2, "espeon": 4}
TABLES = ("tasks", "partners", "selections", "executions", "active_work", "excerpts",
          "moves", "move_candidates", "move_sources", "move_dispatches", "move_usages",
          "provider_journal", "memories")
JSON_COLUMNS = {"proposal", "record", "steps", "pins", "criteria", "partner_drafts",
                "workflows", "navigation"}
CASES = {
    "menu": "MV02A: empty menu/actions, exact Request/caret/focus restoration, no execution",
    "run-storage": "MV01A/XC07: approved run, restart/form/partner isolation, four quick moves and 32 library capacity",
    "pins": "MV01B/MV02B: three selectors, excluded/missing refusal, actual per-pin review",
    "run-use": "MV02B/MV07: one Enter normal review, denial, real argv/cwd, cancellation, journal counts/restart",
    "repeat-runs": "MV03A/MV03B: exact three sources, argument separation, idle offers, Not-now +3, Never/restart, consent",
    "names": "MV06: three distinct local suggestions, CJK rename, duplicate refusal",
    "reminder": "MV04/MV05: exact scope replacement, Give-up unchanged, restart/relearn identical steps",
    "request": "MV01C/MV02B: actually sent Espeon instruction, full receipt/consent, raw HTTP privacy",
    "task": "MV01C: user goal/criteria, neutral Apply, deliberate Apply resets met without effects",
    "repeat-pins": "MV03C: three real HTTP submissions, reordered identical pin set, one Vaporeon candidate",
    "party": "MV08A: Mizu copy, actual teacher-name snapshot, Bolt unchanged, full-target replacement",
    "purge": "MV05: actual last-task deletion purges project reminders, preserves personal reminder",
    "safety": "Cross-cutting: small terminal, paste/repeat consent, privacy/path refusals, idle and terminal restoration",
}
FIXTURE = r'''import json
import os
from pathlib import Path
import sys
import time

root = Path(__file__).resolve().parent
mode = sys.argv[1]
with (root / "invocations.jsonl").open("a", encoding="utf-8") as handle:
    handle.write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:], "cwd": os.getcwd()}, ensure_ascii=False) + "\n")
print("FIXTURE-START " + mode, flush=True)
if mode == "long":
    deadline = time.monotonic() + 8
    while not (root / "stop").exists() and time.monotonic() < deadline:
        time.sleep(.05)
print("FIXTURE-END " + mode, flush=True)
sys.exit(7 if mode == "fail" or (root.parent / "fail-next").exists() else 0)
'''


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def digest(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


class Journey:
    def __init__(self, binary, output, root, size, ascii_mode, results, peer):
        self.binary, self.output, self.root = binary, output, root
        self.size, self.ascii_mode, self.results, self.peer = size, ascii_mode, results, peer
        self.project, self.home, self.data = (root / p for p in ("project 空 格", "home 漢字", "data 空 格"))
        for p in (self.project, self.home, self.output):
            p.mkdir(parents=True)
        self.database = self.data / "workbench.sqlite3"
        self.sessions, self.session, self.serial = [], None, 0
        self.current_case = None
        self.evidence = []
        self.python = "python" if os.name == "nt" else "python3"
        self.query = ""
        for name in ("alpha.txt", "beta.txt", "資料.txt", "delta.txt", "epsilon.txt", "zeta.txt"):
            (self.project / name).write_text("public fixture " + name + "\nsecond line\n", encoding="utf-8")
        (self.project / "fixture.py").write_text(FIXTURE, encoding="utf-8")
        (self.project / ".env.local").write_text("password=synthetic-local-value\n", encoding="utf-8")
        (self.project / ".gitignore").write_text("invocations.jsonl\nstop\n.env.local\n", encoding="utf-8")
        git = shutil.which("git")
        assert git, "git is required for real source observation"
        self.git = git
        for args in (("init", "--quiet"), ("add", "--", "."),
                     ("-c", "user.name=Local acceptance", "-c", "user.email=local@example.invalid",
                      "commit", "-qm", "public fixture")):
            subprocess.run([git, "-c", "core.autocrlf=false", "-c", "core.hooksPath=", *args],
                           cwd=self.project, check=True, capture_output=True, timeout=15)
        self.environment = terminal_environment("mono")
        for key in list(self.environment):
            if key.startswith(("EV_", "EEVEE_", "OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_",
                               "GEMINI_", "GOOGLE_", "AWS_", "OPENCODE_")) or key.lower().endswith("_proxy"):
                self.environment.pop(key, None)
        tools = root / "tools"
        tools.mkdir()
        if os.name == "nt":
            tool_path = os.pathsep.join(dict.fromkeys((str(Path(sys.executable).parent), str(Path(git).parent))))
        else:
            (tools / "python3").symlink_to(Path(sys.executable).resolve())
            (tools / "git").symlink_to(Path(git).resolve())
            tool_path = str(tools)
        self.environment.update(HOME=str(self.home), USERPROFILE=str(self.home),
            XDG_CONFIG_HOME=str(self.home / "config"), XDG_DATA_HOME=str(self.home / "data"),
            XDG_CACHE_HOME=str(self.home / "cache"), APPDATA=str(self.home / "config"),
            LOCALAPPDATA=str(self.home / "cache"), CODEX_HOME=str(self.home / "codex"),
            GEMINI_CLI_HOME=str(self.home / "gemini"), PATH=tool_path,
            NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        self.launch()
        self.session.command("goal")
        self.session.paste("Local acceptance goal")
        self.session.send("\r")
        self.until(lambda: self.task()["goal"] == "Local acceptance goal", "initial task acknowledgement")
        self.workspace()

    def launch(self):
        self.serial += 1
        self.session = Session([str(self.binary), "--data-dir", str(self.data), "--images", "off",
                                "--color", "mono", *(["--ascii"] if self.ascii_mode else [])],
                               self.project, self.environment, self.size,
                               self.output / ("terminal-%02d" % self.serial))
        self.sessions.append(self.session)
        wait_for(self.session, "Storage: saved", timeout=35)

    def state(self):
        with closing(sqlite3.connect(self.database.as_uri() + "?mode=ro", uri=True, timeout=10)) as db:
            db.row_factory = sqlite3.Row
            available = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            result = {}
            for table in TABLES:
                if table not in available:
                    continue
                rows = [dict(r) for r in db.execute('SELECT * FROM "' + table + '" ORDER BY rowid')]
                for row in rows:
                    for key, value in list(row.items()):
                        if key in JSON_COLUMNS and isinstance(value, str):
                            try:
                                row[key] = json.loads(value)
                            except json.JSONDecodeError:
                                pass
                result[table] = rows
        return result

    def task(self):
        state = self.state()
        active = state["selections"][0]["active_id"]
        return next(row for row in state["tasks"] if row["id"] == active)

    def until(self, predicate, description, timeout=35):
        deadline = time.monotonic() + timeout
        while True:
            if predicate():
                return
            assert self.session.alive(), "application exited: " + description
            if time.monotonic() >= deadline:
                self.capture("timeout")
                raise AssertionError(description + "\n" + self.session.display())
            self.session.pump(.03)

    def quiet_assert(self, predicate, description, seconds=.35):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            assert predicate(), description
            assert self.session.alive(), "application exited: " + description
            self.session.pump(.03)

    def capture(self, label):
        self.serial += 1
        name = "%03d-%s" % (self.serial, label)
        self.session.capture(name)
        state = self.state()
        destination = self.output / (name + "-sqlite.json")
        save(destination, state)
        entry = {"label": label, "screen": str((self.session.directory / (name + ".txt")).relative_to(self.output)),
                 "sqlite": destination.name, "cursor": {"x": self.session.screen.cursor.x,
                 "y": self.session.screen.cursor.y, "hidden": self.session.screen.cursor.hidden},
                 "invocations": self.invocations(), "http_posts": len(self.peer.snapshot("POST"))}
        self.evidence.append(entry)
        if self.current_case:
            self.results[self.current_case]["evidence"].append(entry)
        return state

    def case(self, key, action):
        self.current_case = key
        row = self.results[key]
        row["status"] = "running"
        try:
            action()
            self.capture(key + "-passed")
            row["status"] = "passed"
        except Exception as error:
            row.update(status="failed", error=str(error), traceback=traceback.format_exc())
            try:
                self.capture(key + "-failed")
            except Exception as capture_error:
                row["capture_error"] = repr(capture_error)
            raise
        finally:
            self.current_case = None

    def invocations(self):
        file = self.project / "invocations.jsonl"
        return [json.loads(line) for line in file.read_text(encoding="utf-8").splitlines()] if file.exists() else []

    def effects(self):
        state = self.state()
        return {"invocations": self.invocations(), "executions": state.get("executions", []),
                "posts": self.peer.snapshot("POST")}

    def workspace(self, allow_offer=False):
        # Only known temporary drawers are escaped; never press through an approval.
        for _ in range(12):
            view = self.session.display()
            footer = view.rstrip().splitlines()[-1].strip()
            if footer.startswith(("Type Request |", "Enter Send |", "Draft only |")):
                return footer
            if allow_offer and "wants to learn a move!" in view:
                return "Learn prompt"
            assert RUN_REVIEW not in view and "Run once" not in view, "refusing to dismiss an unknown live approval"
            assert "wants to learn a move!" not in view, "candidate needs an explicit decision"
            self.session.send(ESC)
        raise AssertionError("Request owner not reachable\n" + self.session.display())

    def move_ready(self):
        self.until(lambda: "Loading/saving local move facts" not in self.session.display()
                   and "Saving the reviewed move change" not in self.session.display(), "move storage acknowledgement")
        assert "Move storage failed" not in self.session.display(), self.session.display()

    def form(self, name):
        self.workspace()
        self.session.command(name)
        self.until(lambda: self.task()["form"] == FORM[name], "saved form " + name)
        wait_for(self.session, "Storage: saved")
        self.workspace()

    def lead(self, number):
        self.workspace()
        self.session.command("lead " + str(number))
        self.until(lambda: self.task()["lead_partner"] == number, "saved lead")
        self.workspace()

    def nickname(self, name, allow_offer=False):
        self.session.command("nickname")
        self.replace_editor("x" * 64, name)
        self.session.send("\r")
        self.until(lambda: next(p for p in self.state()["partners"] if p["id"] == self.task()["lead_partner"])["nickname"] == name,
                   "nickname acknowledgement")
        self.workspace(allow_offer=allow_offer)

    def replace_editor(self, previous, text):
        self.session.send(HOME)
        for start in range(0, max(len(previous), 1), 24):
            self.session.send(DELETE * min(24, len(previous) - start))
        self.session.paste(text)

    def moves(self, partner=None, form=None, forgotten=False):
        task = self.task()
        return [r for r in self.state().get("moves", [])
                if r["partner"] == (task["lead_partner"] if partner is None else partner)
                and r["form"] == (task["form"] if form is None else form)
                and bool(r["forgotten"]) == forgotten]

    def menu(self, index=0, details=False):
        self.session.command("move")
        wait_for(self.session, "Moves / local reusable actions")
        self.move_ready()
        self.session.send("\t" * index)
        if details:
            self.session.send("d")
            wait_for(self.session, "Use (normal review)")

    def pages(self, label, count=24):
        observed = [self.session.display()]
        self.capture(label + "-top")
        for index in range(count):
            old = self.session.display()
            self.session.send(PAGE)
            observed.append(self.session.display())
            self.capture(label + "-page-" + str(index + 1))
            if self.session.display() == old:
                break
        self.session.send(HOME)
        return "\n".join(observed)
    def review_local(self):
        for index in range(256):
            self.capture("exact-local-review-%03d" % index)
            if "[Stay]" in self.session.display():
                return
            self.session.send(PAGE)
        raise AssertionError("complete local move review never became confirmable")


    def begin_teach(self, name=None, personal=False):
        self.session.command("teach-move")
        wait_for(self.session, "Move name")
        self.move_ready()
        y = self.session.screen.cursor.y
        suggestion = self.session.display().splitlines()[y].strip(" │|┆")
        if name is not None:
            self.replace_editor("x" * 64, name)
        if personal:
            self.session.send("\t" + RIGHT + "\t")
            wait_for(self.session, "Scope: Personal")
        self.capture("name-editor")
        return suggestion

    def commit_name(self, name=None, personal=False, replacement=None):
        before = self.state().get("moves", [])
        self.begin_teach(name, personal)
        self.session.send("\r")
        if replacement is not None:
            wait_for(self.session, "Give up learning")
            self.session.send(DOWN * (replacement + 1) + "\r")
        wait_for(self.session, "Confirm local change")
        self.capture("local-neutral-review")
        assert self.state().get("moves", []) == before, "preview changed stored moves"
        self.review_local()
        self.session.send("y")
        self.until(lambda: self.state().get("moves", []) != before, "explicit local move save")
        self.move_ready()
        self.workspace()
        return self.moves()[-1]

    def argv(self, mode="once", *args):
        return [self.python, "fixture.py", mode, *args]

    def edit_command(self, argv):
        self.workspace()
        previous = self.task()["command"]
        value = json.dumps(argv, ensure_ascii=True)
        self.session.command("command-edit")
        wait_for(self.session, "Command / explicit JSON argv or shell")
        self.replace_editor(previous, value)
        self.session.send("\r")
        self.until(lambda: self.task()["command"] == value, "exact JSON argv saved")

    def run(self, mode="once", *args, review_only=False):
        self.edit_command(self.argv(mode, *args))
        self.session.command("execute")
        wait_for(self.session, RUN_REVIEW, timeout=35)
        assert "wants to learn a move!" not in self.session.display(), "candidate interrupted approval"
        if review_only:
            return
        self.approve_run()

    def approve_run(self, expected="Succeeded"):
        count = len(self.state()["executions"])
        self.session.send(RIGHT + "\r")
        self.until(lambda: len(self.state()["executions"]) == count + 1
                   and self.state()["executions"][-1]["record"]["state"] == expected
                   and not self.state()["executions"][-1]["reserved"], "actual owned run terminal journal")
        wait_for(self.session, "Local command evidence; no command running", timeout=35)
        assert "wants to learn a move!" not in self.session.display(), "candidate interrupted execution drawer"
        return self.state()["executions"][-1]["record"]

    def use_run(self, index=0, deny=False, expected="Succeeded"):
        before = self.effects()
        self.menu(index)
        self.session.send("\r")
        wait_for(self.session, RUN_REVIEW, timeout=35)
        assert self.effects() == before, "one Enter on move menu executed instead of reviewing"
        self.capture("move-normal-run-review")
        if deny:
            self.session.command("deny")
            wait_for(self.session, "Request denied")
            self.until(lambda: any(r["record"]["outcome"] == "Denied" for r in self.state()["move_usages"]), "denial usage save")
            assert self.effects() == before, "denial executed a command"
            return
        return self.approve_run(expected)

    def quit(self):
        self.session.command("quit")
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0, "native binary returned nonzero"
        self.session.close()

    def restart(self):
        before = self.effects()
        self.quit()
        self.launch()
        self.workspace()
        assert self.effects() == before, "restart replayed work"

    def prepare_connection(self):
        footer = self.workspace()
        if footer.startswith("Type Request |"):
            self.session.send("\r")
        revision = self.task()["revision"]
        # Workspace navigation and an earlier task row do not acknowledge the
        # current editor's committed state. Explicitly save the exact Request
        # and caret before setup's idle-only gate, without replaying Send.
        self.session.send("\x13")
        self.until(lambda: self.task()["revision"] > revision
                   and "Storage: saved" in self.session.display()
                   and not self.state()["active_work"],
                   "current Request and caret committed before connection setup")

    def connect(self):
        before = len(self.peer.snapshot("POST"))
        self.prepare_connection()
        self.session.command("connect compatible")
        wait_for(self.session, "API key:")
        self.session.send("\t\x15")
        self.session.paste(self.peer.base)
        self.session.send("\t\x15")
        self.session.paste(MODEL)
        self.session.send("\t\t ")  # Chat-completions default, explicitly choose keyless authentication.
        self.session.send("\t \t")  # Explicit session-only storage; no fixture file survives exit.
        # Save refuses while the keyless toggle is still saving; wait, don't race it.
        wait_for(self.session, "> Save & choose model")
        self.until(lambda: "Storage: saved" in self.session.display(), "connection edit saved before Save & choose")
        self.session.send("\r")
        wait_for(self.session, MODEL, timeout=35)
        wait_for(self.session, "1 matches")
        self.session.send(MODEL + "\r")
        self.workspace()
        assert len(self.peer.snapshot("POST")) == before, "connection setup dispatched inference"

    def write_request(self, text):
        footer = self.workspace()
        if footer.startswith("Type Request |"):
            self.session.send("\r")
        self.session.send(UP * 16 + HOME)
        for _ in range(12):
            self.session.send(DELETE * 24)
        self.session.paste(text)
        self.session.send("\x13")
        def saved():
            drafts = self.task()["partner_drafts"].get(str(self.task()["lead_partner"]), {}).get("drafts", [])
            return any(d["field"] == "Request" and d["text"] == text for d in drafts) if text else not any(
                d["field"] == "Request" and d["text"] for d in drafts)
        self.until(saved, "exact Request retained draft")
        # The app acknowledges the durable write after SQLite shows it.
        self.until(lambda: "Storage: saved" in self.session.display(), "Request save acknowledged")

    def send_request(self, text, answer, mode="complete"):
        self.write_request(text)
        before = len(self.peer.snapshot("POST"))
        self.peer.plans.put((mode, answer))
        self.session.send("\r")
        self.until(lambda: len(self.peer.snapshot("POST")) == before + 1, "real native HTTP submission")
        if mode == "complete":
            wait_for(self.session, answer, timeout=35)
            self.until(lambda: not self.state()["active_work"] and "Storage: saved" in self.session.display(), "native HTTP settlement")
        return self.peer.snapshot("POST")[-1]

    def pin(self, filename):
        self.workspace()
        before = len(self.state()["excerpts"])
        self.session.command("files")
        wait_for(self.session, "eligible files /", timeout=35)
        self.session.command("search")
        wait_for(self.session, "Filename filter")
        self.replace_editor(self.query, filename)
        self.session.send("\r")
        self.query = filename
        wait_for(self.session, "1 eligible files /", timeout=35)
        self.session.send("\r")
        wait_for(self.session, SOURCE_REVIEW, timeout=35)
        assert len(self.state()["excerpts"]) == before, "inspection implicitly attached source"
        self.capture("source-before-explicit-attach")
        self.session.command("attach")
        self.until(lambda: len(self.state()["excerpts"]) == before + 1, "actual source attachment saved")
        self.workspace()
        return self.state()["excerpts"][-1]

    def unpin_all(self):
        self.workspace()
        task_id = self.task()["id"]
        while any(r["task_id"] == task_id for r in self.state()["excerpts"]):
            count = sum(r["task_id"] == task_id for r in self.state()["excerpts"])
            self.session.command("files")
            wait_for(self.session, "eligible files /", timeout=35)
            self.session.command("basket")
            wait_for(self.session, "stored")
            self.session.command("unpin")
            wait_for(self.session, "Review local excerpt removal")
            self.session.send(RIGHT + "\r")
            self.until(lambda: sum(r["task_id"] == task_id for r in self.state()["excerpts"]) == count - 1,
                       "source unpin acknowledgement")
            self.workspace()

    def close(self):
        # Cooperative stop bounds commands even if the app detached a process tree.
        (self.project / "stop").touch()
        for session in self.sessions:
            if not session.closed:
                try:
                    session.capture("failure-cleanup")
                finally:
                    session.close()
        processes = []
        for row in self.invocations():
            try:
                processes.append(NativeProcess(row["pid"]))
            except OSError:
                if os.name != "nt":
                    raise
                # Windows cannot open an already-exited process.
        try:
            deadline = time.monotonic() + 9
            while any(process.alive() for process in processes) and time.monotonic() < deadline:
                time.sleep(.05)
            assert not any(process.alive() for process in processes), "fixture process survived cooperative stop and cleanup deadline"
        finally:
            for process in processes:
                process.close()
        save(self.output / "observations.json", self.evidence)

    def menu_case(self):
        self.write_request("retained 漢字 Request /move\nnot a command")
        self.session.send(UP + HOME + RIGHT * 3)
        before = self.effects()
        caret = (self.session.screen.cursor.x, self.session.screen.cursor.y, self.session.screen.cursor.hidden)
        self.menu()
        wait_for(self.session, "No moves in this form yet")
        self.capture("empty-menu")
        self.session.send("\r")
        assert self.effects() == before
        self.session.send(ESC)
        after = (self.session.screen.cursor.x, self.session.screen.cursor.y, self.session.screen.cursor.hidden)
        assert after == caret, ("Request caret/focus changed", caret, after)
        assert any(d["field"] == "Request" and d["text"] == "retained 漢字 Request /move\nnot a command"
                   for d in self.task()["partner_drafts"].get(str(self.task()["lead_partner"]), {}).get("drafts", [])), "menu changed exact retained Request"
        self.session.send("Z")
        self.session.send("\x13")
        self.until(lambda: any("Z" in d["text"] for d in self.task()["partner_drafts"].get(str(self.task()["lead_partner"]), {}).get("drafts", []) if d["field"] == "Request"), "restored editor receives literal input")
        self.session.send("\x0b")
        wait_for(self.session, "/ Commands")
        self.session.send("move")
        assert "move" in self.session.display().lower(), "actions does not expose move entry"
        self.session.send(ESC)
        assert self.effects() == before
        self.write_request("")

    def storage_case(self):
        self.form("jolteon")
        self.nickname("Bolt")
        self.run("manual", "東京 空 格")
        self.workspace()
        self.first_move = self.commit_name("Manual label marker 7F2", personal=True)
        steps = {"Run": {"argv": self.argv("manual", "東京 空 格"), "cwd": "."}}
        assert self.first_move["proposal"]["steps"] == steps, self.first_move
        self.restart()
        self.menu()
        wait_for(self.session, "Manual label marker 7F2")
        self.workspace()
        self.session.command("forms")
        wait_for(self.session, "Manual label marker 7F2")
        self.capture("form-picker-saved-move-inventory")
        self.workspace()
        self.form("eevee")
        self.menu()
        wait_for(self.session, "No moves in this form yet")
        self.workspace()
        self.form("jolteon")
        assert self.moves()[0]["proposal"]["steps"] == steps
        self.session.command("party")
        self.session.send("a")
        self.until(lambda: len(self.state()["partners"]) == 2, "party add saved")
        self.workspace()
        self.lead(2)
        self.nickname("Mizu")
        self.form("jolteon")
        self.menu()
        wait_for(self.session, "No moves in this form yet")
        assert not self.moves()
        self.workspace()
        self.lead(1)

    def names_case(self):
        suggestions = []
        for number in range(3):
            self.run("naming", str(number))
            self.workspace()
            before = len(self.moves())
            suggestions.append(self.begin_teach())
            self.session.send("\r")
            wait_for(self.session, "Confirm local change")
            self.review_local()
            self.session.send("y")
            self.until(lambda: len(self.moves()) == before + 1, "default local name saved")
            self.move_ready()
            self.workspace()
        assert len(set(suggestions)) == 3 and all(suggestions), suggestions
        self.results["names"]["suggestions"] = suggestions
        self.menu(0, details=True)
        self.session.send("r")
        wait_for(self.session, "Rename (label only)")
        self.replace_editor("x" * 64, "雷光の手順")
        self.session.send("\r")
        wait_for(self.session, "Confirm local change")
        self.review_local()
        self.session.send("y")
        self.until(lambda: self.moves()[0]["proposal"]["name"] == "雷光の手順", "CJK rename save")
        self.move_ready()
        self.workspace()
        before = self.moves()
        self.menu(1, details=True)
        self.session.send("r")
        self.replace_editor("x" * 64, "雷光の手順")
        self.session.send("\r")
        wait_for(self.session, "already has that name")
        assert self.moves() == before, "duplicate rename changed moves"
        self.workspace()
        self.session.command("forms")
        for record in self.moves():
            wait_for(self.session, record["proposal"]["name"])
        self.capture("form-picker-four-distinct-moves")
        self.workspace()

    def reminder_case(self):
        for number in range(4, 32):
            self.commit_name("Library entry %02d" % number)
            self.workspace()
        before = self.moves()
        self.begin_teach("Fifth candidate")
        self.session.send("\r")
        wait_for(self.session, "Give up learning")
        self.capture("fifth-four-exact-slots")
        self.pages("full-slot-scope")
        self.session.send("\r")
        assert self.moves() == before, "neutral Give up learning changed slots"
        self.workspace()
        old = self.moves()[0]
        self.begin_teach("Replacement fifth")
        self.session.send("\r")
        wait_for(self.session, "Give up learning")
        self.session.send(DOWN + "\r")
        wait_for(self.session, "Forgets Personal")
        self.capture("replacement-exact-personal-scope")
        assert self.moves() == before
        self.review_local()
        self.session.send("y")
        self.until(lambda: any(r["id"] == old["id"] for r in self.moves(forgotten=True)), "exact replaced move forgotten")
        self.move_ready()
        self.workspace()
        self.restart()
        self.session.command("move-reminder")
        self.move_ready()
        wait_for(self.session, "雷光の手順")
        self.capture("forgotten-restart")
        self.session.send("\r")
        wait_for(self.session, "Give up learning")
        self.session.send(DOWN + "\r")
        wait_for(self.session, "Relearn identical steps")
        self.review_local()
        self.session.send("y")
        self.until(lambda: any(r["id"] == old["id"] for r in self.moves()), "identical relearn save")
        restored = next(r for r in self.moves() if r["id"] == old["id"])
        assert restored["proposal"] == old["proposal"], "relearn changed steps/name/scope/provenance"
        self.move_ready()
        self.workspace()
        self.run_move_id = restored["id"]

    def run_use_case(self):
        index = next(i for i, r in enumerate(self.moves()) if r["id"] == self.run_move_id)
        self.use_run(index, deny=True)
        self.workspace()
        run = self.use_run(index)
        assert run["prepared"]["spec"]["Direct"] == {"program": self.python, "args": ["fixture.py", "manual", "東京 空 格"]}, run
        assert Path(run["prepared"]["cwd"]).samefile(self.project)
        assert self.invocations()[-1]["argv"] == ["manual", "東京 空 格"]
        assert Path(self.invocations()[-1]["cwd"]).samefile(self.project)
        self.until(lambda: any(r["record"]["move_id"] == self.run_move_id and r["record"]["outcome"] == "Succeeded" for r in self.state()["move_usages"]), "journal-linked move use outcome")
        self.workspace()
        failure_switch = self.root / "fail-next"
        failure_switch.touch()
        failed_run = self.use_run(index, expected="Failed")
        failure_switch.unlink()
        assert failed_run["exit_code"] == 7
        self.until(lambda: any(r["record"]["move_id"] == self.run_move_id and r["record"]["outcome"] == "Failed"
                              for r in self.state()["move_usages"]), "actual failed move outcome")
        usages = [r["record"] for r in self.state()["move_usages"] if r["record"]["move_id"] == self.run_move_id]
        assert len(usages) == 3 and {u["outcome"] for u in usages} == {"Succeeded", "Failed", "Denied"}, usages
        assert next(u for u in usages if u["outcome"] == "Succeeded")["run_id"] == run["prepared"]["run_id"]
        self.workspace()
        self.restart()
        assert [r["record"] for r in self.state()["move_usages"] if r["record"]["move_id"] == self.run_move_id] == usages
        self.menu(index, details=True)
        details = self.pages("journal-counts-and-revision")
        assert "Uses: 3 / 1/2 exited 0" in details and "Denied: 1" in details, details
        journals = {r["record"]["prepared"]["run_id"]: r["record"] for r in self.state()["executions"]}
        for usage in usages:
            if usage["run_id"]:
                journal = journals[usage["run_id"]]
                expected_revision = journal["after"]["fingerprint"] or journal["prepared"]["before"]["fingerprint"]
                assert usage["revision"] == expected_revision, (usage, journal)
        self.workspace()
        # One real bounded long command is approved, then the taught move is cancelled.
        self.run("long")
        self.workspace()
        long_move = self.commit_name("Long cancellation", replacement=0)
        self.menu(next(i for i, r in enumerate(self.moves()) if r["id"] == long_move["id"]))
        self.session.send("\r")
        wait_for(self.session, RUN_REVIEW)
        self.session.send(RIGHT + "\r")
        self.until(lambda: self.state()["executions"][-1]["record"]["state"] == "Running", "owned long command running")
        assert "wants to learn a move!" not in self.session.display()
        self.session.command("cancel")
        self.until(lambda: self.state()["executions"][-1]["record"]["state"] == "Cancelled" and not self.state()["executions"][-1]["reserved"], "actual command cancellation journal")
        self.until(lambda: any(r["record"]["move_id"] == long_move["id"] and r["record"]["outcome"] == "Cancelled" for r in self.state()["move_usages"]), "cancelled move use saved")
        self.workspace()

    def party_case(self):
        bolt = self.moves()
        source = bolt[0]
        self.menu(0, details=True)
        self.session.send("t")
        wait_for(self.session, "Mizu")
        self.session.send("\r")
        wait_for(self.session, "Move name")
        self.move_ready()
        self.session.send("\r")
        wait_for(self.session, "Confirm local change")
        self.review_local()
        self.session.send("y")
        self.until(lambda: len(self.moves(partner=2)) == 1, "Mizu copy acknowledgement")
        copied = self.moves(partner=2)[0]
        assert copied["proposal"]["steps"] == source["proposal"]["steps"]
        assert copied["proposal"]["provenance"]["taught_by"] == 1
        assert copied["proposal"]["provenance"]["teacher_name"] == "Bolt", copied
        assert self.moves(partner=1) == bolt, "source changed while copying"
        self.move_ready()
        self.workspace()
        self.nickname("Bolt改名", allow_offer=True)
        # Three approved manual uses remain an eligible project candidate
        # after its personal move was forgotten. Settle that real idle offer
        # explicitly before the independent full-target teaching checks.
        manual_steps = {"Run": {"argv": self.argv("manual", "東京 空 格"), "cwd": "."}}
        wait_for(self.session, "wants to learn a move!")
        assert any(r["record"]["proposal"]["steps"] == manual_steps and r["record"]["uses"] == 3
                   for r in self.state()["move_candidates"])
        self.capture("forgotten-used-routine-idle-offer")
        self.session.send(DOWN * 2 + "\r")
        self.until(lambda: any(r["record"]["proposal"]["steps"] == manual_steps and r["never"]
                               for r in self.state()["move_candidates"]), "explicit unused-routine Never")
        self.workspace()
        assert self.moves(partner=2)[0]["proposal"]["provenance"]["teacher_name"] == "Bolt", "teacher snapshot changed with nickname"
        for number in range(31):
            self.menu(0, details=True)
            self.session.send("t\r")
            wait_for(self.session, "Move name")
            self.move_ready()
            self.replace_editor("x" * 64, "Target slot " + str(number))
            self.session.send("\r")
            wait_for(self.session, "Confirm local change")
            self.review_local()
            self.session.send("y")
            self.until(lambda: len(self.moves(partner=2)) == number + 2, "target slot filled")
            self.move_ready()
            self.workspace()
        target_before = self.moves(partner=2)
        self.menu(0, details=True)
        self.session.send("t\r")
        wait_for(self.session, "Move name")
        self.move_ready()
        self.replace_editor("x" * 64, "Target fifth")
        self.session.send("\r")
        wait_for(self.session, "Give up learning")
        self.capture("full-target-exact-replacement")
        assert self.moves(partner=2) == target_before and self.moves(partner=1) == bolt
        self.session.send("\r")
        assert self.moves(partner=2) == target_before
        self.workspace()
        self.menu(0, details=True)
        self.session.send("t\r")
        wait_for(self.session, "Move name")
        self.move_ready()
        self.replace_editor("x" * 64, "Target fifth")
        self.session.send("\r")
        wait_for(self.session, "Give up learning")
        self.session.send(DOWN + "\r")
        wait_for(self.session, "Confirm local change")
        self.review_local()
        self.session.send("y")
        self.until(lambda: any(r["proposal"]["name"] == "Target fifth" for r in self.moves(partner=2)), "full target replacement acknowledged")
        assert self.moves(partner=1) == bolt and len(self.moves(partner=2)) == 32
        self.move_ready()
        self.workspace()

    def safety_case(self):
        before = self.effects()
        self.begin_teach("Consent safety")
        self.session.paste("\ny /move-use\n")
        wait_for(self.session, "Name insertion refused")
        assert self.effects() == before
        self.replace_editor("x" * 64, "Consent safety")
        self.session.send("\r")
        wait_for(self.session, "Give up learning")
        self.session.send(DOWN + "\r")
        wait_for(self.session, "Confirm local change")
        before_moves = self.state()["moves"]
        self.session.resize((59, 17))
        wait_for(self.session, "Moves: need 60 x 18")
        self.session.send("y" + RIGHT + "\r")
        self.quiet_assert(lambda: self.state()["moves"] == before_moves and self.effects() == before, "below-minimum consent changed facts")
        self.session.resize(self.size)
        wait_for(self.session, "Confirm local change")
        self.session.paste("y\n")
        self.session.send(ESC + "[121;1:2u")  # Kitty repeat event; raw bytes retained, no synthetic reducer call.
        self.quiet_assert(lambda: self.state()["moves"] == before_moves, "paste/repeat committed a local change")
        self.session.send("\r")
        assert self.state()["moves"] == before_moves, "neutral local Enter committed a change"
        self.workspace()
        self.run("below-size-review", review_only=True)
        review_effects = self.effects()
        self.session.resize((59, 17))
        self.session.send(RIGHT + "\r" + "y")
        self.quiet_assert(lambda: self.effects() == review_effects, "below-minimum normal run approval executed")
        self.session.resize(self.size)
        wait_for(self.session, RUN_REVIEW)
        self.session.command("deny")
        wait_for(self.session, "Request denied")
        self.workspace()
        # Exercise actual approved producer inputs without asking storage to
        # retain recognizable credentials. Secret/masked argv are separately
        # covered by the typed-domain and committed-store admission regressions.
        outside = self.root / "outside.txt"
        outside.write_text("outside-project public fixture\n", encoding="utf-8")
        for argument in ("FOO=visible-fixture", str(outside)):
            for _ in range(3):
                self.run("privacy", argument)
                self.workspace()
            forbidden_steps = {"Run": {"argv": self.argv("privacy", argument), "cwd": "."}}
            assert not any(r["record"]["proposal"]["steps"] == forbidden_steps
                           for r in self.state()["move_candidates"]), "unsafe repeated run became a candidate"
            self.session.command("teach-move")
            self.quiet_assert(lambda: self.state()["moves"] == before_moves, "unsafe approved command taught")
            assert "Move name" not in self.session.display(), "unsafe approved command entered teaching"
            self.capture("approved-ineligible-command-teaching-refused")
            self.workspace()
        (self.project / "escape.txt").symlink_to(outside)
        self.session.command("files")
        wait_for(self.session, "eligible files /", timeout=35)
        self.session.command("refresh")
        wait_for(self.session, "eligible files /", timeout=35)
        self.session.command("search")
        wait_for(self.session, "Filename filter")
        self.replace_editor(self.query, "escape.txt")
        self.session.send("\r")
        self.query = "escape.txt"
        wait_for(self.session, "0 eligible files /", timeout=35)
        self.capture("out-of-project-source-path-refused")
        self.workspace()
        self.menu()
        self.workspace()
        wait_for(self.session, "Storage: saved")
        self.session.pump(.3)
        self.session.idle()
        self.quit()

    def candidate(self, mode):
        return [r for r in self.state()["move_candidates"]
                if r["record"]["proposal"]["steps"] == {"Run": {"argv": self.argv(mode), "cwd": "."}}]

    def repeat_case(self):
        self.form("jolteon")
        self.nickname("Bolt")
        for number in range(3):
            self.run("repeat")
            self.capture("repeat-run-" + str(number + 1))
            if number < 2:
                assert not self.candidate("repeat"), "candidate offered before three actual runs"
            self.workspace() if number < 2 else None
        self.until(lambda: len(self.candidate("repeat")) == 1, "third run candidate persisted")
        candidate = self.candidate("repeat")[0]
        assert candidate["record"]["uses"] == 3 and len(candidate["record"]["sources"]) == 3
        journals = {"run:" + r["record"]["prepared"]["run_id"] for r in self.state()["executions"]}
        assert set(candidate["record"]["sources"]) <= journals
        assert not self.moves(), "candidate auto-learned"
        assert "wants to learn a move!" not in self.session.display(), "offer appeared over execution review"
        self.workspace(allow_offer=True)
        wait_for(self.session, "wants to learn a move!")
        self.pages("three-actual-run-sources")
        self.session.send(ESC)
        self.until(lambda: self.candidate("repeat")[0]["threshold"] == 6, "Escape durably Not now +3")
        self.workspace()
        self.restart()
        assert self.candidate("repeat")[0]["threshold"] == 6
        self.run("different-argument")
        self.workspace()
        assert not self.candidate("different-argument") and self.candidate("repeat")[0]["record"]["uses"] == 3
        for count in (4, 5, 6):
            self.run("repeat")
            self.until(lambda: self.candidate("repeat")[0]["record"]["uses"] == count, "next actual run candidate observation")
            if count < 6:
                self.workspace()
                self.quiet_assert(lambda: "wants to learn a move!" not in self.session.display(), "Not now returned before exact +3")
        self.workspace(allow_offer=True)
        wait_for(self.session, "wants to learn a move!")
        self.session.send(DOWN * 2 + "\r")
        self.until(lambda: self.candidate("repeat")[0]["never"] == 1, "Never durable tombstone")
        self.workspace()
        self.restart()
        self.run("repeat")
        self.workspace()
        self.quiet_assert(lambda: "wants to learn a move!" not in self.session.display(), "Never resurfaced after restart")
        for number in range(3):
            self.run("learn-candidate")
            if number < 2:
                self.workspace()
        self.workspace(allow_offer=True)
        wait_for(self.session, "wants to learn a move!")
        self.session.send(DOWN + "\r")
        wait_for(self.session, "Move name")
        assert not self.moves(), "Learn accepted without name/local review"
        self.replace_editor("x" * 64, "Learned exact candidate")
        self.session.send("\r")
        wait_for(self.session, "Confirm local change")
        assert not self.moves()
        self.session.send("\r")
        assert not self.moves(), "neutral Enter learned candidate"
        self.workspace()
        self.session.command("move-reminder")
        self.move_ready()
        wait_for(self.session, "postponed")
        self.session.send("\r")
        wait_for(self.session, "wants to learn a move!")
        self.session.send(DOWN + "\r")
        wait_for(self.session, "Move name")
        self.replace_editor("x" * 64, "Learned exact candidate")
        self.session.send("\r")
        wait_for(self.session, "Confirm local change")
        self.session.send("y")
        self.until(lambda: len(self.moves()) == 1, "explicit candidate learning")
        self.move_ready()
        self.workspace()
        assert self.moves()[0]["proposal"]["steps"] == {"Run": {"argv": self.argv("learn-candidate"), "cwd": "."}}
        self.quit()

    def pins_case(self):
        self.form("vaporeon")
        selectors = []
        for filename in ("alpha.txt", "beta.txt", "資料.txt"):
            selectors.append(self.pin(filename))
        move = self.commit_name("Three paths")
        assert len(move["proposal"]["steps"]["Pins"]["pins"]) == 3
        self.menu(0, details=True)
        self.pages("three-pin-paths-preview")
        self.workspace()
        self.session.command("files")
        wait_for(self.session, "eligible files /", timeout=35)
        self.session.command("search")
        wait_for(self.session, "Filename filter")
        self.replace_editor(self.query, ".env.local")
        self.session.send("\r")
        self.query = ".env.local"
        wait_for(self.session, "0 eligible files /", timeout=35)
        self.capture("excluded-source-refused")
        self.workspace()
        self.unpin_all()
        self.menu()
        effects = self.effects()
        self.session.send("\r")
        for index, pin in enumerate(move["proposal"]["steps"]["Pins"]["pins"]):
            wait_for(self.session, SOURCE_REVIEW, timeout=35)
            wait_for(self.session, pin["path"])
            assert len(self.state()["excerpts"]) == index, "move pin auto-attached"
            self.capture("move-pin-" + str(index) + "-explicit-review")
            self.session.command("attach")
            self.until(lambda: len(self.state()["excerpts"]) == index + 1, "move pin durable attachment")
        self.until(lambda: any(r["record"]["move_id"] == move["id"] and r["record"]["outcome"] == "Completed" for r in self.state()["move_usages"]), "pin-set local completion")
        assert self.effects() == effects, "pin-set use executed or sent work"
        self.workspace()
        # Remove only our disposable source, then require honest unavailable evidence.
        (self.project / "alpha.txt").unlink()
        self.session.command("refresh")
        wait_for(self.session, "Storage: saved")
        self.menu()
        self.session.send("\r")
        self.until(lambda: any(word in self.session.display().lower() for word in ("unavailable", "missing", "cannot read", "not found")), "missing move pin refusal")
        self.capture("missing-pin-use-refused")
        assert self.effects() == effects
        assert len(self.state()["excerpts"]) == 3, "missing pin implicitly attached a new capture"
        self.session.send(ESC)
        self.workspace()
        self.quit()

    def request_case(self):
        self.connect()
        self.form("espeon")
        memory_marker = "APPROVED_UNSELECTED_MEMORY_NEVER_MINED_19D"
        self.session.command("memory")
        wait_for(self.session, "Approved memories")
        self.session.command("create-memory")
        self.session.paste(memory_marker)
        self.session.send("\r")
        wait_for(self.session, "Approve")
        self.session.send("y")
        self.until(lambda: len(self.state()["memories"]) == 1, "actual approved unselected memory")
        self.workspace()
        self.sent = "USER_INSTRUCTION_ONLY_86C: explain this public fixture concisely."
        self.output_marker = "PROVIDER_OUTPUT_NEVER_TEACH_B92"
        first = self.send_request(self.sent, self.output_marker)
        self.write_request("UNSENT_CHANGED_DRAFT_62A")
        before = self.effects()
        move = self.commit_name("PRIVATE_LABEL_NOT_USER_TEXT_C41")
        assert move["proposal"]["steps"] == {"Request": {"instruction": self.sent, "pins": []}}, move
        assert self.output_marker not in json.dumps(move) and "UNSENT_CHANGED" not in json.dumps(move)
        assert memory_marker not in json.dumps(move), "approved memory mined into taught request"
        self.request_move = move
        self.menu()
        self.session.send("\r")
        wait_for(self.session, "Request draft retained")
        assert self.effects() == before
        self.workspace()
        self.write_request("")
        self.menu()
        self.session.send("\r")
        wait_for(self.session, "Run once", timeout=35)
        self.session.send("d")
        self.pages("request-full-payload-receipt")
        self.session.send("d")
        wait_for(self.session, "Run once")
        assert self.effects() == before, "request move sent before receipt approval"
        self.session.send("\r")
        assert self.effects() == before, "neutral request review dispatched"
        self.workspace()
        self.write_request("")
        self.menu()
        self.session.send("\r")
        wait_for(self.session, "Run once", timeout=35)
        self.peer.plans.put(("complete", "SECOND_REQUEST_COMPLETED"))
        self.session.send(RIGHT + "\r")
        self.until(lambda: len(self.peer.snapshot("POST")) == 2, "explicit move native HTTP submission")
        wait_for(self.session, "SECOND_REQUEST_COMPLETED", timeout=35)
        self.until(lambda: not self.state()["active_work"], "request move ownership settled")
        raw = self.peer.snapshot("POST")[-1]
        policy, messages = decoded_payload("chat_completions", raw["body"])
        encoded = json.dumps(raw["body"], ensure_ascii=False)
        assert move["proposal"]["name"] not in encoded, "move label leaked into HTTP"
        assert "Taught by" not in encoded and "taught_by" not in encoded and "teacher_name" not in encoded
        assert "UNSENT_CHANGED_DRAFT_62A" not in encoded
        assert messages[-1] == ("user", self.sent), messages
        assert memory_marker not in encoded, "unselected memory entered move HTTP packet"
        assert self.output_marker not in messages[-1][1] and self.output_marker not in policy
        assert decoded_payload("chat_completions", first["body"])[1][-1] == ("user", self.sent)
        self.results["request"]["privacy"] = {"last_user": messages[-1], "earlier_assistant_history_allowed": True,
            "move_labels_in_packet": False, "provider_output_in_taught_instruction": False,
            "account_proof": False, "transport": "actual keyless loopback HTTP/SSE"}
        self.workspace()

    def repeat_pins_case(self):
        self.form("vaporeon")
        for index, order in enumerate((("delta.txt", "epsilon.txt", "zeta.txt"),
                                      ("zeta.txt", "delta.txt", "epsilon.txt"),
                                      ("epsilon.txt", "zeta.txt", "delta.txt"))):
            self.unpin_all()
            for filename in order:
                self.pin(filename)
            before = len(self.peer.snapshot("POST"))
            self.send_request("PIN_SET_DISPATCH_%d public instruction" % index, "PIN_SET_RESPONSE_%d" % index)
            assert len(self.peer.snapshot("POST")) == before + 1
            self.capture("actual-pin-set-submission-" + str(index + 1))
            if index < 2:
                assert not [r for r in self.state()["move_candidates"] if r["form"] == 1], "pin candidate before third dispatch"
        self.until(lambda: len([r for r in self.state()["move_candidates"] if r["form"] == 1]) == 1,
                   "reordered identical pin set single candidate")
        candidate = next(r for r in self.state()["move_candidates"] if r["form"] == 1)
        assert candidate["record"]["uses"] == 3 and len(candidate["record"]["sources"]) == 3
        assert len(set(candidate["record"]["sources"])) == 3
        assert candidate["record"]["proposal"]["form"] == 1
        dispatch_ids = {"request:" + r["id"] for r in self.state()["move_dispatches"]}
        assert set(candidate["record"]["sources"]) <= dispatch_ids
        self.workspace()
        self.write_request("")
        wait_for(self.session, "wants to learn a move!", timeout=35)
        self.pages("three-real-pin-dispatch-sources")
        self.session.send(ESC)
        self.workspace()
        self.quit()

    def task_case(self):
        self.form("eevee")
        self.session.command("criteria")
        self.session.command("criteria-edit")
        self.session.paste("User criterion alpha\nUser criterion beta")
        self.session.send("\r")
        self.until(lambda: [c["text"] for c in self.task()["criteria"]] == ["User criterion alpha", "User criterion beta"], "saved authored criteria")
        self.workspace()
        task_move = self.commit_name("Goal and criteria template")
        exact = {"Task": {"goal": "Local acceptance goal", "criteria": ["User criterion alpha", "User criterion beta"]}}
        assert task_move["proposal"]["steps"] == exact
        self.task_move = task_move
        self.session.command("goal")
        self.replace_editor(self.task()["goal"], "Different goal before Apply")
        self.session.send("\r")
        self.until(lambda: self.task()["goal"] == "Different goal before Apply", "changed task goal")
        self.workspace()
        self.session.command("criteria")
        self.session.send("\r")
        self.until(lambda: self.task()["criteria"][0]["met"], "criterion met toggle saved")
        self.workspace()
        before, effects = self.task(), self.effects()
        self.menu()
        self.session.send("\r")
        wait_for(self.session, "Apply template")
        self.pages("user-authored-task-template")
        self.session.send("\r")
        assert self.task()["goal"] == before["goal"] and self.task()["criteria"] == before["criteria"]
        assert self.effects() == effects
        self.workspace()
        self.menu()
        self.session.send("\r")
        wait_for(self.session, "Apply template")
        self.session.send("y")
        self.until(lambda: self.task()["goal"] == exact["Task"]["goal"] and all(not c["met"] for c in self.task()["criteria"]), "deliberate local task template Apply")
        self.until(lambda: any(r["record"]["move_id"] == task_move["id"] and r["record"]["outcome"] == "Completed" for r in self.state()["move_usages"]), "task Completed after save acknowledgement")
        assert self.effects() == effects
        self.workspace()

    def purge_case(self):
        self.menu(0, details=True)
        self.session.send("f")
        wait_for(self.session, "Forget Project")
        self.session.send("y")
        self.until(lambda: len(self.moves(forgotten=True)) == 1, "project reminder saved")
        self.move_ready()
        self.workspace()
        personal = self.commit_name("Personal preserved reminder", personal=True)
        self.menu(0, details=True)
        self.session.send("f")
        wait_for(self.session, "Forget Personal")
        self.session.send("y")
        self.until(lambda: len(self.moves(forgotten=True)) == 2, "personal reminder saved")
        self.move_ready()
        self.workspace()
        self.restart()
        self.session.command("move-reminder")
        self.move_ready()
        self.pages("both-scope-reminders-before-purge")
        self.workspace()
        assert len(self.state()["tasks"]) == 1, "purge fixture must delete its actual last task"
        self.session.command("tasks")
        wait_for(self.session, "Task cards / committed snapshots")
        self.session.send(HOME)
        self.session.command("delete")
        wait_for(self.session, "Review local task deletion")
        self.capture("last-task-purge-neutral")
        self.session.send("\r")
        assert len(self.state()["tasks"]) == 1, "neutral task deletion deleted project"
        self.session.command("delete")
        wait_for(self.session, "Review local task deletion")
        self.session.send(END + RIGHT + "\r")
        self.until(lambda: not self.state()["tasks"], "real last-task deletion committed")
        remaining = self.state()["moves"]
        assert len(remaining) == 1 and remaining[0]["id"] == personal["id"] and remaining[0]["forgotten"], remaining
        assert remaining[0]["proposal"] == personal["proposal"]
        assert not self.state()["move_candidates"] and not self.state()["move_sources"]
        self.quit()
        self.launch()
        self.session.command("goal")
        self.session.paste("New task after project purge")
        self.session.send("\r")
        self.until(lambda: bool(self.state()["tasks"]), "new task after purge")
        self.workspace()
        self.session.command("move-reminder")
        self.move_ready()
        wait_for(self.session, "Personal preserved reminder")
        assert "Goal and criteria template" not in self.session.display()
        self.workspace()
        self.quit()


def run_suite(binary, output, size, ascii_mode, results, keys, steps):
    suite_name = "-".join(keys)
    suite_output = output / suite_name
    journey = None
    with tempfile.TemporaryDirectory(prefix="moves-空 格-") as temporary:
        with Peer("chat_completions", "unused-synthetic-keyless", True) as peer:
            try:
                journey = Journey.__new__(Journey)
                journey.__init__(binary, suite_output, Path(temporary).resolve(), size, ascii_mode, results, peer)
                for key, method in steps:
                    journey.case(key, getattr(journey, method))
                peer.assert_clean()
            except Exception as error:
                for key in keys:
                    if results[key]["status"] == "pending":
                        results[key].update(status="failed", error="Prerequisite failed before this case: " + str(error), exercised=False)
                if not any(results[key]["status"] == "failed" for key in keys):
                    results[keys[-1]].update(status="failed", error=str(error), traceback=traceback.format_exc())
            finally:
                if journey:
                    try:
                        journey.close()
                    except Exception as error:
                        results[keys[-1]].update(status="failed", cleanup_error=str(error))
                suite_output.mkdir(parents=True, exist_ok=True)
                save(suite_output / "http-fixture-requests.json", {
                    "scope": "Actual loopback HTTP bytes decoded as JSON; not real-account proof",
                    "authentication": "Explicit keyless fixture; no real credential/account used",
                    "requests": peer.snapshot(), "errors": list(peer.errors)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="Immutable already-built executable")
    parser.add_argument("--output", required=True, type=Path, help="New retained evidence directory (must not exist)")
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    assert binary.is_file(), "--binary must identify an existing frozen executable"
    output.mkdir(parents=True, exist_ok=False)
    original_hash = digest(binary)
    result = {"status": "failed", "host": {"platform": platform.platform(), "machine": platform.machine(),
        "python": sys.version, "terminal": "ConPTY" if os.name == "nt" else "PTY"},
        "binary": str(binary), "binarySHA256": original_hash,
        "scope": "13 Wave5 Moves tasks through actual native TUI, real local commands and loopback HTTP/SSE only",
        "real_account_proof": False, "sql_writes": False, "sizes": []}
    save(output / "results.json", result)
    try:
        for size, ascii_mode in (((120, 36), False), ((60, 18), True)):
            name = "%dx%d-%s" % (*size, "ascii" if ascii_mode else "unicode")
            directory = output / name
            directory.mkdir()
            cases = {key: {"requirement": text, "status": "pending", "evidence": []} for key, text in CASES.items()}
            size_result = {"size": list(size), "ascii": ascii_mode, "cases": cases, "status": "failed"}
            result["sizes"].append(size_result)
            suites = (
                (("menu", "run-storage", "names", "reminder", "run-use", "party", "safety"),
                 (("menu", "menu_case"), ("run-storage", "storage_case"), ("names", "names_case"),
                  ("reminder", "reminder_case"), ("run-use", "run_use_case"), ("party", "party_case"), ("safety", "safety_case"))),
                (("repeat-runs",), (("repeat-runs", "repeat_case"),)),
                (("pins",), (("pins", "pins_case"),)),
                (("request", "repeat-pins"), (("request", "request_case"), ("repeat-pins", "repeat_pins_case"))),
                (("task", "purge"), (("task", "task_case"), ("purge", "purge_case"))),
            )
            for keys, steps in suites:
                assert digest(binary) == original_hash, "frozen executable changed during acceptance"
                run_suite(binary, directory, size, ascii_mode, cases, keys, steps)
                save(output / "results.json", result)
            size_result["status"] = "passed" if all(c["status"] == "passed" for c in cases.values()) else "failed"
        assert digest(binary) == original_hash, "binary changed during acceptance"
        result["status"] = "passed" if all(row["status"] == "passed" for row in result["sizes"]) else "failed"
    except Exception as error:
        result.update(status="failed", error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / "results.json", result)
    print(json.dumps({"status": result["status"], "results": str(output / "results.json")}, ensure_ascii=False))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
