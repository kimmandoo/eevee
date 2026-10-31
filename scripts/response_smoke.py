#!/usr/bin/env python3
"""Exercise response scrolling through an actual PTY and keyless loopback SSE.

Uses an already-built executable, disposable homes and read-only SQLite queries.
This is local input/render/transport proof, not a GUI-terminal, Windows-native
mouse or live-provider qualification. Dependencies match terminal_smoke.py.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import tempfile
import threading
import traceback
import unicodedata
from unittest.mock import patch

import native_connection_smoke

from native_connection_smoke import Journey, MODEL, Peer, decoded_payload, until
from terminal_smoke import Session, confirm_exit
from workbench_smoke import wait_for

ESC = "\x1b"
UP, DOWN = ESC + "[A", ESC + "[B"
PAGE_UP, PAGE_DOWN = ESC + "[5~", ESC + "[6~"
ALT_HOME, ALT_END = ESC + "[1;3H", ESC + "[1;3F"
ALT_UP = ESC + "[1;3A"
FIRST_REQUEST = "Explain this public scrolling fixture."
DRAFT = "Draft界e\u0301Z-end"
SECOND_ANSWER = "SECOND_RESPONSE_MARKER"


# Only critical visible controls are named here; provider payloads and drafts
# remain literal. These are navigation selectors, not catalog snapshot tests.
SURFACE = {"response": "Response", "previous": "Previous Response",
           "following": "Following", "paused": "Paused", "unread": "+new",
           "complete": "complete", "help": "KEYS",
           "fields": ("API key", "Endpoint", "Default model", "Protocol",
                      "Authentication", "Storage", "Save & choose model"),
           "matches": "1 matches", "loading": "Loading metadata", "search": "Search: ",
           "home": "Type Request |", "edit": "Enter Send |", "draft": "Draft only |"}


class SurfaceSession(Session):
    def __init__(self, argv, *args, **kwargs):
        self.surface = SURFACE
        super().__init__(argv, *args, **kwargs)

    def command(self, name, *, settle=None):
        # Ctrl+K opens/resets the chooser from any drawer. A controlled active
        # turn uses semantic predicates, not idle-only console settling.
        self.send("\x0b", settle=settle)
        self.send(name, settle=settle)
        self.send("\r", settle=settle)

    def paste(self, text, *, settle=None):
        self.send(ESC + "[200~" + text + ESC + "[201~", settle=settle)


class SurfaceJourney(Journey):
    def __init__(self, *args, **kwargs):
        self.surface = SURFACE
        # Constructor adaptation only: every input/output still uses the real
        # Session PTY and all upstream isolation/SQL/HTTP ownership is retained.
        with patch.object(native_connection_smoke, "Session",
                          SurfaceSession):
            super().__init__(*args, **kwargs)

    def workspace(self):
        for _ in range(8):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if 'Alt+A' in footer or footer.startswith(tuple(self.surface[key] for key in ("home", "edit", "draft"))):
                return "Type Request |" if footer.startswith(self.surface["home"]) else "Enter Send |"
            self.session.send(ESC)
        raise AssertionError("Request owner not reachable\n" + self.session.display())

    def setup(self, *, initial=False):
        session = self.session
        before_posts = len(self.peer.snapshot("POST"))
        expected_metadata = len(self.peer.snapshot("GET")) + 1
        session.command("connect compatible")
        fields = self.surface["fields"]
        wait_for(session, fields[0] + ":")
        for index, field in enumerate(fields):
            assert "> " + field + ":" in session.display(), session.display()
            if index < 3:
                assert not session.screen.cursor.hidden
            if index == 1:
                session.send("\x15")
                session.paste(self.peer.base)
            elif index == 2:
                session.send("\x15")
                session.paste(MODEL)
            elif index == 4:
                assert initial and self.keyless
                session.send(" ")
            elif index == 5 and not self.persist:
                session.send(" ")  # Keep this fixture explicitly session-only.
            elif index == 6:
                self.capture("setup-explicit-save-and-choose-model")
                session.send("\r")
                break
            session.send("\t")
        self.choose_saved_model(expected_metadata)
        assert len(self.peer.snapshot("POST")) == before_posts, "setup performed inference"

    def choose_saved_model(self, expected_metadata):
        session = self.session
        wait_for(session, MODEL)
        until(session, lambda: len(self.peer.snapshot("GET")) == expected_metadata,
              "exactly one authorized native metadata request")
        wait_for(session, self.surface["matches"])
        until(session, lambda: self.surface["loading"] not in session.display(),
              "metadata catalogue ready")
        session.send(MODEL)
        wait_for(session, self.surface["search"] + MODEL)
        self.capture("model-metadata-search")
        session.send("\r")
        if 'Reasoning · ' in session.display():
            session.send('\r')
        self.workspace()
        assert len(self.peer.snapshot("GET")) == expected_metadata, "unrequested metadata fetch/retry"

def paragraphs(start, end):
    return "".join(" ".join(f"R{row:03}w{word:02}-界" for word in range(20)) + "\n"
                   for row in range(start, end))


class StreamingPeer(Peer):
    def __init__(self):
        super().__init__("chat_completions", "unused-public-keyless-fixture", True)
        self.prefix, self.suffix = paragraphs(0, 80), paragraphs(80, 100)
        self.release_append, self.release_finish = threading.Event(), threading.Event()
        self.started, self.appended = threading.Event(), threading.Event()
        self.stream_used = False
        fixture = self
        inherited = self.server.RequestHandlerClass

        class Handler(inherited):
            def do_POST(self):
                if fixture.stream_used:
                    return super().do_POST()
                fixture.stream_used = True
                size = int(self.headers.get("Content-Length", "0"))
                assert 0 < size <= 1024 * 1024, "unexpected request size"
                fixture.recorded_stream = self.record(json.loads(self.rfile.read(size)))
                assert self.path == "/v1/chat/completions", self.path
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Connection", "close")
                self.end_headers()
                self.close_connection = True

                def event(content=None, finished=False):
                    value = {"model": MODEL, "choices": [{"index": 0,
                             "delta": {} if content is None else {"content": content},
                             "finish_reason": "stop" if finished else None}]}
                    self.wfile.write(("data: " + json.dumps(value) + "\n\n").encode())
                    self.wfile.flush()

                def gate(signal):
                    for _ in range(1200):
                        if fixture.stopped.is_set():
                            return False
                        if signal.wait(.05):
                            return True
                    raise AssertionError("owned stream gate exceeded its deadline")

                event(fixture.prefix)
                fixture.started.set()
                if not gate(fixture.release_append):
                    return
                event(fixture.suffix)
                fixture.appended.set()
                if not gate(fixture.release_finish):
                    return
                event(finished=True)
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()

        self.server.RequestHandlerClass = Handler


def response_view(session):
    rows = session.display().splitlines()
    surface = session.surface
    header_pattern = (
        rf"(?:{re.escape(surface['previous'])}|{re.escape(surface['response'])}"
        rf"|(?:Previous )?Request outcome|(?:Previous )?Plan|(?:Previous )?Tool output)(?: \d+/\d+)? · "
    )
    header = next((index for index, row in enumerate(rows)
                   if re.search(header_pattern, row)), None)
    assert header is not None, "response header missing\n" + session.display()
    status_pattern = (rf"(?:{re.escape(surface['following'])}|{re.escape(surface['paused'])})"
                      rf"(?: {re.escape(surface['unread'])})? (?P<start>\d+)-(?P<end>\d+)/(?P<total>\d+)")
    status = next((index for index in range(header + 1, len(rows))
                   if re.search(status_pattern, rows[index])), None)
    assert status is not None and status > header + 1, "response body not visible\n" + session.display()
    # Outcomes prefix metadata independently of the observation kind. The
    # navigation footer starts at the actual body edge; a label inside the
    # metadata line does not (and would clip source/request/reason prefixes).
    position = re.search(status_pattern, rows[status])
    column = position.start()
    first, last, total = (int(position[name]) for name in ('start', 'end', 'total'))
    # The status range counts actual transcript rows. The mutable notice
    # between body and navigation is chrome, not retained request evidence.
    visible = max(1, last - first + 1)
    assert header + visible < status, 'transcript range overlaps navigation chrome'
    screen = session.screen
    right = screen.columns - column
    rail = next((y for y in range(header) if 'PARTNER' in rows[y]), None)
    if rail is not None:
        # One gutter and one card inset separate the transcript from its heading.
        right = min(right, rows[rail].index('PARTNER') - 2)

    def cells(y):
        from wcwidth import wcswidth
        value, x = [], column
        while x < right:
            data = screen.buffer[y][x].data
            value.append(data or ' ')
            x += max(1, wcswidth(data)) if data else 1
        return unicodedata.normalize('NFC', ''.join(value))

    return {"body": [cells(y) for y in range(header + 1, header + 1 + visible)],
            "point": (column + 1, header + 1), "status": cells(status),
            "range": (first, last, total)}


def wheel(session, direction, point, count=1):
    column, row = point
    code = 64 if direction == "up" else 65
    session.send((f"{ESC}[<{code};{column + 1};{row + 1}M") * count)


def saved_request(journey, text, cursor):
    with closing(sqlite3.connect((journey.data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
        rows = db.execute("SELECT partner_drafts FROM tasks").fetchall()
    return any(draft["field"] == "Request" and draft["text"] == text and draft["cursor"] == cursor
               for (encoded,) in rows for owner in json.loads(encoded).values()
               for draft in owner.get("drafts", []))


def run(binary, output, root, size, ascii_mode, ):
    cases = []
    root.mkdir()
    with StreamingPeer() as peer:
        journey = SurfaceJourney(binary, output, root, size, ascii_mode, "compatible",
                                 "chat_completions", True, False, peer, )
        session = journey.session
        surface = journey.surface
        try:
            wait_for(session, "Storage: saved")
            journey.setup(initial=True)
            journey.write_request(FIRST_REQUEST)
            session.send("\r")
            until(session, peer.started.is_set, "first real streaming HTTP request")
            wait_for(session, "R079w19")
            view = response_view(session)
            assert surface["following"] in view["status"], view
            assert len(peer.snapshot("POST")) == 1
            assert all(mode not in session.text for mode in (ESC + "[?1002h", ESC + "[?1003h"))
            assert ESC + "[?1000h" in session.text and ESC + "[?1006h" in session.text
            session.paste(DRAFT)
            session.send(ESC + "[D" + ESC + "[D" + ESC + "[D")
            caret = (session.screen.cursor.x, session.screen.cursor.y)
            wheel(session, "up", view["point"], 8)
            paused = response_view(session)
            assert surface["paused"] in paused["status"] and paused["body"] != view["body"]
            assert not session.screen.cursor.hidden
            assert (session.screen.cursor.x, session.screen.cursor.y) == caret
            session.capture("wheel-paused-draft-caret-kept")
            cases.append("wheel scroll paused the transcript without moving draft caret or focus")

            wheel(session, "up", (0, 0), 3)
            assert response_view(session)["body"] == paused["body"]
            session.command("help")
            wait_for(session, surface["help"])
            wheel(session, "up", paused["point"], 3)
            assert surface["help"] in session.display() and len(peer.snapshot("POST")) == 1
            session.send(ESC)
            assert response_view(session)["body"] == paused["body"]
            assert (session.screen.cursor.x, session.screen.cursor.y) == caret
            cases.append("outside and overlaid wheel gestures neither scrolled hidden output nor dispatched work")

            session.send(PAGE_UP)
            page = response_view(session)
            assert page["body"] != paused["body"] and surface["paused"] in page["status"]
            session.send(PAGE_DOWN)
            assert response_view(session)["body"] == paused["body"]
            cases.append("viewport-sized page up/down returned to the same reading location while editing")

            anchor = re.search(r"R\d{3}w\d{2}", "\n".join(paused["body"])).group()
            other_size = (60, 18) if size == (120, 36) else (120, 36)
            session.resize(other_size)
            resized = response_view(session)
            assert anchor in "\n".join(resized["body"]), (anchor, resized)
            session.capture("paused-unicode-reflow")
            session.resize(size)
            assert response_view(session)["body"] == paused["body"]
            cases.append("paused Unicode content stayed anchored through narrow/wide resize and return")

            peer.release_append.set()
            until(session, lambda: surface["unread"] in response_view(session)["status"], "new output while paused")
            assert peer.appended.is_set()
            assert response_view(session)["body"] == paused["body"]
            session.capture("stream-appended-without-reading-jump")
            cases.append("actual later SSE chunks showed unread output without moving paused text")

            session.command("help")
            wait_for(session, surface["help"])
            session.command("response")
            fronted = response_view(session)
            assert not session.screen.cursor.hidden
            assert (session.screen.cursor.x, session.screen.cursor.y) == caret
            wheel(session, "up", fronted["point"], 2)
            assert response_view(session)["body"] != fronted["body"]
            session.capture("response-command-fronted-output")
            session.send(ALT_END)
            wait_for(session, "R099w19")
            assert surface["following"] in response_view(session)["status"]
            cases.append("response command fronted inline output without an extra drawer or Escape; Alt+End resumed latest")

            peer.release_finish.set()
            until(session, lambda: not journey.snapshot()["claims"] and surface["complete"] in session.display(),
                  "completed stream and ownership settlement")
            session.send("!")
            expected_draft = DRAFT[:-3] + "!" + DRAFT[-3:]
            expected_cursor = len((DRAFT[:-3] + "!").encode())
            session.send("\x13")
            until(session, lambda: saved_request(journey, expected_draft, expected_cursor),
                  "literal Unicode draft and original insertion caret committed")
            cases.append("post-scroll insertion committed exact Unicode draft bytes at the original caret")

            wheel(session, "up", response_view(session)["point"], 8)
            old = response_view(session)
            peer.plans.put(("complete", SECOND_ANSWER))
            session.send("\r")
            until(session, lambda: len(peer.snapshot("POST")) == 2 and not journey.snapshot()["claims"],
                  "one deliberately submitted follow-up")
            previous = response_view(session)
            assert surface["previous"] in session.display() and surface["paused"] in previous["status"]
            session.capture("new-turn-kept-previous-reading-location")
            assert re.findall(r"R\d{3}w\d{2}", previous["body"][0]) == re.findall(
                r"R\d{3}w\d{2}", old["body"][0]
            ), (old["body"][0], previous["body"][0])
            _, messages = decoded_payload("chat_completions", peer.snapshot("POST")[-1]["body"])
            assert messages == [("user", FIRST_REQUEST), ("assistant", peer.prefix + peer.suffix),
                                ("user", expected_draft)], messages
            session.send(ALT_END)
            wait_for(session, SECOND_ANSWER)
            boundary_rows = list(response_view(session)["body"])
            # No response-selection key: ordinary pages must cross both ways
            # through the segment boundary, including its attribution rows.
            for _ in range(32):
                session.send(PAGE_UP)
                crossed = response_view(session)
                boundary_rows.extend(crossed["body"])
                if re.search(r"R\d{3}w", "\n".join(crossed["body"])) and SECOND_ANSWER not in "\n".join(crossed["body"]):
                    break
            else:
                raise AssertionError("page movement could not reach the preceding response\n" + session.display())
            assert surface["paused"] in crossed["status"]
            assert SECOND_ANSWER not in "\n".join(crossed["body"])
            boundary_anchor = re.search(r"R\d{3}w\d{2}", "\n".join(crossed["body"])).group()
            session.resize(other_size)
            assert boundary_anchor in "\n".join(response_view(session)["body"])
            session.resize(size)
            assert response_view(session)["body"] == crossed["body"]
            session.capture("continuous-previous-segment-resize-anchor")
            for _ in range(32):
                session.send(PAGE_DOWN)
                boundary_rows.extend(response_view(session)["body"])
                if SECOND_ANSWER in "\n".join(response_view(session)["body"]):
                    break
            else:
                raise AssertionError("page movement could not return across the response boundary\n" + session.display())
            assert surface["paused"] in response_view(session)["status"], "manual bottom resumed follow"
            # Wheel navigation traverses the same boundary, not a separate item page.
            for _ in range(32):
                wheel(session, "up", response_view(session)["point"])
                if re.search(r"R\d{3}w", "\n".join(response_view(session)["body"])) and SECOND_ANSWER not in "\n".join(response_view(session)["body"]):
                    break
            else:
                raise AssertionError("wheel could not cross the response boundary\n" + session.display())
            for _ in range(32):
                wheel(session, "down", response_view(session)["point"])
                if SECOND_ANSWER in "\n".join(response_view(session)["body"]):
                    break
            else:
                raise AssertionError("wheel could not return to the newer response\n" + session.display())
            session.capture("continuous-boundary-pages-and-wheel")
            assert unicodedata.normalize("NFC", expected_draft) in unicodedata.normalize("NFC", "\n".join(boundary_rows)), "segment request attribution was not traversable"
            session.send(ESC)
            for _ in range(64):
                session.send(UP)
                line_view = response_view(session)
                if re.search(r"R\d{3}w", "\n".join(line_view["body"])) and SECOND_ANSWER not in "\n".join(line_view["body"]):
                    break
            else:
                raise AssertionError("line navigation could not cross the response boundary\n" + session.display())
            for _ in range(64):
                session.send(DOWN)
                if SECOND_ANSWER in "\n".join(response_view(session)["body"]):
                    break
            else:
                raise AssertionError("line navigation could not return across the response boundary\n" + session.display())
            assert len(peer.snapshot("POST")) == 2 and journey.snapshot()["executions"] == 0
            cases.append("pages and wheel crossed response boundaries both ways; paused resize retained the anchor without dispatch")
            session.send(ALT_UP)
            session.send(ALT_HOME)
            top = response_view(session)["body"]
            session.send(PAGE_UP)
            assert response_view(session)["body"] == top, "transcript Home did not clamp at the first row"
            cases.append("new turn preserved the old reading location; explicit latest/inspect/top keys remained local")

            session.send(ESC)
            at_top = response_view(session)
            session.send(DOWN)
            after_down = response_view(session)
            assert after_down["body"] != at_top["body"]
            session.send(UP)
            assert response_view(session)["body"] == at_top["body"]
            assert len(peer.snapshot("POST")) == 2 and journey.snapshot()["executions"] == 0
            session.capture("reading-arrows-do-not-switch-replies")
            session.idle()
            cases.append("reading arrows moved one row, not replies; no local commands, hidden sends or idle redraws")

            session.command("quit")
            confirm_exit(session)
            assert session.wait_exit() == 0
            for mode in (1000, 1006):
                assert session.text.rfind(f"{ESC}[?{mode}l") > session.text.rfind(f"{ESC}[?{mode}h")
            peer.assert_clean()
            cases.append("clean exit restored mouse, raw, alternate-screen, paste and cursor modes")
            return {"size": size, "ascii": ascii_mode, "status": "passed",
                    "cases": cases, "actual_posts": len(peer.snapshot("POST"))}
        finally:
            peer.release_append.set()
            peer.release_finish.set()
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with binary.open("rb") as handle:
        binary_hash = hashlib.file_digest(handle, "sha256").hexdigest()
    result = {"status": "failed", "binary": str(binary), "sha256": binary_hash, "journeys": [], "limits": ["No live account, physical trackpad, GUI terminal or Windows-native qualification."]}
    try:
        with tempfile.TemporaryDirectory(prefix="ev-response-空 格-") as temporary:
            for size, ascii_mode, label in [((120, 36), False, "wide"), ((60, 18), True, "compact")]:
                result["journeys"].append(run(binary, output / label, Path(temporary) / label,
                                             size, ascii_mode, ))
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
        raise
    finally:
        (output / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(result, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
