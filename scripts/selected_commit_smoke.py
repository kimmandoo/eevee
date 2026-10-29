#!/usr/bin/env python3
"""Opt-in GP07 real TUI/owned-runner/SQLite/local-Git acceptance.

Requires an already-built ev and existing pyte dependencies. Every Git mutation
is confined to CooperationJourney's disposable temporary repository; never the
checkout or a user repository. No paid/authenticated/live provider calls. The
single controlled JSONL peer is the existing local cooperation fixture.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import sqlite3
import subprocess
import time

from cooperation_smoke import Journey as CooperationJourney, ESC, HOME, UP, RIGHT, LEFT, DELETE
from response_smoke import SurfaceSession, PAGE_DOWN
from workbench_smoke import wait_for

TITLE = "Review one-shot selected LOCAL commit"


class Journey(CooperationJourney):
    def launch(self, suffix):
        session = SurfaceSession(self.argv, self.project, self.environment, self.size, self.output / suffix)
        wait_for(session, "Storage: saved", timeout=40)
        return session

    def git(self, *args, input=None):
        environment = {key: value for key, value in self.environment.items() if not key.startswith("GIT_")}
        environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_SYSTEM="/dev/null", GIT_CONFIG_GLOBAL="/dev/null")
        result = subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false",
                                 "-c", "commit.gpgSign=false", "-c", "core.autocrlf=false", *args],
                                cwd=self.project, env=environment, input=input, capture_output=True, timeout=15)
        assert result.returncode == 0, result.stderr.decode(errors="replace")
        return result.stdout

    def head(self):
        return self.git("rev-parse", "HEAD").decode().strip()

    def runs(self):
        with closing(sqlite3.connect(self.database.as_uri() + "?mode=ro", uri=True)) as db:
            return [(json.loads(record), bool(reserved)) for record, reserved in db.execute("SELECT record,reserved FROM executions ORDER BY rowid")]

    def author(self, paths, message):
        self.workspace()
        previous = self.snapshot()[0]["command"]
        draft = next((row for row in self.snapshot()[0]["partner_drafts"].get(str(self.snapshot()[0]["lead_partner"]), {}).get("drafts", []) if row["field"] == "Command"), None)
        if draft is not None:
            previous = draft["text"]
        self.session.command("commit-edit")  # Discoverable real palette, not a test-only direct state hook.
        wait_for(self.session, "Command / explicit JSON argv or shell")
        # Empty fields can receive the ordinary /commit-edit syntax template.
        self.session.send(UP * 100 + HOME)
        count = max(len(previous), len('{"selected_commit":[],"message":""}'))
        for _ in range(0, count + 32, 32):
            self.session.send(DELETE * 32)
        value = json.dumps({"selected_commit": paths, "message": message}, ensure_ascii=False)
        self.session.paste(value)
        self.session.send("\r")
        self.until(lambda: self.snapshot()[0]["command"] == value, "exact selected commit authoring did not commit")
        return value

    def review(self):
        # This only revisits pending read-only preparation; no affirmative keys.
        for _ in range(30):
            self.session.command("execute")
            if TITLE in self.session.display():
                return
            self.session.pump(.1)
        raise AssertionError("selected commit review was not reachable\n" + self.session.display())

    def expose(self):
        deadline = time.monotonic() + 30
        previous = -1
        while True:
            assert time.monotonic() < deadline, "complete selected review did not become eligible\n" + self.session.display()
            matches = re.findall(r"Reviewed (\d+)/(\d+) rows", self.session.display())
            if not matches:
                self.session.pump(.05)
                continue
            covered, total = map(int, matches[-1])
            if covered == total:
                return total
            assert covered >= previous, "coverage unexpectedly moved backwards on an unchanged visible review"
            previous = covered
            self.session.send(PAGE_DOWN)

    def approve(self, before, expected="Succeeded"):
        self.expose()
        self.session.send(RIGHT + "\r")
        self.until(lambda: len(self.runs()) > before and not self.runs()[-1][1], "selected commit terminal journal did not commit")
        record = self.runs()[-1][0]
        assert record["state"] == expected, record
        assert record["tree"] == "OwnedTreeExited", record
        self.until(lambda: not self.claims(), "selected commit writer was released before/without final evidence")
        return record


def run(binary, output, size, keep):
    j = Journey(binary, output, size, "selected-commit")
    result = {"status": "failed", "size": list(size), "cases": [],
              "scope": "Actual native TUI, one-shot writer/runner, committed SQLite, disposable local Git, controlled local JSONL peer; no live provider calls"}
    try:
        j.setup(count=2)
        j.git("config", "user.name", "GP07 actual TUI fixture")
        j.git("config", "user.email", "fixture@example.invalid")
        other = j.project / "other dirty.txt"
        other.write_bytes(b"base other\n")
        j.git("add", "--", other.name)
        j.git("commit", "-qm", "second disposable fixture file")
        other_bytes = b"exact other unselected bytes\x00\xff\r\n"
        other.write_bytes(other_bytes)
        selected = j.original.replace("a + b", "a - b").encode()
        j.source.write_bytes(selected)
        marker = j.root / "MUST-NOT-EXECUTE"
        helper = j.root / "marker-helper"
        helper.write_text("#!/bin/sh\nprintf invoked >> " + shlex.quote(str(marker)) + "\nexit 99\n")
        helper.chmod(0o700)
        hooks = j.project / ".git" / "hooks"
        for name in ("pre-commit", "prepare-commit-msg", "commit-msg", "post-commit", "reference-transaction"):
            target = hooks / name
            target.write_bytes(helper.read_bytes())
            target.chmod(0o700)
        for key, value in (("core.hooksPath", str(hooks)), ("filter.tripwire.clean", str(helper)),
                           ("filter.tripwire.smudge", str(helper)), ("filter.tripwire.process", str(helper)),
                           ("filter.tripwire.required", "true"), ("commit.gpgSign", "true"),
                           ("gpg.program", str(helper)), ("core.fsmonitor", str(helper)),
                           ("core.sshCommand", str(helper)), ("credential.helper", str(helper)),
                           ("diff.external", str(helper))):
            j.git("config", key, value)
        (j.project / ".gitattributes").write_text("* filter=tripwire diff=tripwire\n")
        message = "Exact selected local message 東京\n\nNo shell interpolation: $(touch MUST-NOT-EXECUTE)\n" + "Bounded long-message review exposure. " * 18 + "No final newline"
        original_owner = j.snapshot()[0]["lead_partner"]
        head = j.head()
        request_draft = "Unsent GP07 request draft 東京 exact caret tail"
        if j.workspace().startswith("Type Request |"):
            j.session.send("\r")
        j.session.paste(request_draft)
        j.session.send(LEFT * 3)
        j.author(["calc.py"], message)
        expected_draft = j.draft(original_owner)
        assert expected_draft is not None and expected_draft["text"] == request_draft
        assert expected_draft["cursor"] == len(request_draft.encode()) - 3
        j.review()
        j.session.send("y")  # Incomplete hidden rows cannot consent.
        assert j.head() == head and not j.runs()
        j.expose()
        j.session.capture("fully-exposed-neutral-review")
        j.session.send("\r")  # Neutral Enter is Stay, even after exposure.
        assert j.head() == head and not j.runs()
        j.review()
        j.expose()
        # An obscuring chooser and a resize both destroy coverage and selection.
        j.session.send("\x0b")
        j.session.send("y")
        assert j.head() == head and not j.runs()
        j.session.send(ESC)
        j.session.resize((120, 36) if size[0] == 60 else (60, 18))
        j.session.pump(.2)
        j.session.send("y")
        assert j.head() == head and not j.runs()
        j.session.resize(size)
        j.session.pump(.2)
        j.session.capture("obscured-resized-consent-invalidated")
        record = j.approve(0)
        assert record["exit_code"] is None, "a Git transaction must not fabricate one OS exit code"
        outcome = record["selected_commit"]
        assert outcome["created_commit"] == outcome["observed_head"] == outcome["observed_reference"] == j.head()
        assert outcome["ref_published"] and outcome["index_published"]
        assert outcome["observed_index_sha256"] == outcome["expected_index_sha256"]
        assert j.git("show", "HEAD:calc.py") == selected
        assert j.git("show", "HEAD:" + other.name) == b"base other\n"
        assert other.read_bytes() == other_bytes
        assert j.git("diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD") == b"calc.py\n"
        assert j.git("cat-file", "commit", "HEAD").split(b"\n\n", 1)[1] == message.encode()
        assert not marker.exists() and not j.peers("started")
        assert j.snapshot()[0]["lead_partner"] == original_owner and not j.snapshot()[0]["complete"]
        assert j.draft(original_owner) == expected_draft, "local authoring/review changed the retained Request draft or caret"
        result["cases"].append("palette authoring preserves exact task/partner/draft/caret; exact long message, all-row neutral consent, obscuring/resize invalidation, selected-only commit, unselected bytes preserved, hooks/filter/signing/helpers excluded, actual durable ref/index evidence; no verification/automatic completion")

        for drift in ("source", "index", "config", "HEAD", "merge", "reflog"):
            j.source.write_bytes(selected + ("# newly authored " + drift + "\n").encode())
            j.author(["calc.py"], "Deliberately authored refusal fixture " + drift + "\n")
            j.review()
            j.expose()
            before = len(j.runs())
            initial_head = j.head()
            index_bytes = (j.project / ".git/index").read_bytes()
            if drift == "source":
                j.source.write_bytes(b"changed after review\n")
            elif drift == "index":
                (j.project / ".git/index").write_bytes(index_bytes + b"controlled index drift")
            elif drift == "config":
                j.git("config", "user.name", "Changed after reviewed identity")
            elif drift == "HEAD":
                reference = j.git("symbolic-ref", "HEAD").decode().strip()
                previous = j.git("rev-parse", "HEAD^").decode().strip()
                j.git("update-ref", reference, previous, initial_head)
            elif drift == "reflog":
                reference = j.git("symbolic-ref", "HEAD").decode().strip()
                log = j.project / ".git" / "logs" / reference
                original_log = log.read_bytes()
                log.unlink()
                os.link(other, log)
            else:
                (j.project / ".git/MERGE_HEAD").write_text(initial_head + "\n")
            expected_head = j.head()
            failed = j.approve(before, "Failed")
            assert failed["selected_commit"]["created_commit"] is None
            assert not failed["selected_commit"]["publication_started"] and j.head() == expected_head
            assert other.read_bytes() == other_bytes and not marker.exists()
            j.session.capture("refused-" + drift.lower())
            # Fixture-only external repair is not an app rollback/reset/retry.
            (j.project / ".git/index").write_bytes(index_bytes)
            if drift == "config":
                j.git("config", "user.name", "GP07 actual TUI fixture")
            elif drift == "HEAD":
                j.git("update-ref", reference, initial_head, previous)
            elif drift == "merge":
                (j.project / ".git/MERGE_HEAD").unlink()
            elif drift == "reflog":
                log.unlink()  # Remove the alias BEFORE restoring the fixture's log.
                log.write_bytes(original_log)
        result["cases"].append("HEAD/source/index/configuration/merge/reflog-hardlink drift refuses before object/ref publication; unselected alias target unchanged, durable failed outcomes and no app rollback/retry")

        # Real other-partner writer owns its exact request while local commit
        # preparation/review proceeds read-only; reservation must refuse.
        j.implementation(2)
        j.request(2, "COOP_GP07_WRITER")
        j.until(lambda: any(row["writing"] for row in j.snapshot()[2]), "controlled peer never held its actual writer")
        j.lead(1)
        j.source.write_bytes(selected + b"# explicit selected commit while writer exists\n")
        before = len(j.runs())
        head = j.head()
        j.author(["calc.py"], "Must refuse while another task/partner owns writer\n")
        j.review()
        j.expose()
        j.session.send(RIGHT + "\r")
        j.until(lambda: "Storage / reservation error" in j.session.display(), "reservation failure banner was not visible")
        j.session.command("run-details")  # Advertised ordinary error inspector.
        j.until(lambda: "exclusive project" in " ".join(j.session.display().lower().split()), "writer-exclusive reservation refusal was not visible")
        assert len(j.runs()) == before and j.head() == head and j.claims() == {2}
        j.session.capture("controlled-other-writer-refuses-commit")
        j.session.command("execute")
        j.session.command("deny")
        j.finish("COOP_GP07_WRITER")
        j.until(lambda: not j.claims(), "controlled writer did not settle")
        assert len(j.peers("started", "COOP_GP07_WRITER")) == 1 and not marker.exists()
        result["cases"].append("controlled native peer writer blocks selected-commit reservation, preserves exact partner ownership and creates no run/commit/replay")
        j.quit()
        j.session = j.launch("restarted-evidence-only")
        j.sessions.append(j.session)
        j.session.pump(.3)
        assert len(j.runs()) == before and j.head() == head and not j.claims()
        assert j.draft(original_owner) == expected_draft, "restart or other-owner work reassigned/lost the original Request draft"
        j.session.capture("restart-never-restores-commit-approval")
        result["cases"].append("restart restores committed evidence only, no selected approval/replay or ownership resurrection")
        j.quit()
        result["status"] = "passed"
        return result
    finally:
        # Failure teardown is not acceptance evidence or an app rollback.
        (j.control / "COOP_GP07_WRITER.finish").touch()
        for session in j.sessions:
            if not session.closed:
                session.capture("final")
                session.close()
        j.output.mkdir(parents=True, exist_ok=True)
        if j.database.exists():
            (j.output / "evidence.json").write_text(json.dumps(j.snapshot(), indent=2, ensure_ascii=False) + "\n")
        (j.output / "peer.jsonl").write_text(j.log.read_text())
        if keep:
            j.temporary._finalizer.detach()
            result["retained_fixture_root"] = str(j.root)
        else:
            j.temporary.cleanup()
        (j.output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--keep-fixture", action="store_true", help="Preserve disposable canonical project/data paths for backup and genuine older-reader refusal proof")
    args = parser.parse_args()
    assert os.name == "posix", "Existing controlled-peer driver requires a native POSIX host; Windows Rust transaction tests remain independent"
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "journeys": []}
    try:
        for size in ((120, 36), (60, 18)):
            result["journeys"].append(run(binary, output, size, args.keep_fixture))
        result["status"] = "passed"
    finally:
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
