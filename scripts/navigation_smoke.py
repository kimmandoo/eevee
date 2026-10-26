#!/usr/bin/env python3
"""Exercise shallow navigation with a real isolated PTY, SQLite and loopback HTTP.

No account or model service is used. --baseline records predecessor detours;
normal mode requires direct return without Escape-unwinding helpers.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import tempfile

from native_connection_smoke import MODEL, until
from response_smoke import DRAFT, ESC, StreamingPeer, SurfaceJourney, saved_request
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

OTHER_MODEL = "navigation-second-model"


class NavigationPeer(StreamingPeer):
    def __init__(self):
        super().__init__()
        inherited = self.server.RequestHandlerClass

        class Handler(inherited):
            def do_GET(self):
                self.record(None)
                assert self.path == "/v1/models"
                self.reply(200, {"data": [
                    {"id": model, "supported_reasoning_levels": ["low", "high"]}
                    for model in (MODEL, OTHER_MODEL)
                ]})

        self.server.RequestHandlerClass = Handler


class NavigationJourney(SurfaceJourney):
    def at_home(self):
        footer = self.session.display().rstrip().splitlines()[-1].strip()
        return footer.startswith(tuple(self.surface[key] for key in ("home", "edit", "draft")))

    def direct_home(self, label):
        until(self.session, self.at_home, label + ": direct composer return")
        assert not self.session.screen.cursor.hidden, self.session.display()
        self.capture(label)

    def choose_saved_model(self, expected_metadata):
        session = self.session
        until(session, lambda: len(self.peer.snapshot("GET")) == expected_metadata
              and self.surface["loading"] not in session.display() and MODEL in session.display(),
              "metadata ready")
        session.send(MODEL)
        session.send("\r")
        # Initial selection may change the model: explicitly accept its effort.
        if self.reasoning in session.display():
            session.send("\r")
        self.direct_home("setup-returned-without-extra-escape")

    @property
    def reasoning(self):
        return "Reasoning ·"

    @property
    def files_hint(self):
        return "Enter Review/attach"

    def review_source(self):
        wait_for(self.session, "Review local capture")
        for _ in range(8):
            if "public navigation source" in self.session.display():
                return
            self.session.send(ESC + "[6~")
        raise AssertionError("source bytes not reachable through review paging")

    def select_task(self, current):
        self.session.command("tasks")
        rows = re.findall(r"([ >])([ *]) #(\d+) (?:Open|Complete) /", self.session.display())
        assert len(rows) == 2, self.session.display()
        index = next(index for index, row in enumerate(rows) if (row[1] == "*") == current)
        self.session.send(ESC + "[H" + (ESC + "[B") * index)
        marker = ">* #" if current else ">  #"
        assert marker + rows[index][2] in self.session.display(), self.session.display()
        self.session.send("\r")

    def excerpts(self):
        with closing(sqlite3.connect((self.data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
            return db.execute("SELECT * FROM excerpts ORDER BY id").fetchall()


def run(binary, output, root, size, baseline):
    root.mkdir()
    output.mkdir(parents=True)
    cases, detours = [], []
    with NavigationPeer() as peer:
        j = NavigationJourney(binary, output, root, size, size[0] == 60, "compatible",
                              "chat_completions", True, False, peer, )
        s = j.session
        try:
            wait_for(s, "Storage: saved")
            j.write_request("Other saved task")
            s.command("task-new")
            j.write_request(DRAFT)
            until(s, lambda: len(j.snapshot()["tasks"]) == 2, "two durable tasks")
            s.send((ESC + "[D") * 3)
            cursor = len(DRAFT[:-3].encode())
            s.send("\x13")
            until(s, lambda: saved_request(j, DRAFT, cursor), "saved draft caret")
            s.send("\x0b")
            j.capture("everyday-actions")
            if not baseline:
                view = s.display()
                assert all("/" + action in view for action in ("provider", "model", "files", "tasks")), view
                assert not re.search(r"[|║]\s+[>x ]*/connect\s", view), view
                if "/session-new " in view:
                    assert view.index("/tasks ") < view.index("/session-new "), view
            s.send(ESC)
            for query, command in (("conn", "provider"), ("attachment", "files"),
                                   ("stop", "cancel-all"), ("evidence", "history")):
                s.send("\x0b")
                s.send(query)
                if not baseline:
                    assert any(re.search(r"/" + command + r"(?:\s|$)", row)
                               for row in s.display().splitlines()), s.display()
                j.capture("search-" + command)
                s.send(ESC)
            for query in ("unfindable-navigation-query", "storage-retry"):
                s.send("\x0b")
                s.send(query)
                s.send("\r")
                assert not j.at_home(), "empty/disabled search dispatched"
                s.send(ESC)
            assert saved_request(j, DRAFT, cursor)
            assert not peer.snapshot()
            cases.append("everyday semantic actions preserve draft; empty/disabled search has no effect")

            (j.project / "navigation.txt").write_text("public navigation source\nsecond line\n", encoding="utf-8")
            s.command("refresh")
            s.command("attach")
            wait_for(s, "navigation.txt")
            s.send("\r")
            j.review_source()
            j.capture("source-preview")
            before = j.excerpts()
            s.send("\t")
            j.capture("source-preview-one-tab")
            if j.files_hint not in s.display() and baseline:
                detours.append("source preview required Attachments before Files")
                s.send("\t")
            wait_for(s, j.files_hint)
            assert "navigation.txt" in s.display() and j.excerpts() == before
            s.command("basket")
            assert j.files_hint not in s.display()
            s.send("\t")
            wait_for(s, j.files_hint)
            s.send("\r")
            j.review_source()
            assert j.excerpts() == before
            s.send("\r")
            j.direct_home("explicit-attachment-returned")
            until(s, lambda: len(j.excerpts()) == len(before) + 1, "one approved attachment")
            attached = j.excerpts()
            s.command("assemble")
            wait_for(s, "EXACT BASKET ENVELOPE")
            s.send("\t")
            j.capture("payload-one-tab")
            if j.files_hint not in s.display() and baseline:
                detours.append("payload Tab Files actually opened Attachments")
                s.send("\t")
            wait_for(s, j.files_hint)
            assert j.excerpts() == attached
            s.send(ESC)
            j.direct_home("source-navigation-returned")
            assert saved_request(j, DRAFT, cursor)
            cases.append("preview and payload return directly to Files; only explicit reviewed attachment mutates context")

            j.setup(initial=True)
            s.command("effort high")
            j.direct_home("explicit-effort-returned")
            before = j.snapshot()
            s.command("model")
            wait_for(s, MODEL)
            s.send("\r")
            j.capture("unchanged-model-one-enter")
            if j.reasoning in s.display() and baseline:
                detours.append("unchanged model and effort reopened reasoning")
                s.send("\r")
            j.direct_home("unchanged-model-returned")
            j.assert_owned_unchanged(before)
            assert saved_request(j, DRAFT, cursor)
            assert len(peer.snapshot("GET")) == 1 and not peer.snapshot("POST")
            s.command("model")
            s.send(OTHER_MODEL)
            s.send("\r")
            wait_for(s, j.reasoning)
            s.send(ESC)
            j.direct_home("changed-model-cancelled")
            j.assert_owned_unchanged(before)
            cases.append("unchanged model skips reasoning; changed model still stages reasoning and cancels atomically")

            j.write_request("Explain this navigation fixture without modifying files.")
            s.send("\r")
            until(s, peer.started.is_set, "owned real streaming request")
            j.write_request(DRAFT)
            s.send((ESC + "[D") * 3)
            s.send("\x13")
            until(s, lambda: saved_request(j, DRAFT, cursor), "future draft during active response")
            active = j.snapshot()
            assert active["claims"]
            j.select_task(current=True)
            j.capture("current-task-one-enter")
            if not j.at_home() and baseline:
                detours.append("current task resume hit cross-task active-work guard")
                s.send(ESC)
            j.direct_home("current-task-returned-during-response")
            assert j.snapshot()["claims"] == active["claims"]
            assert saved_request(j, DRAFT, cursor)
            j.select_task(current=False)
            assert not j.at_home(), "different task escaped active-work guard"
            assert j.snapshot()["claims"] == active["claims"]
            s.send(ESC)
            j.direct_home("different-task-refused")
            assert len(peer.snapshot("POST")) == 1
            peer.release_append.set()
            peer.release_finish.set()
            until(s, lambda: not j.snapshot()["claims"], "response settled without replay")
            s.send("!")
            s.send("\x13")
            until(s, lambda: saved_request(j, DRAFT[:-3] + "!" + DRAFT[-3:], cursor + 1),
                  "original Unicode insertion caret preserved")
            cases.append("same-task resume preserves live response and caret; another task remains guarded")
            s.command("quit")
            confirm_exit(s)
            assert s.wait_exit() == 0
            peer.assert_clean()
            result = {"status": "baseline-observed" if baseline else "passed", "size": size,
                      "cases": cases, "detours": detours,
                      "actual_posts": len(peer.snapshot("POST"))}
            (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
            return result
        finally:
            peer.release_append.set()
            peer.release_finish.set()
            s.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", action="store_true")
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="eevee-navigation-") as temporary:
        journeys = [run(binary, output / str(size[0]), Path(temporary) / str(size[0]),
                        size, args.baseline) for size in ((120, 36), (60, 18))]
    result = {"status": "baseline-observed" if args.baseline else "passed",
              "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "journeys": journeys}
    (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({"status": result["status"], "output": str(output)}))


if __name__ == "__main__":
    main()
