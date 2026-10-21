#!/usr/bin/env python3
"""Qualify visible localized hints through a real PTY and read-only SQLite.

Runs 120x36 Unicode and 60x18 ASCII journeys for --language en|ko (default en).
Only an explicitly configured keyless loopback model-metadata GET is authorized;
no inference plan is installed, and every journey requires zero actual POSTs.
Uses an already-built binary and the terminal_smoke.py dependencies. Screen
captures, native terminal bytes, input events and observed SQL state are retained.
This is not live-provider, physical-terminal or model-understanding qualification.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import tempfile
import traceback

from native_connection_smoke import MODEL, Peer, until
from response_smoke import DRAFT, ESC, SurfaceJourney, saved_request
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

HANGUL = re.compile(r"[\uac00-\ud7a3]")
NOTE = "Literal 한글 { $not_a_template } e\u0301 raw-note"
PANELS = {
    "en": {"provider": "Connections", "model": "Model ·", "files": "Sources /",
           "workflows": "Specialist desk /", "memory": "Approved memories /",
           "handoff": "Portable task handoff /"},
    "ko": {"provider": "연결", "model": "모델 ·", "files": "출처 /",
           "workflows": "전문가 데스크 /", "memory": "승인된 메모리 /",
           "handoff": "이동 가능한 작업 인계 /"},
}


class TooltipJourney(SurfaceJourney):
    def read_state(self):
        with closing(sqlite3.connect((self.data / "workbench.sqlite3").as_uri() + "?mode=ro",
                                     uri=True)) as db:
            db.row_factory = sqlite3.Row
            tasks = [dict(row) for row in db.execute("SELECT * FROM tasks ORDER BY id")]
            # Only view/editor-navigation state may change while opening desks.
            # All durable task facts and every Request draft remain compared.
            ignored = {"navigation", "revision", "updated_at", "memory_draft", "workflow_draft"}
            owned = {"tasks": [{key: value for key, value in task.items() if key not in ignored}
                               for task in tasks]}
            for table in ("partners", "active_work", "provider_journal", "executions", "memories",
                          "excerpts", "local_artifacts", "provider_usage", "provider_preferences"):
                owned[table] = [dict(row) for row in db.execute("SELECT * FROM " + table + " ORDER BY rowid")]
        return owned, tasks

    def unchanged(self, label):
        current, _ = self.read_state()
        assert current == self.baseline, label + ": unexpected durable effect: " + repr((self.baseline, current))
        assert not self.peer.snapshot("POST"), label + ": hidden provider inference"
        assert len(self.peer.snapshot("GET")) == 1, label + ": unrequested metadata fetch"
        assert saved_request(self, DRAFT, self.draft_cursor), label + ": literal Request/caret changed"
        self.observations.append({"case": label, "owned_state_unchanged": True,
                                  "actual_posts": 0, "actual_metadata_gets": 1})

    def selected_hint(self, query, command, label):
        session = self.session
        session.send("\x0b")
        session.send(query)
        until(session, lambda: any(re.search(r">\s+/" + re.escape(command) + r"(?:\s|$)", row)
                                  for row in session.display().splitlines()),
              "selected stable /" + command + " for query " + query)
        rows = session.display().splitlines()
        controls = next((index for index, row in enumerate(rows)
                         if "Up/Down" in row and "Enter" in row and "Esc" in row), None)
        assert controls is not None, "command picker critical controls missing\n" + session.display()
        # The selected explanation is immediately above the picker controls,
        # not its localized heading/footer or the separately typed query.
        hint = "\n".join(rows[max(0, controls - 2):controls]).strip(" |│\n")
        assert hint and len(re.findall(r"[A-Za-z]+|[\uac00-\ud7a3]+", hint)) >= 2, hint
        if self.language == "ko":
            assert HANGUL.search(hint), "selected tooltip has no Hangul: " + hint
        else:
            assert not HANGUL.search(hint), "English selected hint unexpectedly localized: " + hint
        self.capture(label)
        self.hints.append({"query": query, "selected_command_id": command, "visible_hint": hint,
                           "screen": label + ".txt"})
        self.unchanged(label)

    def returned(self, label):
        self.workspace()
        session = self.session
        assert not session.screen.cursor.hidden, "Request editing focus was not returned"
        assert (session.screen.cursor.x, session.screen.cursor.y) == self.caret, label + ": caret moved"
        self.unchanged(label)
        self.capture(label)

    def panel(self, command):
        self.selected_hint(command, command, command + "-selected-hint")
        self.session.send("\r")
        wait_for(self.session, PANELS[self.language][command])
        view = self.session.display()
        assert "Esc" in view, command + ": return control missing"
        if self.language == "ko":
            assert HANGUL.search("\n".join(view.splitlines()[-5:])), command + ": localized controls missing"
        self.capture(command + "-opened")
        self.unchanged(command + "-opened")

    def memory_candidate(self, scope):
        _, tasks = self.read_state()
        for task in tasks:
            state = json.loads(task["memory_draft"])
            if isinstance(state, dict) and (state.get("candidate") or {}).get("scope") == scope:
                assert state["candidate"]["content"] == NOTE, state
                assert state["text"] == NOTE, state
                return state
        return None


def run(binary, output, root, size, ascii_mode, language):
    root.mkdir()
    with Peer("chat_completions", "unused-public-keyless-fixture", True) as peer:
        journey = TooltipJourney(binary, output, root, size, ascii_mode, "compatible",
                                 "chat_completions", True, False, peer, language=language)
        session = journey.session
        journey.hints, journey.observations = [], []
        try:
            wait_for(session, "Storage: saved")
            journey.setup(initial=True)
            journey.write_request(DRAFT)
            session.send((ESC + "[D") * 3)
            journey.draft_cursor = len(DRAFT[:-3].encode())
            session.send("\x13")
            until(session, lambda: saved_request(journey, DRAFT, journey.draft_cursor),
                  "literal draft and insertion caret saved")
            journey.caret = (session.screen.cursor.x, session.screen.cursor.y)
            journey.baseline, _ = journey.read_state()
            journey.capture("request-before-tooltips")

            # Korean-description matching is exercised even in the English UI.
            # No command ID appears in this query and Enter must open /help.
            journey.selected_hint("탐색 도움말", "help", "korean-semantic-search-help")
            session.send("\r")
            wait_for(session, journey.surface["help"])
            assert all(control in session.display() for control in ("Ctrl+K", "Enter", "Esc", "PgUp/PgDn"))
            journey.capture("semantic-search-opened-help")
            journey.unchanged("semantic-search-opened-help")
            session.send(ESC)
            journey.returned("help-returned-exact-request-caret")

            for command in ("provider", "model", "files", "workflows", "memory", "handoff"):
                journey.panel(command)
                if command == "provider":
                    # Search edits connection navigation, never the Request.
                    session.send("Compatible")
                    wait_for(session, "Compatible")
                    # The alias is not a display label. Exact native options
                    # below prove the route; here only selection must survive.
                    assert sum(bool(re.search(r">\*\s+", row))
                               for row in session.display().splitlines()) == 1, session.display()
                    journey.capture("provider-original-identity-filter")
                    session.send("\t")
                    selected_action = next((row for row in session.display().splitlines()
                                            if re.search(r"[│║|]\s*>\s+", row)), "")
                    assert selected_action, "provider action selection missing\n" + session.display()
                    if language == "ko":
                        assert HANGUL.search(selected_action), selected_action
                    journey.capture("provider-selected-action-hints")
                    session.send(ESC + "[B")
                    journey.capture("provider-next-action-hints")
                    journey.unchanged("provider-action-navigation-no-effects")
                elif command == "model":
                    wait_for(session, MODEL)
                    session.send(MODEL)
                    wait_for(session, journey.surface["search"] + MODEL)
                    assert any(MODEL in row and ">" in row for row in session.display().splitlines()), session.display()
                    journey.capture("model-original-id-search")
                elif command == "workflows":
                    session.send(ESC + "[C")
                    selected_tab = "[Refactor]" if language == "en" else "[리팩터링]"
                    assert selected_tab in session.display(), session.display()
                    journey.capture("specialist-tab-navigation")
                elif command == "memory":
                    session.command("memory-help")
                    assert "/memory-scope" in session.display(), session.display()
                    journey.capture("memory-scope-help")
                session.send(ESC)
                journey.returned(command + "-returned-exact-request-caret")

            # Inspect exact protocol/model/endpoint options without saving,
            # authorizing metadata, switching provider, or changing the draft.
            session.send("\x13")
            wait_for(session, "Storage: saved")
            session.command("connect compatible")
            wait_for(session, journey.surface["fields"][0] + ":")
            for _ in range(3):
                session.send("\t")
            assert "chat_completions" in session.display() and MODEL in session.display(), session.display()
            assert peer.base in session.display(), session.display()
            journey.capture("connection-literal-protocol-model-endpoint")
            session.send(ESC)
            journey.returned("connection-cancel-returned-exact-request-caret")

            # Scope labels are domain values, not translatable authorization.
            # A literal note is previewed across all three scopes but never
            # approved or selected for outgoing context; SQL proves no memory.
            session.command("memory")
            wait_for(session, PANELS[language]["memory"])
            session.command("create-memory")
            session.paste(NOTE)
            session.send("\r")
            for scope, label in (("task", "Task / only this task"),
                                 ("project", "Project / all tasks in this project"),
                                 ("personal", "Personal / all projects")):
                wait_for(session, label)
                until(session, lambda: journey.memory_candidate(scope) is not None,
                      "literal unapproved memory preview persisted in " + scope + " scope")
                candidate = journey.memory_candidate(scope)
                journey.capture("memory-unapproved-" + scope + "-scope")
                journey.unchanged("memory-unapproved-" + scope + "-scope")
                journey.observations.append({"scope": scope, "literal_scope_label": label,
                                             "candidate": candidate["candidate"], "approved_memories": 0})
                if scope != "personal":
                    session.command("memory-scope")
            session.command("reject-memory")
            session.send(ESC)
            journey.returned("memory-rejected-returned-exact-request-caret")

            # The physical caret check above is paired with literal-byte SQL
            # and an actual insertion, so a hidden cursor offset cannot pass.
            session.send("!")
            expected = DRAFT[:-3] + "!" + DRAFT[-3:]
            expected_cursor = len((DRAFT[:-3] + "!").encode())
            session.send("\x13")
            until(session, lambda: saved_request(journey, expected, expected_cursor),
                  "actual insertion at returned caret preserves literal Unicode bytes")
            journey.capture("actual-insertion-proves-returned-caret")
            assert not peer.snapshot("POST") and len(peer.snapshot("GET")) == 1
            assert not journey.snapshot()["claims"] and journey.snapshot()["executions"] == 0
            wait_for(session, "Storage: saved")
            session.idle()
            session.command("quit")
            confirm_exit(session)
            assert session.wait_exit() == 0
            peer.assert_clean()
            result = {"status": "passed", "language": language, "size": size, "ascii": ascii_mode,
                      "selected_hints": journey.hints, "observations": journey.observations,
                      "actual_posts": 0, "actual_metadata_gets": 1,
                      "baseline_owned_state": journey.baseline,
                      "returned_draft": expected, "returned_cursor_utf8": expected_cursor}
            (output / "state-evidence.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
            return result
        finally:
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--language", choices=("en", "ko"), default="en")
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with binary.open("rb") as handle:
        binary_hash = hashlib.file_digest(handle, "sha256").hexdigest()
    result = {"status": "failed", "binary": str(binary), "sha256": binary_hash,
              "language": args.language, "journeys": [],
              "limits": ["No live account, model inference, physical terminal, GUI or Windows-native qualification."]}
    try:
        with tempfile.TemporaryDirectory(prefix="ev-tooltip-空 格-") as temporary:
            for size, ascii_mode, label in (((120, 36), False, "wide"), ((60, 18), True, "compact")):
                result["journeys"].append(run(binary, output / label, Path(temporary) / label,
                                               size, ascii_mode, args.language))
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
        raise
    finally:
        (output / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
