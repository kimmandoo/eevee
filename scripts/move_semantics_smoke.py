#!/usr/bin/env python3
"""MV09 real PTY journey against an immutable, already-built executable.

Example: python3 scripts/move_semantics_smoke.py --binary /absolute/ev \
  --output /new/evidence --size 120x36 --size 60x18
Uses terminal_smoke.Session and tm_smoke.Driver's
isolated HOME/project/store and read-only observations. No builds, SQL writes,
provider setup, discovery, accounts, inference or external network. All move
history comes from real app import/teach/use/rename actions and real processes.
Stale UI consent is exercised by resize and by abandoning review before a real
rename; exact concurrent writer ID/revision races are covered by Rust regressions.
Optionally pass --previous-binary /path/to/preserved/v0.1.31 to observe its real
admission or refusal after repeated genuine same-name replacements. Refusal must
leave durable database/WAL bytes unchanged. Then current startup and explicit use
are exercised again; the driver does not invent an old-reader compatibility result.
Requires the existing pyte/wcwidth PTY helper dependencies.
"""
import argparse
import json
import hashlib
from pathlib import Path
import platform
import tempfile
import time
import traceback

from terminal_smoke import Session
from tm_smoke import Driver, definition, save, sha, ESC, HOME, DELETE, RIGHT, UP

NAME = "Exact argument"
FIXTURE = '''import json
from pathlib import Path
import os
import sys
with Path("invocations.jsonl").open("a", encoding="utf-8") as out:
    out.write(json.dumps({"argv": sys.argv[1:], "cwd": os.getcwd()}, ensure_ascii=False) + "\\n")
print("REAL-MOVE-COMPLETED", flush=True)
'''


class SemanticsDriver(Driver):
    def launch(self):
        # Local commands and TMs require no selected provider or catalogue peer.
        self.serial += 1
        self.session = Session([str(self.binary), "--continue", "--data-dir", str(self.data),
            "--images", "off", "--color", "mono", ],
            self.project, self.environment, self.size,
            self.output / ("terminal-%02d" % self.serial))
        self.sessions.append(self.session)
        self.until(lambda: "Storage: saved" in self.session.display(), "current process ready")
        self.until(lambda: self.database.is_file() and self.state().get("partners"), "store ready")
        if not self.state().get("tasks"):
            self.session.command("goal")
            self.session.paste("Exact local move semantics acceptance")
            self.session.send("\r")
            self.until(lambda: any(t["goal"] == "Exact local move semantics acceptance"
                       for t in self.state().get("tasks", [])), "actual task saved")
        self.workspace()

    def task(self):
        state = self.state()
        active = state["selections"][0]["active_id"]
        return next(t for t in state["tasks"] if t["id"] == active)

    def invocations(self):
        path = self.project / "invocations.jsonl"
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()] if path.exists() else []

    def no_effect(self, before):
        assert self.effects() == before, "local inventory review caused execution/context/evidence effects"
        assert not (self.project / "UNEXPECTED_EXECUTION").exists(), "unexpected fixture executed"

    def active_move(self):
        rows = self.rows(2)
        assert len(rows) == 1, "this journey owns one active Jolteon move"
        return rows[0]

    def approve_command(self, expected_argument):
        before = len(self.state().get("executions", []))
        calls = len(self.invocations())
        self.session.send(RIGHT + "\r")
        self.until(lambda: len(self.state().get("executions", [])) == before + 1
            and self.state()["executions"][-1]["record"]["state"] == "Succeeded"
            and not self.state()["executions"][-1]["reserved"], "real command terminal journal")
        run_id = self.state()["executions"][-1]["record"]["prepared"]["run_id"]
        self.until(lambda: any(row["id"] == "run:" + run_id
            for row in self.state().get("move_sources", [])), "actual command move-source acknowledgement")
        self.until(lambda: len(self.invocations()) == calls + 1, "real process recorded argv")
        assert self.invocations()[-1] == {"argv": [expected_argument], "cwd": str(self.project)}
        self.capture("ordinary-fresh-consent-real-argv")
        self.workspace()

    def use_move(self, expected_argument):
        self.workspace()
        effects, calls = self.effects(), self.invocations()
        move = self.active_move()
        self.session.command("move")
        self.wait("menu")
        self.session.send("\r")
        self.wait("run")
        assert self.effects() == effects and self.invocations() == calls, "staging inherited approval"
        self.capture("explicit-use-neutral-command-review")
        self.approve_command(expected_argument)
        self.until(lambda: any(u["move_id"] == move["id"] and u["record"]["outcome"] == "Succeeded"
            for u in self.state().get("move_usages", [])), "usage linked to actual original move ID")

    def run_changed_command(self):
        self.workspace()
        command = json.dumps(["python3", "fixture.py", ""])
        self.session.command("command-edit")
        self.until(lambda: "Command /" in self.session.display(), "command editor")
        self.session.send(HOME + DELETE * 256)
        self.session.paste(command)
        self.session.send("\r")
        self.until(lambda: self.task()["command"] == command, "exact changed argv saved")
        self.session.command("execute")
        self.wait("run")
        self.approve_command("")

    def start_import_review(self, path, local_name=None):
        self.import_preview(path)
        if local_name:
            self.session.send(HOME + DELETE * 64)
            self.session.paste(local_name)
        self.session.send("\r")
        self.wait("review")

    def cancel_review(self, before, effects):
        self.session.send("\r")  # The neutral choice, even after all details were seen.
        self.wait("menu")
        self.workspace()
        assert self.state()["moves"] == before, "neutral cancellation mutated inventory"
        self.no_effect(effects)

    def draft(self):
        self.workspace()
        footer = self.session.display().rstrip().splitlines()[-1].strip()
        if footer.startswith(("Type Request |", )):
            self.session.send("\r")
        text = "Keep 水の exact draft"
        self.session.send(UP * 16 + HOME + DELETE * 256)
        self.session.paste(text)
        self.session.send(HOME + RIGHT * 3 + "\x13")
        self.until(lambda: any(d["field"] == "Request" and d["text"] == text
            for d in self.task()["partner_drafts"].get(str(self.task()["lead_partner"]), {}).get("drafts", [])),
            "exact invoking Request saved")
        return text, (self.session.screen.cursor.x, self.session.screen.cursor.y, self.session.screen.cursor.hidden)

    def assert_draft(self, text, caret=None):
        drafts = self.task()["partner_drafts"].get(str(self.task()["lead_partner"]), {}).get("drafts", [])
        assert any(d["field"] == "Request" and d["text"] == text for d in drafts), "Request bytes changed"
        if caret is not None:
            assert (self.session.screen.cursor.x, self.session.screen.cursor.y, self.session.screen.cursor.hidden) == caret, "Request caret/focus changed"

    def rename_current(self, name):
        before = self.active_move()
        self.workspace()
        self.session.command("move")
        self.wait("menu")
        self.session.send("dr")
        label = "Rename (label only)"
        self.until(lambda: label in self.session.display(), "rename editor")
        self.session.send(HOME + DELETE * 64)
        self.session.paste(name)
        self.session.send("\r")
        self.wait("review")
        seen = self.expose_review()
        expected = "Name-only edit"
        assert expected in seen, "rename failed to disclose name-only semantics"
        self.session.send("y")
        self.until(lambda: self.active_move()["proposal"]["name"] == name, "actual concurrent-to-old-review rename committed")
        after = self.active_move()
        assert after["id"] == before["id"] and after["revision"] == before["revision"] + 1
        assert after["proposal"]["steps"] == before["proposal"]["steps"]
        self.wait("menu")
        return after

    def database_bytes(self):
        return {path.name: path.read_bytes() for path in
            (self.database, Path(str(self.database) + "-wal")) if path.exists()}

    def probe_previous(self):
        state = self.state()
        inventory, usages, calls = state["moves"], state["move_usages"], self.invocations()
        before = self.database_bytes()
        baseline_hash = sha(self.previous_binary)
        previous = Session([str(self.previous_binary), "--data-dir", str(self.data),
            "--images", "off", "--color", "mono", ],
            self.project, self.environment, self.size, self.output / "previous-binary")
        self.sessions.append(previous)
        observation = {"binary": str(self.previous_binary), "sha256": baseline_hash,
                       "database_before": {name: {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
                                           for name, data in before.items()},
                       "status": "unobserved", "forced_close": False}
        try:
            deadline = time.monotonic() + 35
            while previous.alive():
                view = previous.display()
                if "Storage: saved" in view:
                    observation["status"] = "admitted"
                    break
                if "Storage: save failed" in view:
                    observation["status"] = "refused"
                    break
                assert time.monotonic() < deadline, "old reader did not expose admission/refusal"
                previous.pump(.05)
            observation["screen"] = previous.display()
            previous.capture("old-reader-admission-observed")
            assert observation["status"] in ("admitted", "refused"), "old process exited without an observed admission result"
            previous.command("quit")
            deadline = time.monotonic() + 8
            while previous.alive() and time.monotonic() < deadline:
                if self.labels["exit"] in previous.display():
                    previous.send("y")
                previous.pump(.05)
            observation["forced_close"] = previous.alive()
            if not previous.alive():
                observation["exit_status"] = previous.wait_exit()
        finally:
            previous.close()
            state = self.state()
            after = self.database_bytes()
            observation["database_bytes_unchanged"] = before == after
            observation["database_after"] = {
                name: {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
                for name, data in after.items()}
            observation["move_history_unchanged"] = (
                state["moves"] == inventory and state["move_usages"] == usages)
            save(self.output / "previous-binary-observation.json", observation)
        assert sha(self.previous_binary) == baseline_hash, "preserved old executable changed"
        if observation["status"] == "refused":
            assert before == after, "old-reader refusal modified durable database/WAL bytes"
        assert observation["move_history_unchanged"], "old startup rewrote move identity/history"
        assert self.invocations() == calls, "old startup dispatched work"
        return observation

    def run(self):
        (self.project / "fixture.py").write_text(FIXTURE, encoding="utf-8")
        old_path, new_path = self.root / "old.toml", self.root / "new.toml"
        new_path.write_text(definition(2, NAME), encoding="utf-8")
        old_path.write_text(definition(2, NAME).replace('"fixture.py", ""', '"fixture.py", "before"'), encoding="utf-8")
        original = self.learn(old_path, 2, name=NAME)
        self.form(2)
        self.use_move("before")
        old_uses = list(self.state()["move_usages"])
        assert old_uses and all(u["move_id"] == original["id"] for u in old_uses)

        # Teach from an actually approved different argv, not seeded execution rows.
        self.run_changed_command()
        before, effects = self.state()["moves"], self.effects()
        self.session.command("teach-move")
        self.wait("name")
        self.session.send(HOME + DELETE * 64)
        self.session.paste(NAME)
        self.session.send("\r")
        self.wait("review")
        seen = self.expose_review()
        assert "argv[2]" in seen and '"before"' in seen and '""' in seen, "single/empty argument difference unavailable"
        self.cancel_review(before, effects)
        self.capture("changed-argument-teaching-cancelled")

        text, caret = self.draft()
        before, effects = self.state()["moves"], self.effects()
        self.start_import_review(new_path)
        seen = self.expose_review()
        assert "argv[2]" in seen and '"before"' in seen and '""' in seen
        assert str(original["id"]) in seen and NAME in seen, "exact local target missing"
        self.cancel_review(before, effects)
        self.assert_draft(text, caret)
        self.capture("same-name-tm-neutral-cancel-exact-draft")
        self.quit()
        self.launch()
        assert self.state()["moves"] == before and self.state()["move_usages"] == old_uses
        self.assert_draft(text)
        self.capture("cancel-restart-no-replay")

        # A hidden/reflowed complete-scope receipt is not reusable consent.
        self.start_import_review(new_path)
        self.expose_review()
        before = self.state()["moves"]
        self.session.send(RIGHT)  # Arm the old surface, but do not confirm it.
        self.session.resize((59, 17))
        self.session.send("y" + RIGHT + "\r")
        assert self.state()["moves"] == before, "undersize review committed"
        self.session.resize(self.size)
        self.wait("review")
        self.session.send("\r")  # Resize revoked the old affirmative choice.
        self.wait("menu")
        assert self.state()["moves"] == before, "stale hidden review reused consent"
        self.capture("stale-resize-consent-refused")
        self.session.send(ESC)
        self.workspace()

        # Leaving a review and actually renaming its old record revokes the old
        # review, rather than reusing it against the renamed current revision.
        renamed = self.rename_current("Renamed exact")
        renamed_inventory = self.state()["moves"]
        self.session.send("y")  # Old semantic confirmation, now at the menu.
        assert self.state()["moves"] == renamed_inventory, "abandoned old revision was committed"
        self.capture("old-review-revoked-after-real-rename")
        self.workspace()
        self.start_import_review(new_path, local_name="Renamed exact")
        seen = self.expose_review()
        assert "argv[2]" in seen and '"before"' in seen and '""' in seen
        self.session.send("y")
        self.until(lambda: self.active_move()["id"] != renamed["id"], "fresh exact replacement committed")
        replacement = self.active_move()
        retained = next(r for r in self.state()["moves"] if r["id"] == renamed["id"])
        assert retained["forgotten"] and retained["proposal"] == renamed["proposal"]
        assert replacement["proposal"]["steps"]["Run"]["argv"] == ["python3", "fixture.py", ""]
        assert self.state()["move_usages"] == old_uses, "replacement rewrote historical use attribution"
        self.wait("menu")
        self.workspace()
        self.capture("same-name-new-record-forgotten-original")
        self.use_move("")
        all_uses = self.state()["move_usages"]
        assert {u["move_id"] for u in all_uses} == {original["id"], replacement["id"]}
        repeated_path = self.root / "repeated.toml"
        repeated_path.write_text(definition(2, NAME).replace(
            '"fixture.py", ""', '"fixture.py", "after"'), encoding="utf-8")
        self.start_import_review(repeated_path, local_name="Renamed exact")
        self.expose_review()
        self.session.send("y")
        self.until(lambda: self.active_move()["id"] != replacement["id"], "second genuine same-name replacement committed")
        latest = self.active_move()
        inventory, calls = self.state()["moves"], self.invocations()
        assert len(inventory) == 3 and sum(bool(r["forgotten"]) for r in inventory) == 2
        assert len({r["proposal"]["name"] for r in inventory}) == 1
        assert self.state()["move_usages"] == all_uses
        self.wait("menu")
        self.workspace()
        self.capture("repeated-same-name-history-before-old-reader")
        self.quit()
        if self.previous_binary:
            self.previous_observation = self.probe_previous()
        self.launch()
        assert self.state()["moves"] == inventory and self.state()["move_usages"] == all_uses
        assert self.invocations() == calls, "restart replayed a command"
        self.capture("replacement-and-attributed-history-restart")
        self.use_move("after")
        final_uses = self.state()["move_usages"]
        assert {u["move_id"] for u in final_uses} == {original["id"], replacement["id"], latest["id"]}
        save(self.output / "actual-invocations.json", self.invocations())
        self.quit()


def terminal_size(value):
    try:
        width, height = (int(part) for part in value.lower().split("x"))
    except (ValueError, TypeError):
        raise argparse.ArgumentTypeError("use WIDTHxHEIGHT")
    if width < 60 or height < 18:
        raise argparse.ArgumentTypeError("main review size must be at least 60x18; undersize is exercised separately")
    return width, height


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="New evidence directory; never overwrite")
    parser.add_argument("--previous-binary", type=Path, help="Optional preserved genuine prior release; observe admission/refusal without forcing either result")
    parser.add_argument("--size", action="append", type=terminal_size, help="Repeatable WIDTHxHEIGHT; defaults to 120x36 and 60x18")
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    assert binary.is_file(), "--binary must be an existing immutable executable"
    previous = args.previous_binary.resolve() if args.previous_binary else None
    if previous:
        assert previous.is_file() and previous != binary, "--previous-binary must be a distinct existing preserved executable"
    output.mkdir(parents=True, exist_ok=False)
    digest = sha(binary)
    result = {"status": "failed", "binary": str(binary), "sha256": digest,
        "host": platform.platform(), "sql_writes": False,
        "scope": "MV09 real PTY/local commands, changed argv, same-name import, cancel, restart, stale UI consent and fresh explicit use; no accounts/network/models",
        "cases": []}
    if previous:
        result["previous_binary"] = {"path": str(previous), "sha256": sha(previous)}
    try:
        for size in args.size or [(120, 36), (60, 18)]:
            case = {"size": list(size), "status": "failed"}
            result["cases"].append(case)
            directory = output / ("%dx%d" % size)
            directory.mkdir()
            driver = None
            with tempfile.TemporaryDirectory(prefix="ev-move-semantics-") as temporary:
                try:
                    driver = SemanticsDriver.__new__(SemanticsDriver)
                    driver.__init__(binary, directory, Path(temporary).resolve(), size, None)
                    driver.previous_binary = previous
                    driver.previous_observation = None
                    driver.run()
                    case["status"] = "passed"
                    if driver.previous_observation is not None:
                        case["previous_binary"] = driver.previous_observation
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
            assert sha(binary) == digest, "immutable binary changed during acceptance"
            save(output / "results.json", result)
        result["status"] = "passed" if all(case["status"] == "passed" for case in result["cases"]) else "failed"
    finally:
        save(output / "results.json", result)
    print(json.dumps({"status": result["status"], "results": str(output / "results.json")}))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
