#!/usr/bin/env python3
"""Wave-3 relay acceptance through actual keyboard UI at both supported sizes.

Uses disposable git projects, real approved Python runs and an explicitly labelled
loopback proposal fixture. Never seeds execution, source, verdict or decision rows.
The only SQLite access is read-only observation of committed application state.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import shlex
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import NativeFixture, wait_for

ESC = "\x1b"
UP, DOWN, RIGHT, HOME, DELETE, PAGE = (ESC + s for s in ("[A", "[B", "[C", "[H", "[3~", "[6~"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = {"status": "failed", "cases": [], "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "scope": "Actual native keyboard UI, real local execution; deterministic loopback planning is not live-provider conformance."}
    sessions = []
    try:
        for size in ((120, 36), (60, 18)):
            with tempfile.TemporaryDirectory(prefix="ev-relays-漢字-") as temporary, NativeFixture() as fixture:
                fixture.responses = ("Criterion alpha\nCriterion beta\nCriterion gamma\nApplied reviewed replacement to calc.py. (Untrusted native proposal only; no file write.)", "unused proposal")
                root = Path(temporary).resolve()
                project, data = root / "project", root / "data"
                project.mkdir()
                source = project / "calc.py"
                original = "def total(a, b):\n    return a + b\n\n\ndef other(a):\n    return a * 2\n"
                source.write_text(original)
                (project / "check.py").write_text("from calc import total, other\nassert total(7, 2) == 9\nassert other(3) == 6\nprint('actual checks passed', flush=True)\n")
                for argv in (("init", "-q"), ("add", "--", "calc.py", "check.py"),
                             ("-c", "user.name=Smoke", "-c", "user.email=smoke@example.invalid", "commit", "-qm", "fixture")):
                    subprocess.run(["git", "-c", "core.hooksPath=", *argv], cwd=project, check=True, capture_output=True, timeout=15)
                database = data / "workbench.sqlite3"
                env = terminal_environment("mono")
                for key in list(env):
                    if key.startswith(("EV_", "OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GOOGLE_", "GEMINI_")) or key.lower().endswith("_proxy"):
                        env.pop(key, None)
                home = root / "home"
                home.mkdir()
                runtime, control = root / "runtime", root / "control"
                runtime.mkdir()
                control.mkdir()
                peer_log = root / "peer.jsonl"
                peer_log.touch()
                peer_script = Path(__file__).with_name("controlled_peer_fixture.py").resolve()
                executable = runtime / "codex"
                executable.write_text("#!/bin/sh\nexec " + shlex.quote(sys.executable) + " " + shlex.quote(str(peer_script)) + " --fake-peer \"$@\"\n")
                executable.chmod(0o700)
                controlled = original.replace("return a + b", "return (a + b)")
                env.update(PATH=str(runtime) + os.pathsep + env.get("PATH", ""), CODEX_API_KEY="relay-fixture-not-real",
                           EV_COOP_CONTROL=str(control), EV_COOP_LOG=str(peer_log), EV_COOP_EDIT_PATH="calc.py", EV_COOP_EDIT_CONTENT=controlled)
                env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / "config"),
                           EV_PROVIDER="compatible", EV_API_BASE_URL=f"http://127.0.0.1:{fixture.server.server_port}/v1",
                           EV_API_PROTOCOL="chat_completions", EV_API_MODEL="relay-fixture", EV_API_KEYLESS="1",
                           NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")

                def rows():
                    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=10)) as db:
                        db.row_factory = sqlite3.Row
                        task = dict(db.execute("SELECT * FROM tasks ORDER BY id LIMIT 1").fetchone())
                        runs = [json.loads(r[0]) for r in db.execute("SELECT record FROM executions ORDER BY rowid")]
                        pins = [dict(r) for r in db.execute("SELECT * FROM excerpts ORDER BY id")]
                        task["_partners"] = [dict(r) for r in db.execute("SELECT * FROM partners ORDER BY id")]
                    for field in ("workflows", "criteria", "partner_drafts", "workflow_draft"):
                        task[field] = json.loads(task[field])
                    return task, runs, pins

                def until(predicate, reason, timeout=35):
                    deadline = time.monotonic() + timeout
                    while not predicate():
                        assert session.alive() and time.monotonic() < deadline, reason + "\n" + session.display()
                        session.pump(.05)

                def launch(suffix):
                    argv = [str(binary), "--data-dir", str(data), "--images", "off", "--color", "mono"]
                    if size[0] == 60:
                        argv.append("--ascii")
                    s = Session(argv, project, env, size, output / f"{size[0]}-{suffix}")
                    sessions.append(s)
                    wait_for(s, "Storage: saved", timeout=35)
                    return s

                def save_wait():
                    wait_for(session, "Storage: saved", timeout=35)
                    session.pump(.7)

                def desk(tab):
                    session.command("workflow-" + tab)
                    wait_for(session, "Specialist desk")

                def field(command, text):
                    session.command(command)
                    session.send(UP * 100 + HOME + DELETE * 512)
                    session.paste(text)
                    session.send("\x13")

                def pin(start, end, count):
                    session.command("files")
                    wait_for(session, "eligible files /", timeout=35)
                    session.command("search")
                    session.send(HOME + DELETE * 100)
                    session.paste("calc.py")
                    session.send("\r")
                    wait_for(session, "1 eligible files /", timeout=35)
                    session.send("\r")
                    wait_for(session, "Review local capture /")
                    session.command("range")
                    session.send(HOME + DELETE * 30)
                    session.paste(f"{start}:{end}")
                    session.send("\r")
                    session.command("attach")
                    until(lambda: len(rows()[2]) == count and any(
                        record["path"] == "calc.py" and record["start_line"] == start and record["end_line"] == end
                        for row in rows()[2] for record in [json.loads(row["record"])]),
                        "exact reviewed attachment did not commit")
                    workspace()
                    save_wait()

                command = json.dumps([str(Path(sys.executable).resolve()), "-B", str(project / "check.py")])

                def run(count):
                    session.command("jolteon")
                    session.command("command-edit")
                    wait_for(session, "Command / explicit JSON argv or shell")
                    session.send(UP * 100 + HOME + DELETE * 512)
                    session.paste(command)
                    session.send("\r")
                    until(lambda: rows()[0]["command"] == command and "Storage: saved" in session.display(),
                          "exact local command did not commit before review")
                    session.command("execute")
                    wait_for(session, "Review one-shot execution permission", timeout=35)
                    session.send(RIGHT + "\r")
                    until(lambda: len(rows()[1]) == count and rows()[1][-1]["state"] == "Succeeded", "actual check did not pass")
                    wait_for(session, "Local command evidence; no command running", timeout=35)
                    session.send(ESC)

                def workspace():
                    for _ in range(12):
                        footer = session.display().rstrip().splitlines()[-1].strip()
                        if footer.startswith(("Type Request |", "Enter Send |")):
                            return footer
                        session.send(ESC)
                        session.pump(.15)
                    raise AssertionError("workspace not reached\n" + session.display())

                def clear_request(partner):
                    workspace()
                    session.command(f"lead {partner}")
                    until(lambda: rows()[0]["lead_partner"] == partner, "draft owner was not fronted")
                    footer = workspace()
                    if footer.startswith("Type Request |"):
                        session.send("\r")  # Open the Request editor; never send its contents.
                    session.send(UP * 100 + HOME + DELETE * 512 + "\x13")
                    until(lambda: not any(d["text"] for d in rows()[0]["partner_drafts"].get(str(partner), {}).get("drafts", []) if d["field"] == "Request"), "recipient Request was not explicitly cleared")
                    save_wait()

                def relay_to_second(cancel=False):
                    wait_for(session, "Only idle partners")
                    assert rows()[0]["lead_partner"] == 1, "relay unexpectedly changed the sender"
                    session.send(UP * 100 + DOWN + "\r")
                    wait_for(session, "Stage references")
                    session.capture("relay-review-" + str(len(result["cases"])))
                    session.send(ESC if cancel else "y")
                    save_wait()

                session = launch("initial")
                session.command("goal")
                session.paste("Relay acceptance task")
                session.send("\r")
                session.command("criteria-edit")
                session.paste("Existing behavior remains unchanged")
                session.send("\r")
                save_wait()
                pin(2, 2, 1)
                pin(6, 6, 2)
                run(1)

                # Prepare a real retained pre-refactor comparison before edits.
                desk("refactor")
                field("edit-contract", "Preserve both observed numeric results")
                session.command("refactor-hotspot")
                session.send("\r")
                session.command("refactor-baseline")
                wait_for(session, "eligible passing baseline")
                session.capture("eligible-baseline-age-revision")
                session.send("\r")
                session.command("record-refactor")
                until(lambda: rows()[0]["workflows"]["refactor"] is not None, "retained baseline not recorded")
                result["cases"].append(f"EV16 retained passing baseline with age/revision {size}")

                # PT10 recipient and preserved Unicode sender draft/caret.
                session.command("party")
                session.command("add")
                until(lambda: len(rows()[0]["_partners"]) == 2, "second partner was not durably added")
                session.command("lead 2")
                session.command("flareon")
                session.command("lead 1")
                session.command("umbreon")
                workspace()
                session.paste("sender draft 漢字")
                session.send("\x13")
                save_wait()
                sender = rows()[0]["partner_drafts"]["1"]
                desk("diagnosis")
                field("edit-hypothesis", "The selected calculation lines determine the observed results")
                session.command("add-hypothesis")
                session.command("specialist-fix")
                wait_for(session, "hypothesis must be supported")
                session.command("hypothesis-evidence")
                session.send("\r")
                session.command("hypothesis-evidence")
                session.send(DOWN * 2 + "\r")
                save_wait()
                session.command("specialist-fix")
                before = rows()[0]["workflows"]
                relay_to_second(cancel=True)
                assert rows()[0]["workflows"] == before
                session.command("specialist-fix")
                relay_to_second()
                until(lambda: any("Fix hypothesis" in d["text"] for d in rows()[0]["partner_drafts"]["2"]["drafts"]), "recipient missing diagnosis reference")
                assert rows()[0]["partner_drafts"]["1"] == sender
                assert fixture.snapshot("POST") == []
                result["cases"].append(f"EV12/PT10 supported-only cancel/send sender-caret retained no network {size}")

                # Reuse the deterministic JSONL peer, but cross the actual owned
                # Codex write callback/approval. No synthetic applied journal rows.
                marker = "COOP_HYPOTHESIS_EDIT"
                (control / (marker + ".prompt")).write_text("edit")
                session.command("lead 2")
                session.command("provider-codex")
                session.command("role-implement")
                session.command("paths")
                wait_for(session, "JSON", timeout=20)
                session.send(HOME + DELETE * 512)
                session.paste('["calc.py"]')
                session.send("\r")
                workspace()
                session.send(" ")
                session.paste("\n" + marker)
                session.send("\r")
                deadline = time.monotonic() + 40
                while not any(json.loads(line).get("kind") == "started" for line in peer_log.read_text().splitlines()):
                    assert session.alive() and time.monotonic() < deadline, session.display()
                    if "y Run" in session.display():
                        session.capture("controlled-hypothesis-provider-review")
                        session.send("y")
                    elif "1 pending approvals" in session.display() and "/approvals" in session.display():
                        session.command("approvals")
                    session.pump(.05)
                until(lambda: any(json.loads(line).get("kind") == "prompt" for line in peer_log.read_text().splitlines()), "controlled edit was not requested")
                deadline = time.monotonic() + 30
                while "a Allow once" not in session.display():
                    assert session.alive() and time.monotonic() < deadline, session.display()
                    session.command("response")
                    session.pump(.1)
                session.capture("actual-controlled-file-permission")
                session.send("a")
                until(lambda: source.read_text() == controlled, "actual Eevee-controlled callback did not write selected source")
                (control / (marker + ".finish")).touch()
                until(lambda: any(json.loads(line).get("kind") == "finished" for line in peer_log.read_text().splitlines()), "controlled turn did not finish")
                until(lambda: bool(rows()[0]["workflows"].get("patch_authors")), "successful controlled-write author was not retained after completion")
                session.command("glaceon")
                session.command("lead 1")
                desk("review")
                session.command("refresh-evidence")
                session.pump(1)
                session.command("managed-review")
                wait_for(session, "same partner")
                session.capture("hypothesis-controlled-author-same-partner")
                session.send(ESC)
                session.command("lead 2")
                session.command("flareon")
                clear_request(2)
                session.command("lead 1")
                pin(2, 2, 3)
                pin(6, 6, 4)
                run(2)
                result["cases"].append(f"PT10 actual mediated hypothesis patch author labelled; deterministic peer not live conformance {size}")

                # Two range-specific findings in the same file; clear recipient explicitly.
                clear_request(2)
                session.command("lead 1")
                session.command("glaceon")
                desk("review")
                for index in range(2):
                    field("edit-finding", f"Finding for selected source range {index + 1}")
                    field("edit-recommendation", "Make the expression explicit without changing the result")
                    session.command("finding-evidence")
                    session.send(UP * 100 + DOWN * (index + 2) + "\r")
                    session.command("record-finding")
                    until(lambda: len(rows()[0]["workflows"]["findings"]) == index + 1, "range finding was not recorded")
                    session.command("verdict-changes")
                    save_wait()
                    before = rows()[0]["workflows"]
                    session.command("specialist-fix")
                    relay_to_second(cancel=True)
                    assert rows()[0]["workflows"] == before
                    session.command("specialist-fix")
                    relay_to_second()
                    until(lambda: len(rows()[0]["workflows"]["relayed_findings"]) == index + 1, "selected finding relay was not retained")
                    clear_request(2)
                    session.command("lead 1")
                    desk("review")
                source.write_text(controlled.replace("return (a + b)", "return a + (b)"))
                session.command("refresh-evidence")
                session.pump(1)
                desk("review")
                until(lambda: rows()[0]["workflows"].get("review_stale") is True, "changed source range did not stale review")
                touched = rows()[0]["workflows"]["relayed_findings"]
                assert [r["addressed"] for r in touched] == [True, False], touched
                assert all(f["resolution"] is None for f in rows()[0]["workflows"]["findings"])
                session.capture("range-specific-needs-rereview")
                session.command("lead 2")
                session.command("glaceon")
                session.command("lead 1")
                desk("review")
                session.command("managed-review")
                wait_for(session, "Only idle partners")
                assert "same partner" not in session.display(), "external edit invented controlled authorship"
                session.capture("external-edit-author-unknown")
                session.send(ESC)
                result["cases"].append(f"EV13A cancellation; EV13B one-of-two ranges stale never closed; external author unknown {size}")

                run(3)
                desk("refactor")
                session.command("refactor-post")
                session.send(UP * 100 + "\r")  # Newest actual post-change run, not a retained old selection.
                session.command("compare-refactor")
                comparison = rows()[0]["workflows"]["refactor"]
                assert comparison["baseline_run"] != comparison["post_run"]
                runs_by_id = {r["prepared"]["run_id"]: r for r in rows()[1]}
                before, after = (runs_by_id[comparison[key]] for key in ("baseline_run", "post_run"))
                unseen = {
                    "Baseline " + comparison["baseline_run"],
                    "Post-change " + comparison["post_run"],
                    f"Some({before['ended_at_ms'] - before['started_at_ms']})",
                    f"Some({after['ended_at_ms'] - after['started_at_ms']})",
                    "Single-run times cannot establish speedup.",
                }
                for page in range(40):
                    visible = session.display()
                    found = {fact for fact in unseen if fact in visible}
                    if found:
                        session.capture(f"comparison-facts-page-{page}")
                        unseen -= found
                    if not unseen:
                        break
                    session.send(PAGE)
                assert not unseen, "comparison facts inaccessible by keyboard: " + repr(unseen) + "\n" + session.display()
                session.capture("distinct-before-after-duration")
                session.command("refactor-baseline")
                session.send(UP * 100 + DOWN * 2 + "\r")  # Deliberately choose the pre-change run.
                wait_for(session, "revision differs")
                session.send(ESC)
                session.command("jolteon")
                session.command("command-edit")
                session.send(UP * 100 + HOME + DELETE * 512)
                different = json.dumps([str(Path(sys.executable).resolve()), "-B", str(project / "check.py"), "--different-argv"])
                session.paste(different)
                session.send("\r")
                until(lambda: rows()[0]["command"] == different and "Storage: saved" in session.display(),
                      "different exact argv did not commit before baseline inspection")
                desk("refactor")
                session.command("refactor-baseline")
                wait_for(session, "argv differs")
                session.send("\r")
                wait_for(session, "argv differs")
                session.send(ESC)
                session.command("approvals")
                wait_for(session, "Review one-shot execution permission")
                session.command("deny")
                desk("refactor")
                session.command("fresh-baseline")
                wait_for(session, "Command / explicit JSON argv or shell")
                assert len(rows()[1]) == 3, "fresh fallback executed before approval"
                session.send(ESC)
                result["cases"].append(f"EV16 revision refusal distinct run timing fresh fallback approval {size}")

                # Actual loopback response becomes canonical only via /decision + save.
                clear_request(1)
                session.command("espeon")
                # Prior patch/refresh fixtures deliberately made every old capture stale.
                # Remove those selected references through normal reviewed UI before a
                # separate source-free design request; never bypass dispatch freshness.
                session.command("files")
                wait_for(session, "eligible files /", timeout=35)
                session.command("basket")
                for remaining in range(len(rows()[2]) - 1, -1, -1):
                    session.send(UP * 100)
                    session.command("unpin")
                    wait_for(session, "Remove selected basket reference")
                    session.send("y")
                    until(lambda: len(rows()[2]) == remaining, "stale selection was not explicitly removed")
                    wait_for(session, f"{remaining} task-owned basket references / stored", timeout=35)
                session.command("close")
                workspace()
                session.command("provider-compatible")
                session.pump(.2)
                if "y Switch" in session.display():
                    session.send("y")
                workspace()
                session.paste("Propose three acceptance criteria")
                session.send("\r")
                deadline, approved_connection = time.monotonic() + 35, False
                while "Criterion gamma" not in session.display():
                    assert session.alive() and time.monotonic() < deadline, session.display()
                    if "y Run" in session.display() and not approved_connection:
                        session.send("y")
                        approved_connection = True
                    elif "Applied reviewed" in session.display():
                        # Compact follow shows the actual tail, not the earlier criteria.
                        # Inspect preceding text instead of requiring it in the last page.
                        session.send(ESC + "[5~")
                    else:
                        session.pump(.05)
                session.command("decision")
                wait_for(session, "Next action")
                session.send("\r")
                until(lambda: rows()[0]["workflows"].get("chosen_decision") is not None, "explicit decision was not canonical")
                desk("recipe")
                session.command("decision-criteria")
                session.send(UP * 100 + HOME + DELETE * 512)
                session.paste("Criterion alpha\nCriterion beta edited\nCriterion gamma\nCriterion alpha")
                session.send("\x13")
                session.command("confirm-decision-criteria")
                session.send(PAGE)
                wait_for(session, "duplicate")
                session.send(ESC)
                assert len(rows()[0]["criteria"]) == 1
                session.command("confirm-decision-criteria")
                session.send("y")
                until(lambda: len(rows()[0]["criteria"]) == 4, "three confirmed criteria not saved")
                origins = rows()[0]["workflows"]["criterion_origins"]
                assert len(origins) == 3 and len({o["decision_id"] for o in origins}) == 1
                session.command("criteria")
                session.send(UP * 20 + "\r")
                until(lambda: rows()[0]["criteria"][0]["met"], "criterion met/total did not update")
                desk("recipe")
                session.capture("decision-linked-criteria-confirmed")
                session.command("party")
                session.command("add")
                until(lambda: len(rows()[0]["_partners"]) == 3, "idle Eevee recipient was not added")
                desk("recipe")
                criteria_before_relay = rows()[0]["criteria"]
                session.command("relay-decision")
                relay_to_second(cancel=True)
                assert rows()[0]["criteria"] == criteria_before_relay
                session.command("relay-decision")
                relay_to_second()
                until(lambda: any("Chosen decision" in d["text"] for d in rows()[0]["partner_drafts"]["3"]["drafts"]), "idle Eevee missing decision references")
                assert rows()[0]["criteria"] == criteria_before_relay
                assert len(fixture.snapshot("POST")) == 1
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()
                session = launch("restart")
                assert rows()[0]["workflows"]["criterion_origins"] == origins
                assert any(c["text"] == "Criterion beta edited" and not c["met"] for c in rows()[0]["criteria"])
                assert sum(c["met"] for c in rows()[0]["criteria"]) == 1
                assert len(fixture.snapshot("POST")) == 1, "relay or restart called the provider"
                assert len(rows()[0]["workflows"].get("patch_authors", [])) == 1, "native proposal invented another patch author"
                session.capture("restart-provenance-retained")
                result["cases"].append(f"EV15 selected decision three/edit/duplicate/cancel/restart provenance {size}")
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()
        result["status"] = "passed"
    except Exception as error:
        result["error"] = repr(error)
        raise
    finally:
        for session in sessions:
            if not session.closed:
                session.capture("final-failure")
                session.close()
        (output / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
