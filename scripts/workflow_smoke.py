#!/usr/bin/env python3
"""Native UI acceptance for persisted specialist decisions and consent boundaries.

Only disposable projects and explicitly approved local Python checks are used.
No model request, credentials, or source-changing provider is implicit.
"""
import argparse
from contextlib import ExitStack, closing
import hashlib
import json
import os
import platform
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC = "\x1b"
UP, DOWN, RIGHT, LEFT = (ESC + value for value in ("[A", "[B", "[C", "[D"))
HOME, DELETE, PAGE = ESC + "[H", ESC + "[3~", ESC + "[6~"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    if os.name == "nt" and not binary.exists():
        binary = binary.with_suffix(".exe")
    output.mkdir(parents=True, exist_ok=False)
    result = {"status": "failed", "host": platform.platform(), "cases": [],
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "scope": "Actual native UI and local evidence; no external-provider acceptance claim"}
    sessions = []

    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)

    def cleanup():
        for session in sessions:
            if not session.closed:
                session.capture("final")
                session.close()

    try:
        with ExitStack() as stack:
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="ev-workflow-空 格-"))).resolve()
            stack.callback(cleanup)
            project, data = root / "project 漢字", root / "data"
            project.mkdir()
            source = project / "calc.py"
            source.write_text("def total(a, b):\n    return a + b\n", encoding="utf-8")
            (project / "check.py").write_text("from calc import total\nactual = total(7, 2)\nprint(f'observed total={actual}; inputs=7,2; expected=9', flush=True)\nassert actual == 9\n", encoding="utf-8")
            for arguments in (("init", "--quiet"), ("add", "--", "calc.py", "check.py"),
                              ("-c", "user.name=Smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "fixture")):
                subprocess.run(["git", "-c", "core.autocrlf=false", "-c", "core.hooksPath=", *arguments],
                               cwd=project, check=True, capture_output=True, timeout=10)
            database = data / "workbench.sqlite3"

            def rows():
                with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=1)) as db:
                    db.row_factory = sqlite3.Row
                    task = dict(db.execute("SELECT * FROM tasks ORDER BY id LIMIT 1").fetchone())
                    runs = [json.loads(row[0]) for row in db.execute("SELECT record FROM executions ORDER BY rowid")]
                    memories = [dict(row) for row in db.execute("SELECT * FROM memories ORDER BY id")]
                    journals = db.execute("SELECT count(*) FROM provider_journal").fetchone()[0]
                for field in ("workflows", "workflow_draft", "memory_draft", "approved_memories", "criteria"):
                    task[field] = json.loads(task[field])
                return task, runs, memories, journals

            def request_text():
                task = rows()[0]
                partners = json.loads(task["partner_drafts"])
                drafts = partners.get(str(task["lead_partner"]), {}).get("drafts", [])
                return next((draft["text"] for draft in drafts if draft["field"] == "Request"), "")

            def until(session, predicate, reason, timeout=35):
                deadline = time.monotonic() + timeout
                while not predicate():
                    assert session.alive(), "ev exited: " + reason
                    assert time.monotonic() < deadline, reason + "\n" + session.display()
                    session.pump(0.05)

            def launch(name, cwd=project, restored_memory=False):
                session = Session([str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"],
                                  cwd, terminal_environment("mono"), (120, 36), output / name)
                sessions.append(session)
                if restored_memory:
                    wait_for(session, "Approved memories", timeout=35)
                    session.capture("restored-memory-surface")
                    session.send(ESC)
                wait_for(session, "Storage: saved", timeout=35)
                return session

            def menu(session, index):
                session.send(UP * 20 + DOWN * index + "\r")

            def edit(session, index, label, text, previous=""):
                menu(session, index)
                wait_for(session, label)
                if previous:
                    session.send(HOME + DELETE * len(previous))
                session.paste(text)
                session.send("\x13")

            def pin_source(session, expected):
                session.command("files")
                wait_for(session, "eligible files /", timeout=35)
                session.command("search")
                wait_for(session, "Filename filter")
                session.send(HOME + DELETE * 16)
                session.paste("calc.py")
                session.send("\r")
                wait_for(session, "1 eligible files /", timeout=35)
                session.send("\r")
                wait_for(session, "Review local capture /")
                session.command("range")
                wait_for(session, "Source range / inclusive")
                session.send(HOME + DELETE * 3)
                session.paste("1:2")
                session.send("\r")
                session.command("attach")
                wait_for(session, str(expected) + " task-owned basket references / stored")
                session.command("close")
                wait_for(session, "Storage: saved")

            def execute_check(session, expected, state="Succeeded"):
                session.command("jolteon")  # Jolteon owns command preparation.
                session.command("run")
                wait_for(session, "local command evidence")
                session.command("command-edit")
                wait_for(session, "Command / explicit JSON argv or shell")
                if not rows()[0]["command"]:
                    session.paste(json.dumps([str(Path(sys.executable).resolve()), "-B", str(project / "check.py")]))
                session.send("\r")
                wait_for(session, "Approval pending", timeout=35)
                session.command("run")
                wait_for(session, "Review one-shot execution permission")
                session.send(RIGHT + "\r")
                until(session, lambda: len(rows()[1]) == expected and rows()[1][-1]["state"] == state,
                      "actual bounded check did not reach " + state)
                wait_for(session, "Local command evidence; no command running", timeout=35)
                session.capture("check-" + str(expected))
                session.send(ESC)

            def attach(session, action, choice, refute=False):
                menu(session, action)
                session.send(DOWN * choice + (RIGHT if refute else "") + "\r")
                wait_for(session, "Specialist desk")

            def quit_app(session):
                session.send("\x03")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()

            first = launch("specialists")
            first.send(ESC)
            first.command("goal")
            first.paste("Specialist acceptance 漢字")
            first.send("\r")
            first.command("criteria")
            first.command("criteria-edit")
            first.paste("total(7, 2) returns 9")
            first.send("\r")
            first.send(ESC)
            first.command("next")
            first.paste("Keep bounded checks and explicit approvals.")
            first.send("\r")
            wait_for(first, "Storage: saved")
            until(first, lambda: rows()[0]["next_action"] == "Keep bounded checks and explicit approvals.", "task setup")

            first.command("workflows")
            wait_for(first, "Specialist desk")
            edit(first, 0, "Hypothesis / explanation", "The numeric types may overflow 漢字 é")
            menu(first, 1)
            until(first, lambda: len(rows()[0]["workflows"]["hypotheses"]) == 1, "hypothesis save")
            wait_for(first, "Unresolved")
            first.capture("diagnosis-unresolved")
            first.resize((60, 18))
            wait_for(first, "Specialist desk")
            first.capture("diagnosis-compact")
            first.resize((59, 17))
            wait_for(first, "need 60 x 18")
            before = rows()[0]["workflows"]
            first.send("\r" + RIGHT + "\r")
            assert rows()[0]["workflows"] == before, "below-minimum affirmative input changed decisions"
            first.resize((120, 36))
            passed("Unicode hypothesis persistence, unresolved status, compact and below-minimum safety")

            first.send(RIGHT * 2)
            edit(first, 1, "Finding detail", "No actual passing evidence is linked yet.")
            edit(first, 2, "Actionable recommendation", "Run the bounded check and inspect the selected source.")
            menu(first, 4)
            until(first, lambda: len(rows()[0]["workflows"]["findings"]) == 1, "finding save")
            menu(first, 11)
            wait_for(first, "RequestChanges")
            menu(first, 12)
            assert not rows()[0]["complete"], "a review request completed the task"
            wait_for(first, "RequestChanges")
            first.capture("review-blocked")
            passed("Actionable review finding and guarded acceptance without fabricated verification")

            first.send(RIGHT)
            marker = root / "recipe-must-not-execute"
            recipe_path = root / "workflow.toml"
            recipe = ("schema_version = 1\nname = 'Bounded review recipe'\nscope = ['calc.py']\ncontext = ['calc.py']\n"
                      "steps = [{form = 'Glaceon', action = 'Review the selected patch independently'}]\n"
                      "validation_commands = " + json.dumps([[str(Path(sys.executable).resolve()), "-c", "from pathlib import Path; Path(" + repr(str(marker)) + ").touch()"]]) + "\n"
                      "completion_criteria = ['Recipe criteria require actual evidence']\n")
            recipe_path.write_text(recipe, encoding="utf-8")
            edit(first, 0, "Literal recipe path", str(recipe_path))
            menu(first, 1)
            menu(first, 2)
            wait_for(first, "Apply plan only")
            first.send("\r")
            assert rows()[0]["workflows"]["recipe"] is None, "neutral Enter applied a recipe"
            menu(first, 2)
            first.send(PAGE * 8)
            first.capture("recipe-exact-preview")
            first.send(RIGHT + "\r")
            until(first, lambda: rows()[0]["workflows"]["recipe"] is not None, "recipe plan save")
            assert not marker.exists(), "recipe preview/application executed a command"
            assert rows()[1] == [] and rows()[3] == 0
            assert any(item["text"] == "Recipe criteria require actual evidence" for item in rows()[0]["criteria"])
            assert not rows()[0]["complete"]
            passed("Exact recipe preview, neutral default, explicit plan-only apply without execution or model calls")

            recipe_path.write_text(recipe + "permissions = ['write-anywhere']\n", encoding="utf-8")
            menu(first, 1)
            wait_for(first, "Invalid strict recipe TOML schema")
            assert not marker.exists()
            passed("Unknown recipe capability fields refused")

            first.send(ESC)
            first.command("memory")
            wait_for(first, "Approved memories")
            first.command("create-memory")
            first.paste("Always retain focused regression evidence 漢字.")
            first.send("\r")
            wait_for(first, "Approve")
            first.send("\r")
            assert rows()[2] == [], "neutral Enter approved memory"
            first.send(PAGE * 8 + RIGHT + "\r")
            until(first, lambda: len(rows()[2]) == 1, "explicit memory approval")
            first.command("select-memory")
            until(first, lambda: len(rows()[0]["approved_memories"]) == 1, "memory context selection")
            first.resize((60, 18))
            first.capture("memory-compact")
            first.resize((120, 36))
            passed("Explicitly approved task memory and separately selected outgoing context")
            first.command("create-memory")
            first.paste("Reject this proposed permanent habit.")
            first.send("\r")
            first.command("reject-memory")
            wait_for(first, "INSPECT /")
            assert len(rows()[2]) == 1
            assert "Reject this proposed" not in rows()[2][0]["proposal"]
            quit_app(first)

            resumed = launch("resumed", restored_memory=True)
            resumed.send(ESC)
            resumed.command("workflows")
            wait_for(resumed, "Specialist desk")
            assert len(rows()[0]["workflows"]["hypotheses"]) == 1
            assert len(rows()[0]["workflows"]["findings"]) == 1
            assert rows()[0]["workflows"]["recipe"]["recipe"]["name"] == "Bounded review recipe"
            resumed.send(ESC)
            resumed.command("memory")
            wait_for(resumed, "Approved memories")
            wait_for(resumed, "Always retain")
            resumed.capture("memory-resumed")
            resumed.command("delete-memory")
            wait_for(resumed, "Delete")
            resumed.send("\r")
            assert len(rows()[2]) == 1, "neutral Enter deleted approved memory"
            resumed.send(PAGE * 8 + RIGHT + "\r")
            until(resumed, lambda: rows()[2] == [], "memory deletion")
            until(resumed, lambda: rows()[0]["approved_memories"] == [], "deleted memory removed from future context")
            assert not marker.exists() and rows()[3] == 0
            passed("Restart restored specialist records; deliberate deletion removed future memory context")
            resumed.send(ESC)
            source.write_text("def total(a, b):\n    return a - b\n", encoding="utf-8")
            execute_check(resumed, 1, "Failed")
            pin_source(resumed, 1)
            resumed.command("workflows")
            wait_for(resumed, "Specialist desk")
            resumed.send(RIGHT)  # persisted Recipe -> Diagnosis
            attach(resumed, 3, 1, refute=True)
            wait_for(resumed, "Refuted")
            diagnosis = "Subtraction returns 5 instead of the required integer sum 9."
            edit(resumed, 0, "Hypothesis / explanation", diagnosis)
            menu(resumed, 1)
            attach(resumed, 3, 0, refute=True)  # restore Supporting after prior explicit Refuting
            attach(resumed, 3, 1)
            wait_for(resumed, "Supported")
            resumed.capture("diagnosis-linked-observations")
            passed("Actual source and command links distinguish refuted and supported hypotheses")
            previous_next = rows()[0]["next_action"]
            menu(resumed, 7)
            wait_for(resumed, "Enter Review")
            resumed.send("\r")
            wait_for(resumed, "[Stay]")  # The neutral review must render before confirmation.
            resumed.send("y")
            until(resumed, lambda: rows()[0]["form"] == 3 and diagnosis in request_text(), "diagnosis reference relay save")
            assert "source:" in request_text() and "run:" in request_text()
            assert "overflow" not in request_text()
            assert rows()[0]["next_action"] == previous_next
            assert source.read_text(encoding="utf-8") == "def total(a, b):\n    return a - b\n"
            assert rows()[3] == 0
            resumed.send(ESC)  # relay closes; return from the specialist desk
            resumed.send(ESC)  # preserve the staged Request while returning to the workspace
            source.write_text("def total(a, b):\n    return a + b\n", encoding="utf-8")
            execute_check(resumed, 2)
            pin_source(resumed, 2)
            resumed.command("workflows")
            passed("Seeded defect diagnosis hands off only selected inference/references without writing or sending")

            resumed.send(RIGHT)
            edit(resumed, 0, "Behavior contract", "total(7, 2) returns integer 9; preserve this observed behavior.")
            attach(resumed, 1, 1)
            attach(resumed, 2, 0)
            resumed.command("record-refactor")
            until(resumed, lambda: rows()[0]["workflows"]["refactor"] is not None, "bounded baseline save")
            refactor_request = request_text()
            resumed.command("specialist-fix")
            wait_for(resumed, "Enter Review")
            resumed.send("\r")
            wait_for(resumed, "[Stay]")
            resumed.send("y")
            wait_for(resumed, "nonempty")
            assert request_text() == refactor_request and rows()[0]["form"] == 2, "refactor relay overwrote the existing sender draft"
            assert rows()[3] == 0
            resumed.send(ESC)
            resumed.send(ESC)
            source.write_text("def total(a, b):\n    return a - b\n", encoding="utf-8")
            execute_check(resumed, 3, "Failed")
            resumed.command("workflows")
            attach(resumed, 7, 0)
            resumed.command("compare-refactor")
            wait_for(resumed, "block a behavior-preservation claim")
            resumed.capture("refactor-failed-check-blocked")
            resumed.send(ESC)
            source.write_text("def total(a, b):\n    return sum((a, b))\n", encoding="utf-8")
            execute_check(resumed, 4)
            resumed.command("workflows")
            attach(resumed, 7, 0)  # native journal choices are newest first
            resumed.command("compare-refactor")
            wait_for(resumed, "selected contract check passed")
            resumed.capture("refactor-real-comparison")
            comparison = rows()[0]["workflows"]["refactor"]
            assert comparison["baseline_run"] == rows()[1][1]["prepared"]["run_id"]
            assert comparison["post_run"] == rows()[1][3]["prepared"]["run_id"]
            passed("Distinct ordered real checks compare a bounded behavior-preserving source change")

            resumed.send(ESC)
            pin_source(resumed, 3)
            resumed.command("workflows")
            resumed.send(RIGHT)
            edit(resumed, 2, "Actionable recommendation", "Observed the passing local check and inspected the current source.",
                 previous="Run the bounded check and inspect the selected source.")
            menu(resumed, 14)
            for criterion in range(2):
                attach(resumed, 8, 2)  # newly captured post-change source
                attach(resumed, 8, 3)  # newest run after three source rows
                if criterion == 0:
                    menu(resumed, 0)
            menu(resumed, 12)
            wait_for(resumed, "Recorded Accept")
            assert rows()[0]["workflows"]["review"]["requested"] == "Accept"
            assert not rows()[0]["complete"]
            resumed.capture("review-current-acceptance")
            source.write_text(source.read_text(encoding="utf-8") + "# changed after review\n", encoding="utf-8")
            menu(resumed, 15)
            wait_for(resumed, "Effective verdict: Inconclusive", timeout=35)
            resumed.capture("review-stale-inconclusive")
            passed("Coverage-guarded review accepts current evidence, then invalidates after a source change")
            edit(resumed, 1, "Finding detail", "Source changed after the successful check; current behavior is unverified.")
            edit(resumed, 2, "Actionable recommendation", "Run the current bounded check and review this changed source again.",
                 previous="Observed the passing local check and inspected the current source.")
            attach(resumed, 3, 2)
            attach(resumed, 3, 3)
            menu(resumed, 4)
            until(resumed, lambda: len(rows()[0]["workflows"]["findings"]) == 2, "unresolved finding save")

            resumed.send(ESC)
            resumed.command("handoff")
            wait_for(resumed, "Portable task handoff / local-only")
            document_path = root / "existing-notes.md"
            document_path.write_text("User documentation must stay exact.\n", encoding="utf-8")
            resumed.command("export-markdown")
            wait_for(resumed, "LITERAL FILE PATH")
            resumed.paste(str(document_path))
            resumed.send("\r")
            wait_for(resumed, "SANITIZED EXPORT PREVIEW")
            wait_for(resumed, "y Confirm")
            resumed.send("\r")
            assert document_path.read_text(encoding="utf-8") == "User documentation must stay exact.\n"
            passed("Rejecting the separate documentation-write preview preserves existing bytes")
            resumed.send(ESC)
            resumed.command("export-json")
            wait_for(resumed, "LITERAL FILE PATH")
            exported = root / "specialist-handoff.json"
            resumed.paste(str(exported))
            resumed.send("\r")
            wait_for(resumed, "SANITIZED EXPORT PREVIEW")
            wait_for(resumed, "y Confirm")
            resumed.send(RIGHT + "\r")
            until(resumed, exported.exists, "source-linked handoff export")
            handoff = json.loads(exported.read_text(encoding="utf-8"))
            risks = "\n".join(handoff["unresolved"])
            assert "source:" in risks and "run:" in risks and "unverified" in risks
            assert handoff["task"]["next_action"] == rows()[0]["next_action"]
            assert handoff["schema_version"] == 1
            result["portable_handoff"] = handoff
            passed("Explicit portable export carries source/run-linked unresolved conclusions without grants")
            result["persisted_workflows"] = rows()[0]["workflows"]
            resumed.send(ESC)
            resumed.send(ESC)
            resumed.command("memory")
            wait_for(resumed, "Approved memories")
            for scope, note in enumerate(("Task-only note", "Project-shared note", "Personal-shared note")):
                resumed.command("create-memory")
                resumed.paste(note)
                resumed.send("\r")
                if scope:
                    for _ in range(scope):
                        resumed.command("memory-scope")
                resumed.send(PAGE * 8 + RIGHT + "\r")
                until(resumed, lambda: len(rows()[2]) == scope + 1, "scoped approval did not commit")
            resumed.send(ESC)
            wait_for(resumed, "Storage: saved")
            resumed.command("task-new")
            resumed.command("goal")
            resumed.paste("Second task for memory isolation")
            resumed.send("\r")
            wait_for(resumed, "Storage: saved")
            resumed.command("memory")
            wait_for(resumed, "Project-shared note")
            resumed.send(DOWN)
            wait_for(resumed, "Personal-shared note")
            wait_for(resumed, "Memory 2/2")
            assert "Task-only note" not in resumed.display()
            resumed.capture("same-project-memory-isolation")
            quit_app(resumed)
            recovered = launch("linked-evidence-resume", restored_memory=True)
            recovered.command("tasks")
            wait_for(recovered, "Task cards / committed snapshots")
            wait_for(recovered, "Storage: saved")
            recovered.send(HOME)
            wait_for(recovered, "Storage: saved")
            recovered.send("\r")
            recovered.command("goal")
            wait_for(recovered, "Specialist acceptance")
            wait_for(recovered, "Storage: saved")
            recovered.send("\x13")
            recovered.command("workflows")
            wait_for(recovered, "Specialist desk")
            assert rows()[0]["workflows"]["findings"][-1]["resolution"] is None
            assert rows()[0]["next_action"] == handoff["task"]["next_action"]
            menu(recovered, 7)
            wait_for(recovered, "Stored source reference calc.py:1-2")
            wait_for(recovered, "return sum((a, b))")
            recovered.capture("resumed-exact-source-link")
            recovered.command("close")
            wait_for(recovered, "Specialist desk")
            menu(recovered, 6)
            menu(recovered, 7)
            wait_for(recovered, "observed total=9")
            recovered.capture("resumed-actual-run-link")
            recovered.send(ESC)
            wait_for(recovered, "Specialist desk")
            passed("Unresolved finding and exact next action resume with inspectable stored source and actual run output")
            recovered.send(ESC)
            recovered.command("tasks")
            wait_for(recovered, "Task cards / committed snapshots")
            wait_for(recovered, "Storage: saved")
            recovered.send(HOME)
            recovered.command("delete")
            wait_for(recovered, "Review local task deletion")
            recovered.capture("task-memory-deletion-impact")
            recovered.send(PAGE * 4)
            recovered.send(RIGHT + "\r")
            until(recovered, lambda: len(rows()[2]) == 2, "task purge must delete only task-local memory")
            assert {row["scope"] for row in rows()[2]} == {"project", "personal"}
            assert json.loads(exported.read_text(encoding="utf-8")) == handoff
            recovered.send(ESC)
            recovered.command("memory")
            wait_for(recovered, "Project-shared note")
            recovered.send(DOWN)
            wait_for(recovered, "Personal-shared note")
            wait_for(recovered, "Memory 2/2")
            assert "Task-only note" not in recovered.display()
            passed("Actual task purge preserves approved shared memories and previously exported handoff")
            quit_app(recovered)
            foreign = root / "different-project"
            foreign.mkdir()
            other = launch("foreign-memory-scope", foreign)
            other.send(ESC)
            other.command("goal")
            other.paste("Foreign project memory isolation")
            other.send("\r")
            wait_for(other, "Storage: saved")
            other.command("memory")
            wait_for(other, "Personal-shared note")
            assert "Project-shared note" not in other.display()
            assert "Task-only note" not in other.display()
            other.capture("different-project-memory-isolation")
            passed("Task/project/personal approvals enforce actual cross-task and cross-project visibility")
            quit_app(other)
        result["status"] = "passed"
    except BaseException as error:
        result["error"] = str(error)
        raise
    finally:
        (output / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
