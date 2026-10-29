#!/usr/bin/env python3
"""MV08B real PTY acceptance using an immutable prebuilt executable.

No builds, SQL writes, installed ev, real credentials or model submissions.
All projects/HOME/data and a keyless loopback model-catalogue fixture are disposable.
Requires terminal_smoke.py's pyte/wcwidth dependencies. Retains screens, terminal
bytes, read-only SQLite observations and a machine-readable result at
120x36 and 60x18. A failure is a failure, not an inferred pass.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback

from terminal_smoke import Session, confirm_exit, terminal_environment
from native_connection_smoke import Peer, MODEL

ESC = "\x1b"
HOME, END, DELETE = ESC + "[H", ESC + "[F", ESC + "[3~"
UP, DOWN, RIGHT, PAGE = ESC + "[A", ESC + "[B", ESC + "[C", ESC + "[6~"
FORMS = ("Eevee", "Vaporeon", "Jolteon", "Flareon", "Espeon", "Umbreon", "Leafeon", "Glaceon", "Sylveon")


def expose_review(session, capture):
    """Visit every wrapped row before the existing neutral confirmation."""
    session.send(HOME)
    observed = []
    neutral = "[Stay]"
    for index in range(256):
        view = session.display()
        observed.append(view)
        capture("semantic-review-%03d" % index)
        if neutral in view:
            return "\n".join(observed)
        session.send(PAGE)
    raise AssertionError("complete semantic scope did not become confirmable")

LABELS = {"import": "Import TM: literal", "target": "Import TM / choose", "name": "Move name",
           "review": "Confirm local change", "export": "Export TM: NEW", "export_review": "Create-only TM export",
           "menu": "i Import TM", "replace": "Give up learning", "run": "Review one-shot execution permission",
           "request": "Run once", "apply": "Apply template", "api": "API key", "exit": "Confirm exit",
           "source": "Review local capture"}
TABLES = ("tasks", "partners", "selections", "moves", "excerpts", "executions", "provider_journal",
          "move_usages", "move_sources", "move_dispatches", "move_candidates", "ribbons", "active_work")


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def definition(form, name=None):
    heading = 'schema_version = 1\nname = ' + json.dumps(name or "TM " + FORMS[form], ensure_ascii=False) + '\nform = "' + FORMS[form] + '"\n'
    if form == 0:
        return heading + '[steps.Task]\ngoal = "Portable local goal"\ncriteria = ["Inspect the result"]\n'
    if form == 1:
        return heading + '[[steps.Pins.pins]]\npath = "input.txt"\nstart_line = 1\nend_line = 2\n'
    if form in (2, 5, 6):
        return heading + '[steps.Run]\nargv = ["python3", "fixture.py", ""]\ncwd = "."\n'
    return heading + '[steps.Request]\ninstruction = "Inspect the portable change"\npins = []\n'


class Driver:
    def __init__(self, binary, output, root, size, peer):
        self.binary, self.output, self.root, self.size, self.peer = binary, output, root, size, peer
        self.labels, self.sessions, self.serial = LABELS, [], 0
        self.project = root / "project-one"
        self.home, self.data = root / "home", root / "data"
        self.database = self.data / "workbench.sqlite3"
        self.environment = terminal_environment("mono")
        for key in list(self.environment):
            if key.startswith(("GIT_", "EV_", "EEVEE_", "OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GEMINI_", "GOOGLE_", "AWS_", "OPENCODE_")) or key.lower().endswith("_proxy"):
                self.environment.pop(key, None)
        tools = root / "tools"
        tools.mkdir()
        git = shutil.which("git")
        assert git, "git needed for actual source observation"
        if os.name == "nt":
            tool_path = os.pathsep.join(dict.fromkeys((str(Path(sys.executable).parent), str(Path(git).parent))))
        else:
            (tools / "python3").symlink_to(Path(sys.executable).resolve())
            (tools / "git").symlink_to(Path(git).resolve())
            tool_path = str(tools)
        self.home.mkdir()
        self.environment.update(HOME=str(self.home), USERPROFILE=str(self.home), PATH=tool_path,
            XDG_CONFIG_HOME=str(self.home / "config"), XDG_DATA_HOME=str(self.home / "data"),
            XDG_CACHE_HOME=str(self.home / "cache"), APPDATA=str(self.home / "config"),
            LOCALAPPDATA=str(self.home / "cache"), NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        self.environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                                GIT_CONFIG_SYSTEM=os.devnull, GIT_CONFIG_COUNT="0")
        for project in (self.project, root / "project-two"):
            project.mkdir()
            (project / "fixture.py").write_text('from pathlib import Path\nPath("UNEXPECTED_EXECUTION").write_text("executed")\n', encoding="utf-8")
            (project / "input.txt").write_text("public first line\npublic second line\n", encoding="utf-8")
            for args in (("init", "--quiet"), ("add", "--", "."),
                         ("-c", "user.name=Local acceptance", "-c", "user.email=local@example.invalid", "commit", "-qm", "fixture")):
                subprocess.run([git, "-c", "core.hooksPath=", *args], cwd=project,
                               env=self.environment, check=True, capture_output=True, timeout=15)
        self.launch()

    def launch(self):
        self.serial += 1
        self.session = Session([str(self.binary), "--data-dir", str(self.data), "--images", "off", "--color", "mono", ],
                               self.project, self.environment, self.size, self.output / ("terminal-%02d" % self.serial))
        self.sessions.append(self.session)
        # Existing SQLite rows predate this process; wait for its first ready
        # frame before sending navigation or reading a footer.
        self.until(lambda: "Storage: saved" in self.session.display(), "current process initialized")
        self.until(lambda: self.database.is_file() and self.state().get("partners"), "store startup")
        if not self.state().get("tasks") or not any(r["project"] == str(self.project) for r in self.state()["tasks"]):
            self.session.command("goal")
            self.session.paste("Portable local acceptance")
            self.session.send("\r")
            self.until(lambda: any(r["project"] == str(self.project) and r["goal"] == "Portable local acceptance" for r in self.state()["tasks"]), "local task saved")
        self.workspace()
        self.connect()

    def state(self):
        try:
            with closing(sqlite3.connect(self.database.as_uri() + "?mode=ro", uri=True, timeout=10)) as db:
                db.row_factory = sqlite3.Row
                tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
                result = {name: [dict(r) for r in db.execute('SELECT * FROM "' + name + '" ORDER BY rowid')] for name in TABLES if name in tables}
                for rows in result.values():
                    for row in rows:
                        for key in ("proposal", "record", "partner_drafts", "criteria"):
                            if key in row and isinstance(row[key], str):
                                row[key] = json.loads(row[key])
                return result
        except sqlite3.OperationalError:
            return {}

    def until(self, predicate, why, timeout=30):
        deadline = time.monotonic() + timeout
        while not predicate():
            assert self.session.alive(), "app exited: " + why
            assert time.monotonic() < deadline, why + "\n" + self.session.display()
            self.session.pump(.03)

    def wait(self, key):
        self.until(lambda: self.labels[key] in self.session.display()
                   and (key != "menu" or not any(text in self.session.display() for text in
                        ("Loading/saving local move facts", "Saving the reviewed move change", ))), key)

    def capture(self, name):
        self.serial += 1
        self.session.capture("%03d-%s" % (self.serial, name))
        save(self.output / ("%03d-%s-sqlite.json" % (self.serial, name)), self.state())

    def workspace(self):
        for _ in range(12):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if footer.startswith(("Type Request |", "Enter Send |", "Draft only |", )):
                return
            self.session.send(ESC)
        raise AssertionError("workspace not restored\n" + self.session.display())

    def connect(self):
        self.session.command("connect compatible")
        self.wait("api")
        self.session.send("\t\x15")
        self.session.paste(self.peer.base)
        self.session.send("\t\x15")
        self.session.paste(MODEL)
        self.session.send("\t\t \t\t\r")
        self.until(lambda: MODEL in self.session.display()
                   and "matches" in self.session.display(), "local catalogue chooser")
        self.session.send(MODEL + "\r")
        self.workspace()
        assert not self.peer.snapshot("POST"), "setup submitted a model request"

    def effects(self):
        state = self.state()
        return {key: state.get(key, []) for key in ("executions", "excerpts", "provider_journal", "move_usages", "move_sources", "move_dispatches")}

    def no_effect(self, before):
        assert self.effects() == before, "preview/import/export changed execution/context/evidence"
        assert not self.peer.snapshot("POST"), "TM flow submitted a model request"
        assert not any(project.joinpath("UNEXPECTED_EXECUTION").exists() for project in (self.root / "project-one", self.root / "project-two")), "TM flow executed a command"

    def rows(self, form=None):
        return [r for r in self.state().get("moves", []) if not r["forgotten"] and (form is None or r["form"] == form)]

    def import_preview(self, path):
        self.workspace()
        self.session.command("move")
        self.wait("menu")
        self.session.send("i")
        self.wait("import")
        self.session.paste(str(path))
        self.session.send("\r")
        self.wait("target")
        self.capture("import-partner-preview")
        self.session.send("\r")
        self.wait("name")
    def expose_review(self):
        return expose_review(self.session, self.capture)


    def learn(self, path, form, name=None, personal=False, replacement=None):
        before, effects = self.state().get("moves", []), self.effects()
        self.import_preview(path)
        if name:
            self.session.send(HOME + DELETE * 64)
            self.session.paste(name)
        if personal:
            self.session.send("\t" + RIGHT + "\t")
        self.session.send("\r")
        if replacement is not None:
            self.wait("replace")
            self.session.send(DOWN * (replacement + 1) + "\r")
        self.wait("review")
        assert self.state().get("moves", []) == before, "import preview learned a move"
        self.no_effect(effects)
        self.capture("import-neutral-local-review")
        self.expose_review()
        self.session.send("y")
        self.until(lambda: self.state().get("moves", []) != before, "explicit import committed")
        self.wait("menu")
        self.no_effect(effects)
        self.workspace()
        learned = next(r for r in self.rows(form) if r["proposal"]["name"] == (name or "TM " + FORMS[form]))
        provenance = learned["proposal"]["provenance"]
        assert provenance["taught_by"] is None and provenance["teacher_name"] is None
        assert len(provenance["sources"]) == 1 and provenance["sources"][0].startswith("tm:")
        return learned

    def form(self, form):
        self.workspace()
        self.session.command(FORMS[form].lower())
        self.until(lambda: any(t["project"] == str(self.project) and t["form"] == form for t in self.state()["tasks"]), "form saved")
        self.workspace()

    def export(self, form, path, confirm=True):
        self.form(form)
        self.session.command("move")
        self.wait("menu")
        self.session.send("e")
        self.wait("export")
        self.session.paste(str(path))
        self.session.send("\r")
        self.wait("export_review")
        self.capture("export-neutral-review")
        assert not path.exists(), "export path created before confirmation"
        self.session.send("y" if confirm else "\r")
        if confirm:
            self.until(path.exists, "new TM file created")
        else:
            self.wait("menu")
            assert not path.exists(), "neutral export created output"
        self.workspace()

    def discard_failure(self):
        self.until(lambda: "Move storage failed" in self.session.display()
                   or "Retry" in self.session.display(), "retained failed operation")
        self.capture("refusal-retained-for-explicit-retry")
        self.session.send("d")
        self.session.send("y")
        self.wait("menu")
        self.workspace()

    def quit(self):
        self.workspace()
        self.session.command("quit")
        confirm_exit(self.session)
        self.until(lambda: not self.session.alive(), "quit settled")
        assert self.session.wait_exit() == 0, "native process returned failure"

    def clear_request(self):
        self.workspace()
        footer = self.session.display().rstrip().splitlines()[-1].strip()
        if footer.startswith(("Type Request |", )):
            self.session.send("\r")
        self.session.send(UP * 20 + HOME + DELETE * 200 + "\x13")
        def cleared():
            task = next(t for t in self.state()["tasks"] if t["project"] == str(self.project))
            drafts = task["partner_drafts"].get(str(task["lead_partner"]), {}).get("drafts", [])
            return not any(d["field"] == "Request" for d in drafts)
        self.until(cleared, "empty Request draft durably retained")
        self.workspace()

    def use_review(self, form):
        self.form(form)
        if form in (3, 4, 7, 8):
            self.clear_request()
        self.session.command("move")
        self.wait("menu")
        before = self.effects()
        self.session.send("\r")
        if form == 0:
            self.wait("apply")
            self.capture("task-fresh-apply-review")
            self.session.send("\r")
        elif form == 1:
            self.until(lambda: self.labels["source"] in self.session.display() or "Review local capture" in self.session.display(), "fresh ordinary source review")
            self.capture("pin-fresh-capture-review")
            self.session.send(ESC)
        elif form in (2, 5, 6):
            self.wait("run")
            self.capture("run-fresh-one-shot-review")
            self.session.command("deny")
            # A real explicit denial may record usage, but never a run.
            self.until(lambda: not self.labels["run"] in self.session.display(), "normal command denial")
        else:
            self.wait("request")
            self.capture("request-fresh-full-receipt")
            # Escape only defers a request review. Explicitly cancel its owned
            # preparation before proceeding to another independent fixture.
            self.session.command("cancel")
            self.until(lambda: not self.state().get("active_work"), "owned request review cancelled")
        after = self.effects()
        assert after["executions"] == before["executions"] and after["excerpts"] == before["excerpts"]
        assert not self.peer.snapshot("POST")
        self.workspace()
        if form in (3, 4, 7, 8):
            self.clear_request()

    def run(self):
        sources, exports, original = [], [], []
        for form in range(9):
            source = self.root / ("source-%d.toml" % form)
            source.write_text(definition(form), encoding="utf-8")
            sources.append(source)
            original.append(self.learn(source, form, personal=(form == 8)))
            output = self.root / ("export-%d.toml" % form)
            self.export(form, output)
            exports.append(output)
            self.use_review(form)
        # Cancellation preserves both the complete move inventory and output.
        before, effects = self.state()["moves"], self.effects()
        self.import_preview(sources[0])
        self.session.send(ESC)
        self.workspace()
        assert self.state()["moves"] == before
        cancelled = self.root / "cancelled.toml"
        self.export(0, cancelled, confirm=False)
        self.no_effect(effects)
        # Existing output is refused, byte-identical, with exact retry/discard ownership.
        self.form(0)
        self.session.command("move-export")
        self.wait("export")
        existing = exports[0].read_bytes()
        self.session.paste(str(exports[0]))
        self.session.send("\r")
        self.wait("export_review")
        self.session.send("y")
        self.discard_failure()
        assert exports[0].read_bytes() == existing
        # A same-name TM opens an explicit old/new replacement review, never an
        # implicit overwrite. Neutral cancellation preserves all inventory.
        self.import_preview(sources[0])
        self.session.send("\r")
        self.wait("review")
        assert self.state()["moves"] == before
        self.session.send(ESC)
        self.workspace()
        for suffix in ("Library %02d" % index for index in range(2, 33)):
            self.learn(sources[0], 0, name=suffix)
        full = self.state()["moves"]
        self.import_preview(sources[0])
        self.session.send(HOME + DELETE * 64)
        self.session.paste("Fifth")
        self.session.send("\r")
        self.wait("replace")
        self.session.send("\r")
        assert self.state()["moves"] == full, "neutral full-slot action replaced a move"
        self.workspace()
        self.learn(sources[0], 0, name="Fifth", replacement=0)
        # Refusals are literal file operations; no invented authority is accepted.
        bad = {
            "version": definition(0).replace("schema_version = 1", "schema_version = 2"),
            "foreign-id": 'task_id = 123\n' + definition(0),
            "grant": 'permissions = ["execute"]\n' + definition(0),
            "secret": definition(0).replace("Portable local goal", "token=sk-1234567890123456789012345678901234567890"),
            "traversal": definition(1).replace("input.txt", "../outside.txt"),
            "excluded": definition(1).replace("input.txt", ".env"),
            "oversize": "x" * 65537,
        }
        for kind, text in bad.items():
            path = self.root / ("bad-" + kind + ".toml")
            path.write_text(text, encoding="utf-8")
            before, effects = self.state()["moves"], self.effects()
            self.session.command("move-import")
            self.wait("import")
            self.session.paste(str(path))
            self.session.send("\r")
            self.discard_failure()
            assert self.state()["moves"] == before
            self.no_effect(effects)
        # Restart retains exact steps/provenance and no restored approval.
        before = self.state()["moves"]
        self.quit()
        self.launch()
        assert self.state()["moves"] == before
        assert not self.peer.snapshot("POST")
        self.capture("restart-exact-local-records")
        self.quit()
        self.project = self.root / "project-two"
        self.launch()
        for form, path in enumerate(exports):
            if form == 8:  # Personal TM remains visible across project boundaries.
                continue
            learned = self.learn(path, form, name="Across " + FORMS[form])
            assert learned["proposal"]["steps"] == original[form]["proposal"]["steps"]
            assert learned["proposal"]["provenance"]["task_id"] != original[form]["proposal"]["provenance"]["task_id"]
        assert any(r["proposal"]["scope"] == "Personal" and r["form"] == 8 for r in self.rows())
        self.form(8)
        self.session.command("move")
        self.wait("menu")
        assert "TM Sylveon" in self.session.display(), "personal move not visible in the second project"
        self.capture("personal-move-visible-across-projects")
        self.workspace()
        # Imported pin can honestly remain unavailable until use, never recaptured.
        (self.project / "input.txt").unlink()
        self.form(1)
        self.session.command("move")
        self.wait("menu")
        effects = self.effects()
        self.session.send("\r")
        self.until(lambda: any(reason in self.session.display() for reason in (
            "Source is now ignored or absent", "Source is absent from the current",
            "Selected source is not an available regular file", "No such file or directory",
            "cannot find the file", "Source is binary, oversized, linked, missing or unavailable")),
            "missing imported selector unavailable")
        self.capture("missing-pin-no-substitution-or-capture")
        self.no_effect(effects)
        self.workspace()
        self.quit()
        before = self.state()["moves"]
        self.launch()
        assert self.state()["moves"] == before
        self.capture("cross-project-restart")
        self.quit()
        self.peer.assert_clean()

    def close(self):
        for session in self.sessions:
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    assert binary.is_file(), "--binary must be an immutable already-built executable"
    output.mkdir(parents=True, exist_ok=False)
    digest = sha(binary)
    result = {"status": "failed", "binary": str(binary), "sha256": digest,
              "scope": "MV08B actual PTY, local files/read-only SQL, no inference or real accounts", "cases": []}
    try:
        for size in ((120, 36), (60, 18)):
            case = {"size": list(size), "status": "failed"}
            result["cases"].append(case)
            directory = output / ("%dx%d" % size)
            directory.mkdir()
            driver = None
            with tempfile.TemporaryDirectory(prefix="ev-tm-") as temporary:
                with Peer("chat_completions", "unused-keyless-fixture", True) as peer:
                    try:
                        driver = Driver.__new__(Driver)
                        driver.__init__(binary, directory, Path(temporary).resolve(), size, peer)
                        driver.run()
                        case["status"] = "passed"
                    except Exception as error:
                        case.update(error=str(error), traceback=traceback.format_exc())
                        if driver and hasattr(driver, "session"):
                            try:
                                driver.capture("failed")
                            except Exception as capture_error:
                                case["capture_error"] = str(capture_error)
                    finally:
                        if driver:
                            driver.close()
                        save(directory / "loopback-requests.json", peer.snapshot())
            assert sha(binary) == digest, "binary changed during acceptance"
            save(output / "results.json", result)
        result["status"] = "passed" if all(case["status"] == "passed" for case in result["cases"]) else "failed"
    finally:
        save(output / "results.json", result)
    print(json.dumps({"status": result["status"], "results": str(output / "results.json")}))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
