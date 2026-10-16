#!/usr/bin/env python3
"""Native installed-TUI acceptance for durable local workbench behavior."""
import argparse
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sqlite3
import socket
import subprocess
import tempfile
import threading
import time
from contextlib import closing

from terminal_smoke import Session, confirm_exit, terminal_environment


def lead_drafts(task):
    return task["partner_drafts"].get(str(task["lead_partner"]), {}).get("drafts", [])


def wait_for(session, text, timeout=8):
    deadline = time.monotonic() + timeout
    while text not in session.display():
        assert session.alive(), "application exited while waiting for " + text
        assert time.monotonic() < deadline, "missing state: " + text + "\n" + session.display()
        session.pump(0.05)


class NativeFixture:
    """Deterministic loopback HTTP/SSE service, not live-provider conformance."""

    responses = ("Fixture alpha: proposal only.", "Fixture beta: proposal only.")
    private_reasoning = "NATIVE_FIXTURE_PRIVATE_REASONING"

    def __init__(self):
        self.lock = threading.Lock()
        self.requests = []
        self.errors = []
        self.workers = []
        self.fail_models = threading.Event()
        self.delayed_started = threading.Event()
        self.delayed_closed = threading.Event()
        self.stopped = threading.Event()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_args):
                pass

            def handle(self):
                with fixture.lock:
                    fixture.workers.append(threading.current_thread())
                self.connection.settimeout(2)
                try:
                    super().handle()
                except (OSError, ValueError) as error:
                    if not fixture.stopped.is_set():
                        with fixture.lock:
                            fixture.errors.append(str(error))

            def record(self, body=None):
                with fixture.lock:
                    fixture.requests.append({
                        "method": self.command, "path": self.path,
                        "headers": dict(self.headers), "body": body,
                    })
                    return sum(row["method"] == "POST" for row in fixture.requests)

            def reply(self, status, body):
                encoded = json.dumps(body).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(encoded)
                self.close_connection = True

            def do_GET(self):
                self.record()
                if self.path != "/v1/models":
                    self.reply(404, {"error": "unexpected fixture route"})
                elif fixture.fail_models.is_set():
                    self.reply(503, {"error": "deterministic metadata failure"})
                else:
                    self.reply(200, {"data": [
                        {"id": "fixture-alpha", "name": "fixture-alpha"},
                        {"id": "fixture-beta-mini", "name": "fixture-beta-mini"},
                        {"id": "fixture-beta", "name": "fixture-beta",
                         "supported_reasoning_levels": ["low", "high"]},
                    ]})

            def do_POST(self):
                length = int(self.headers.get("Content-Length", "0"))
                if self.path != "/v1/chat/completions" or not 0 < length <= 1024 * 1024:
                    self.record()
                    self.reply(400, {"error": "unexpected fixture request"})
                    return
                ordinal = self.record(json.loads(self.rfile.read(length)))
                self.send_response(200)
                # Exercise valid semantic SSE even when a gateway omits Content-Type.
                self.send_header("Connection", "close")
                self.end_headers()
                self.close_connection = True
                if ordinal == 3:
                    # No answer bytes: cancellation must drop the streaming
                    # client, not merely hide output or wait for server EOF.
                    self.wfile.write(b": fixture awaiting local cancellation\n\n")
                    self.wfile.flush()
                    fixture.delayed_started.set()
                    self.connection.settimeout(0.1)
                    deadline = time.monotonic() + 10
                    while not fixture.stopped.is_set() and time.monotonic() < deadline:
                        try:
                            if self.connection.recv(1) == b"":
                                fixture.delayed_closed.set()
                                return
                        except socket.timeout:
                            continue
                        except (ConnectionResetError, ConnectionAbortedError):
                            fixture.delayed_closed.set()
                            return
                    if not fixture.stopped.is_set():
                        with fixture.lock:
                            fixture.errors.append("cancelled native client did not close")
                    return
                if ordinal not in (1, 2):
                    with fixture.lock:
                        fixture.errors.append("unexpected extra inference request")
                    return
                frame = {
                    "id": "fixture-response-" + str(ordinal), "model": "fixture-observed",
                    "choices": [{"index": 0, "delta": {
                        "content": fixture.responses[ordinal - 1],
                        "reasoning_content": fixture.private_reasoning,
                    }, "finish_reason": "stop"}],
                }
                self.wfile.write(("data: " + json.dumps(frame) + "\n\ndata: [DONE]\n\n").encode("utf-8"))
                self.wfile.flush()

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.server.block_on_close = False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={"poll_interval": 0.05}, daemon=True)

    def snapshot(self, method=None):
        with self.lock:
            assert not self.errors, self.errors
            return [row.copy() for row in self.requests if method is None or row["method"] == method]

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_error):
        self.stopped.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        with self.lock:
            workers = self.workers[:]
        deadline = time.monotonic() + 3
        for worker in workers:
            worker.join(timeout=max(0, deadline - time.monotonic()))
        assert not self.thread.is_alive() and not any(worker.is_alive() for worker in workers), \
            "native fixture server leaked a thread"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/ev")
    parser.add_argument("--output", default="target/workbench-smoke")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == "nt" and not binary.exists(): binary = binary.with_suffix(".exe")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {"status": "failed", "cases": []}
    sessions = []
    def passed(name):
        result["cases"].append(name)
        print("passed:", name, flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="workbench-空 格-", dir=output) as temporary:
            root = Path(temporary).resolve()
            project = root / "project 漢字"
            project.mkdir()
            data = root / "data 空 格"
            def launch(name, directory=data, cwd=project, size=(120,36), extra=(), environment=None):
                session = Session([str(binary), "--data-dir", str(directory), "--images", "off", "--color", "mono", *extra],
                                  cwd, terminal_environment("mono") if environment is None else environment, size, output / name)
                sessions.append(session)
                session.pump(0.5)
                return session
            def quit_app(session):
                session.command("quit")
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()
            export_counter = 0
            def inspect_store(directory):
                nonlocal export_counter
                export_counter += 1
                destination = root / ("inspect-" + str(export_counter) + ".json")
                command = [str(binary), "--data-dir", str(directory), "--export-data", str(destination)]
                process = subprocess.run(command, capture_output=True, text=True, timeout=10)
                assert process.returncode == 0, process.stderr
                return json.loads(destination.read_text())
            def stored_when(directory, predicate):
                deadline = time.monotonic() + 8
                while True:
                    rows = inspect_store(directory)
                    if predicate(rows):
                        return rows
                    assert time.monotonic() < deadline, "stored state did not settle"
                    time.sleep(0.05)
            # Local-only fixtures explicitly choose task facts instead of submitting a request.
            empty_path = root / "no-provider-programs"
            empty_path.mkdir()
            isolated_home = root / "first-use-home"
            isolated_home.mkdir()
            first_use_environment = terminal_environment("mono")
            for key in list(first_use_environment):
                if key.startswith(("OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GEMINI_", "GOOGLE_", "AWS_", "OPENCODE_")):
                    first_use_environment.pop(key, None)
            first_use_environment.update(
                PATH=str(empty_path), HOME=str(isolated_home), USERPROFILE=str(isolated_home),
                XDG_CONFIG_HOME=str(isolated_home / "config"), XDG_DATA_HOME=str(isolated_home / "data"),
                XDG_CACHE_HOME=str(isolated_home / "cache"), APPDATA=str(isolated_home / "config"),
                LOCALAPPDATA=str(isolated_home / "cache"),
            )
            for width in (120, 60):
                first_use_data = root / ("first-use-" + str(width))
                assets = root / ("first-use-artwork-" + str(width))
                size = (width, 36 if width == 120 else 18)
                first_use_args = dict(directory=first_use_data, size=size,
                                      extra=("--assets-dir", str(assets), "--images", "cells"), environment=first_use_environment)
                fresh = launch("first-use-" + str(width), **first_use_args)
                wait_for(fresh, "Storage: saved")
                fresh.command("goal")
                assert not fresh.screen.cursor.hidden, "first-run task input has no visible caret"
                fresh.capture("visible-first-run-input")
                typed = "qgo? direct typing "
                pasted = "漢字\nlocal-only pasted goal"
                fresh.send(typed)
                fresh.paste(pasted)
                assert "qgo? direct typing" in fresh.display()
                assert "local-only pasted goal" in fresh.display()
                assert fresh.alive(), "typing a navigation letter executed a shortcut"
                fresh.capture("direct-typing-and-paste")
                fresh.send("\r")
                saved_goal = typed + pasted
                stored_when(first_use_data, lambda rows: len(rows) == 1 and rows[0]["goal"] == saved_goal)
                wait_for(fresh, "Storage: saved")
                quit_app(fresh)
                fresh = launch("first-use-restart-" + str(width), **first_use_args)
                fresh.command("goal")
                wait_for(fresh, "qgo? direct typing")
                assert inspect_store(first_use_data)[0]["goal"] == saved_goal
                fresh.capture("local-apply-restart")
                passed("first-run-visible-input-direct-type-paste-local-apply-restart-" + str(width))

                fresh.command("goal")
                fresh.paste(" retained shortcut draft")
                fresh.command("connect")
                wait_for(fresh, "Connections")
                wait_for(fresh, "Not installed")
                fresh.capture("command-provider-list")
                fresh.send("\r")  # Provider -> visible actions.
                wait_for(fresh, "Use native responses")
                fresh.capture("command-provider-actions")
                fresh.send("\r")  # Missing CLI: choose the native connection locally.
                wait_for(fresh, "Codex")
                assert "Review connection" not in fresh.display()
                assert "Connections" not in fresh.display(), "method selection left an unnecessary provider drawer"
                fresh.command("provider")
                wait_for(fresh, "PROVIDERS")
                fresh.send("\r")
                wait_for(fresh, "Use CLI agent")
                fresh.capture("unified-codex-native-method")
                fresh.send("\x1b")
                wait_for(fresh, "PROVIDERS")
                fresh.send("\x1b")
                wait_for(fresh, "Storage: saved")
                fresh.command("goal")
                assert "retained shortcut draft" in fresh.display(), "/connect discarded the task draft"
                fresh.send("\x13")
                wait_for(fresh, "Storage: saved")
                retained = inspect_store(first_use_data)[0]
                assert retained["goal"] == saved_goal
                assert lead_drafts(retained)[0]["text"] == saved_goal + " retained shortcut draft"
                passed("command-discoverable-provider-actions-back-preserves-draft-without-auth-" + str(width))
                for command, label in (("help", "command-help"),
                                       (None, "ctrl-k-actions")):
                    fresh.command("goal")
                    editor_view = fresh.display()
                    fresh.command(command) if command else fresh.send("\x0b")
                    assert fresh.display() != editor_view, label + " did not open"
                    fresh.capture(label)
                    fresh.send("\x1b")
                    fresh.command("goal")
                    assert "retained shortcut draft" in fresh.display(), label + " discarded the draft"
                    fresh.send("\x13")
                    wait_for(fresh, "Storage: saved")
                    assert lead_drafts(inspect_store(first_use_data)[0]) == lead_drafts(retained)
                passed("global-help-and-actions-preserve-inline-draft-" + str(width))

                fresh.command("forms")
                wait_for(fresh, "Choose evolution")
                fresh.send("\x1b[H" + "\x1b[C" * 8)
                wait_for(fresh, "Sylveon")
                fresh.capture("command-evolution-preview")
                fresh.send("\x1b")
                assert inspect_store(first_use_data)[0]["form"] == 0, "preview/Back applied an evolution"
                fresh.command("forms")
                fresh.command("role-sylveon")
                stored_when(first_use_data, lambda rows: rows[0]["form"] == 8)
                assert lead_drafts(inspect_store(first_use_data)[0]) == lead_drafts(retained)
                passed("command-evolution-preview-back-and-explicit-select-" + str(width))
                fresh.command("goal")
                fresh.command("images")  # Retain the draft and open only a review.
                wait_for(fresh, "Images / Eevee and evolutions")
                wait_for(fresh, "0/9")
                wait_for(fresh, "> Back")
                wait_for(fresh, "Download missing / Retry")
                wait_for(fresh, "Reload local")
                fresh.capture("command-image-review-neutral")
                fresh.send("\x1b[F")
                wait_for(fresh, "Enter Select")
                fresh.send("\r")  # Even a fully read review defaults to Back.
                wait_for(fresh, "Storage: saved")
                assert not assets.exists(), "neutral image review downloaded artwork"
                fresh.command("images")
                wait_for(fresh, "> Back")
                fresh.send("\x1b[F\t")
                wait_for(fresh, "> Download missing / Retry")
                fresh.capture("command-download-selected-not-approved")
                fresh.send("\x1b")  # Highlighting Download is not consent.
                wait_for(fresh, "Storage: saved")
                assert "Downloading pinned artwork" not in fresh.text, "neutral image navigation launched a download"
                assert not assets.exists(), "Esc from image review downloaded artwork"
                fresh.command("goal")
                assert "retained shortcut draft" in fresh.display(), "/images discarded the task draft"
                fresh.send("\x13")
                wait_for(fresh, "Storage: saved")
                assert lead_drafts(inspect_store(first_use_data)[0]) == lead_drafts(retained)
                passed("command-image-review-neutral-enter-and-escape-no-download-" + str(width))

                old_task = inspect_store(first_use_data)[0]
                fresh.command("reset")
                wait_for(fresh, "Reset local task")
                fresh.send("n")
                fresh.command("goal")
                assert "retained shortcut draft" in fresh.display(), "/reset cancellation lost the editor"
                fresh.command("reset")
                wait_for(fresh, "Reset local task")
                fresh.capture("reset-neutral")
                fresh.send("y")
                wait_for(fresh, "Ask a question or describe")
                assert not fresh.screen.cursor.hidden, "/reset did not return to an editable request"
                preserved = inspect_store(first_use_data)[0]
                assert preserved["id"] == old_task["id"]
                assert preserved["goal"] == old_task["goal"]
                assert lead_drafts(preserved) == lead_drafts(old_task), "/reset discarded saved drafts"
                fresh.command("init")
                wait_for(fresh, "Espeon")
                assert "LOCAL PREVIEW" not in fresh.display(), "/init submitted without Enter"
                fresh.send("\x13")
                rows = stored_when(first_use_data, lambda rows: len(rows) == 2)
                initialized = next(task for task in rows if task["id"] != old_task["id"])
                initial_draft = lead_drafts(initialized)
                assert any("AGENTS.md" in draft["text"] for draft in initial_draft)
                fresh.command("init")
                wait_for(fresh, "Prepare /init request")
                fresh.send("n")
                fresh.send("\x13")
                wait_for(fresh, "Storage: saved")
                restored = next(task for task in inspect_store(first_use_data) if task["id"] == initialized["id"])
                assert lead_drafts(restored) == initial_draft, "/init cancellation replaced the draft"
                assert not (project / "AGENTS.md").exists(), "/init wrote a guide without approval"
                fresh.capture("init-editable-no-dispatch")
                passed("reset-retains-old-task-and-init-prepares-only-" + str(width))
                with closing(sqlite3.connect((first_use_data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                    assert db.execute("SELECT count(*) FROM provider_journal").fetchone()[0] == 0
                    assert db.execute("SELECT count(*) FROM executions").fetchone()[0] == 0
                assert not assets.exists(), "first-use navigation created an unapproved artwork cache"
                assert not any(path.is_file() for path in isolated_home.rglob("*")), "navigation wrote provider-owned configuration or credentials"
                quit_app(fresh)

            # These journeys drive the installed TUI and real native HTTP client.
            # The service is deterministic and local; no external account or
            # live-provider protocol/billing conformance is exercised or claimed.
            for width in (120, 60):
                native_data = root / ("native-data-" + str(width))
                native_project = root / ("native-project-" + str(width))
                native_project.mkdir()
                native_home = root / ("native-home-" + str(width))
                native_home.mkdir()
                first_request = "native first " + str(width)
                second_request = "native second " + str(width)
                third_request = "native delayed " + str(width)
                unsent_request = "native unsent followup " + str(width)
                custom_model = "fixture/custom:Exact-v1"

                def saved_request(rows):
                    return next((draft["text"] for draft in lead_drafts(rows[0])
                                 if draft["field"] == "Request"), "") if rows else ""

                with NativeFixture() as fixture:
                    native_environment = first_use_environment.copy()
                    for key in list(native_environment):
                        if key.startswith("EV_") or key.lower().endswith("_proxy"):
                            native_environment.pop(key, None)
                    native_environment.update(
                        HOME=str(native_home), USERPROFILE=str(native_home),
                        XDG_CONFIG_HOME=str(native_home / "config"),
                        XDG_DATA_HOME=str(native_home / "data"),
                        XDG_CACHE_HOME=str(native_home / "cache"),
                        APPDATA=str(native_home / "config"), LOCALAPPDATA=str(native_home / "cache"),
                        EV_PROVIDER="compatible",
                        EV_API_BASE_URL="http://127.0.0.1:" + str(fixture.server.server_port) + "/v1",
                        EV_API_PROTOCOL="chat_completions", EV_API_MODEL="custom", EV_API_KEYLESS="1",
                        NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost",
                    )
                    native = launch("native-" + str(width), directory=native_data,
                                    cwd=native_project, size=(width, 36 if width == 120 else 18),
                                    environment=native_environment)
                    wait_for(native, "Storage: saved")
                    native.paste(first_request)
                    native.send("\x13")
                    stored_when(native_data, lambda rows: saved_request(rows) == first_request)
                    native.command("model")
                    wait_for(native, "fixture-beta")
                    assert len(fixture.snapshot("GET")) == 1
                    assert not fixture.snapshot("POST"), "model discovery performed inference"
                    native.send("fixture-beta")
                    wait_for(native, "Search: fixture-beta")
                    wait_for(native, "fixture-beta")
                    assert "fixture-alpha" not in native.display(), "search retained a nonmatching model row"
                    native.capture("native-metadata-search")
                    native.send("\r")
                    wait_for(native, "Reasoning")
                    native.capture("native-model-then-reasoning")
                    native.send("\x1b[B\r")  # Default -> low; commit model and effort together.
                    wait_for(native, first_request)
                    deadline = time.monotonic() + 8
                    while True:
                        with closing(sqlite3.connect((native_data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                            preference = json.loads(db.execute("SELECT record FROM provider_preferences WHERE id=1").fetchone()[0])
                        chosen = preference["selections"][preference["provider"]]
                        if (chosen["model"], chosen["effort"]) == ("fixture-beta", "low"):
                            break
                        assert time.monotonic() < deadline, preference
                        native.pump(0.05)
                    assert not fixture.snapshot("POST"), "reasoning selection sent the retained draft"
                    passed("native-model-then-reasoning-commits-without-sending-" + str(width))
                    native.command("model")
                    wait_for(native, "Current: fixture-beta / low")
                    native.send("\x1b[F\x1b[A\r")  # End: Refresh; previous row: Custom ID.
                    wait_for(native, "CUSTOM MODEL ID")
                    native.paste(custom_model)
                    native.send("\r")
                    wait_for(native, first_request)
                    native.command("model")
                    wait_for(native, "Current: " + custom_model)
                    native.capture("native-exact-custom-model")
                    native.send("\x1b")
                    assert saved_request(inspect_store(native_data)) == first_request
                    assert not fixture.snapshot("POST"), "local model selection submitted a draft"
                    passed("native-fixture-metadata-search-exact-custom-id-retains-draft-" + str(width))

                    quit_app(native)  # Immediate exit waits for the preference commit.
                    restored_environment = native_environment.copy()
                    restored_environment.pop("EV_PROVIDER", None)
                    restored_environment.update(
                        OPENAI_API_KEY="fixture-competing-provider-not-a-real-key",
                        OPENAI_BASE_URL="http://127.0.0.1:" + str(fixture.server.server_port) + "/v1",
                    )
                    native = launch("native-restored-" + str(width), directory=native_data,
                                    cwd=native_project, size=(width, 36 if width == 120 else 18),
                                    environment=restored_environment)
                    wait_for(native, "Storage: saved")
                    assert len(fixture.snapshot("GET")) == 1, "restart queried models without user intent"
                    assert not fixture.snapshot("POST"), "restart submitted an old draft"
                    native.command("model")
                    wait_for(native, "Current: " + custom_model)
                    wait_for(native, "fixture-beta")
                    assert len(fixture.snapshot("GET")) == 2
                    native.capture("durable-model-restored-with-competing-provider")
                    native.send("\x1b")
                    wait_for(native, first_request)
                    with closing(sqlite3.connect((native_data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                        preference = json.loads(db.execute("SELECT record FROM provider_preferences WHERE id=1").fetchone()[0])
                    assert preference["selections"][preference["provider"]]["model"] == custom_model
                    assert "fixture-competing-provider-not-a-real-key" not in json.dumps(preference)
                    passed("durable-exact-model-provider-restoration-without-implicit-network-" + str(width))
                    native.send("\r")  # Enter sends; no second Run or approval key.
                    wait_for(native, fixture.responses[0])
                    wait_for(native, "complete")
                    wait_for(native, "Request: " + first_request)
                    native.paste(second_request)
                    native.send("\x13")
                    stored_when(native_data, lambda rows: saved_request(rows) == second_request)
                    assert "Request: " + first_request in native.display(), "new draft relabelled the old response"
                    native.send("\r")
                    wait_for(native, fixture.responses[1])
                    wait_for(native, "2/2")
                    wait_for(native, "Request: " + second_request)
                    native.capture("native-second-response-owner")
                    native.send("\x1b[1;3A")  # Alt+Up: retain and inspect the first owned response.
                    wait_for(native, fixture.responses[0])
                    wait_for(native, "Previous request: " + first_request)
                    wait_for(native, "Current request: " + second_request)
                    native.capture("native-first-response-owner-retained")
                    native.send("\x1b[1;3B")
                    wait_for(native, fixture.responses[1])
                    assert fixture.private_reasoning not in native.text
                    assert len(fixture.snapshot("POST")) == 2
                    passed("native-fixture-two-direct-send-turns-preserve-response-owners-" + str(width))

                    native.paste(third_request)
                    native.send("\x13")
                    stored_when(native_data, lambda rows: saved_request(rows) == third_request)
                    before_refresh = inspect_store(native_data)
                    fixture.fail_models.set()
                    native.command("models-refresh")
                    wait_for(native, "Refresh failed")
                    native.capture("native-failed-refresh-retains-snapshot")
                    for model in ("fixture-alpha", "fixture-beta"):
                        native.send("\x15")
                        native.send(model)
                        wait_for(native, "Search: " + model)
                        wait_for(native, model)
                        assert "Refresh failed" in native.display()
                    native.send("\x15")
                    native.send("\x04")  # Optional complete catalogue/error details.
                    deadline = time.monotonic() + 4
                    while "Fetch error:" not in native.display():
                        assert time.monotonic() < deadline, native.display()
                        native.send("\x1b[6~")
                    native.capture("native-failed-refresh-error")
                    assert len(fixture.snapshot("GET")) == 3
                    assert len(fixture.snapshot("POST")) == 2
                    native.send("\x1b")
                    native.send("\x1b")  # Details -> picker -> retained editor.
                    assert lead_drafts(inspect_store(native_data)[0]) == lead_drafts(before_refresh[0])
                    wait_for(native, third_request)
                    passed("native-fixture-failed-metadata-refresh-retains-rows-error-and-draft-" + str(width))

                    native.send("\r")
                    deadline = time.monotonic() + 5
                    while not fixture.delayed_started.is_set():
                        assert native.alive() and time.monotonic() < deadline, native.display()
                        native.pump(0.05)
                    native.paste(unsent_request)
                    native.command("cancel")
                    deadline = time.monotonic() + 5
                    while not fixture.delayed_closed.is_set():
                        assert native.alive() and time.monotonic() < deadline, "cancel did not close the local HTTP client"
                        native.pump(0.05)
                    stored_when(native_data, lambda rows: saved_request(rows) == unsent_request)
                    native.pump(0.5)
                    assert len(fixture.snapshot("POST")) == 3, "cancelled inference was automatically resent"
                    native.send("\x1b[1;3A")
                    wait_for(native, fixture.responses[0])
                    wait_for(native, "Previous request: " + first_request)
                    native.send("\x1b[1;3B")
                    wait_for(native, fixture.responses[1])
                    wait_for(native, "Previous request: " + second_request)
                    wait_for(native, unsent_request)
                    native.capture("native-cancel-preserves-responses-and-unsent-draft")
                    requests = fixture.snapshot()
                    assert all(row["path"] == ("/v1/models" if row["method"] == "GET"
                                               else "/v1/chat/completions") for row in requests)
                    for row in requests:
                        assert not any(key.lower() in ("authorization", "cookie", "x-api-key")
                                       for key in row["headers"]), "keyless fixture received credentials"
                    payloads = fixture.snapshot("POST")
                    task = inspect_store(native_data)[0]
                    for row, request in zip(payloads, (first_request, second_request, third_request)):
                        body = row["body"]
                        assert set(body) == {"model", "messages", "stream"}, body
                        assert body["model"] == custom_model and body["stream"] is True
                        assert len(body["messages"]) == 1 and body["messages"][0]["role"] == "user"
                        context = json.loads(body["messages"][0]["content"])
                        assert context["request"] == request
                        assert context["task"]["id"] == task["id"] and context["task"]["goal"] == first_request
                        assert context["proposed_edit_paths_only"] == [] and context["approved_memories"] == []
                        assert context["handoff"] is None
                        assert unsent_request not in row["body"]["messages"][0]["content"]
                    assert task["command"] == "" and task["goal"] == first_request
                    assert fixture.private_reasoning not in native.text + json.dumps(task)
                    with closing(sqlite3.connect((native_data / "workbench.sqlite3").as_uri() + "?mode=ro", uri=True)) as db:
                        assert db.execute("SELECT count(*) FROM executions").fetchone()[0] == 0
                    assert not list(native_project.iterdir()), "native provider created project files"
                    assert not any(path.is_file() for path in native_home.rglob("*")), "native provider wrote credentials or config"
                    native.command("quit")
                    wait_for(native, "Exit with unknown outcome")
                    native.send("y")  # Explicitly acknowledge remote outcome remains unknown.
                    confirm_exit(native)
                    assert native.wait_exit() == 0
                    native.close()
                    assert len(fixture.snapshot("POST")) == 3
                    passed("native-fixture-delayed-cancel-closes-client-no-resend-or-local-effects-" + str(width))

            goal = "durable 漢字 cafe\u0301\nsecond line"
            first = launch("save")
            wait_for(first, "Storage: saved")
            first.send("\x1b")  # Legacy navigation starts outside the clean composer.
            first.command("goal")
            first.paste(goal)
            first.send("\r")
            first.command("forms")
            first.command("role-sylveon")
            wait_for(first, "Storage: saved")
            first.capture("committed")
            stored = inspect_store(data)
            assert len(stored) == 1 and (stored[0]["goal"],stored[0]["form"]) == (goal,8)
            project_identity = stored[0]["project"]
            quit_app(first)
            second = launch("resume")
            second.command("goal")
            wait_for(second, "durable 漢字 café")
            assert "Sylveon" in second.display()
            passed("exact-goal-form-save-exit-resume")
            other = root / "other project"
            other.mkdir()
            conflict = launch("writer-conflict", cwd=other)
            wait_for(conflict, "Storage: save failed")
            conflict.send("\x1b")
            conflict.command("goal")
            conflict.paste("retained writer conflict")
            conflict.send("\r")
            assert "retained writer conflict" in conflict.display()
            conflict.command("quit")
            conflict.send("\r")
            assert conflict.alive(), "neutral exit discarded a failed-save goal"
            quit_app(second)
            conflict.command("storage-retry")
            wait_for(conflict, "Storage: saved")
            quit_app(conflict)
            resumed = launch("other-project", cwd=other)
            resumed.command("goal")
            wait_for(resumed, "retained writer conflict")
            assert "durable 漢字" not in resumed.display()
            quit_app(resumed)
            passed("writer-conflict-retry-and-project-isolation")
            locked = launch("locked-sqlite")
            wait_for(locked, "Storage: saved")
            lock = sqlite3.connect(data / "workbench.sqlite3", timeout=1)
            lock.execute("BEGIN EXCLUSIVE")
            try:
                locked.command("goal")
                locked.paste(" pending locked edit")
                locked.send("\r")
                wait_for(locked, "Storage: save failed")
                locked.command("goal")  # Home summarizes only the first task line.
                assert "pending locked edit" in locked.display()
                locked.capture("save-failure")
            finally:
                lock.rollback()
                lock.close()
            locked.command("storage-retry")
            wait_for(locked, "Storage: saved")
            quit_app(locked)
            passed("sqlite-lock-failure-retains-edit-and-retries")
            for kind in ("corrupt", "newer"):
                folder = root / kind
                folder.mkdir()
                path = folder / "workbench.sqlite3"
                if kind == "corrupt": path.write_bytes(b"not a sqlite database\x00preserve me")
                else:
                    with closing(sqlite3.connect(path)) as db:
                        db.execute("CREATE TABLE future(value TEXT)")
                        db.execute("INSERT INTO future VALUES ('keep')")
                        db.execute("PRAGMA user_version=999")
                        db.commit()
                before = path.read_bytes()
                failed = launch(kind, folder)
                wait_for(failed, "Storage: save failed")
                failed.send("\x1b")
                failed.command("goal")
                failed.paste("recoverable draft")
                failed.send("\r")
                quit_app(failed)
                assert path.read_bytes() == before
                passed(kind + "-database-untouched")
            legacy = root / "legacy"
            legacy.mkdir()
            with closing(sqlite3.connect(legacy / "workbench.sqlite3")) as db:
                db.execute("CREATE TABLE goals(project TEXT PRIMARY KEY,goal TEXT NOT NULL,form INTEGER NOT NULL)")
                db.execute("INSERT INTO goals VALUES (?,?,?)", (project_identity,"migration retained",1))
                db.commit()
            migrated = launch("migration", legacy)
            migrated.command("goal")
            wait_for(migrated, "migration retained")
            wait_for(migrated, "Storage: saved")
            quit_app(migrated)
            exported = root / "recovery.json"
            command = [str(binary), "--data-dir",str(legacy),"--export-data",str(exported)]
            export = subprocess.run(command, capture_output=True, text=True, timeout=10)
            assert export.returncode == 0, export.stderr
            assert "migration retained" in exported.read_text()
            before = exported.read_bytes()
            assert subprocess.run(command, capture_output=True, timeout=10).returncode != 0
            assert exported.read_bytes() == before
            passed("transactional-migration-and-nondestructive-export")
            pending = launch("quit-after-apply", root / "pending")
            wait_for(pending, "Storage: saved")
            pending.send("\x1b")
            pending.command("goal")
            pending.paste("save before exit")
            pending.send("\r")
            pending.command("quit")
            confirm_exit(pending)
            assert pending.wait_exit() == 0
            pending.close()
            reopened = launch("quit-after-apply-resume", root / "pending")
            reopened.command("goal")
            wait_for(reopened, "save before exit")
            quit_app(reopened)
            passed("quit-after-apply-retains-committed-goal")
            for width in (120,60):
                cards = root / ("cards-" + str(width))
                size = (width,36 if width == 120 else 18)
                card = launch("cards-" + str(width), cards, size=size)
                wait_for(card, "Storage: saved")
                card.send("\x1b")
                card.command("goal")
                card.paste("card one 漢字")
                card.send("\r")
                wait_for(card, "Storage: saved")
                card.command("criteria")
                card.command("criteria-edit")
                card.paste("criterion alpha\ncriterion beta")
                card.send("\r\x1b")
                wait_for(card, "Storage: saved")
                card.command("next")
                card.paste("inspect first source")
                card.send("\r")
                card.command("forms")
                card.command("role-vaporeon")
                wait_for(card, "Storage: saved")
                card.command("goal")
                card.paste(" pending one")
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.command("task-new")
                card.command("goal")
                card.paste("card two 界")
                card.send("\r")
                wait_for(card, "Storage: saved")
                card.command("criteria")
                card.command("criteria-edit")
                card.paste("criterion gamma")
                card.send("\r\x1b")
                card.command("next")
                card.paste("inspect second source")
                card.send("\r")
                card.command("forms")
                card.command("role-jolteon")
                wait_for(card, "Storage: saved")
                card.command("goal")
                card.paste(" pending two")
                card.send("\x13")
                wait_for(card, "Storage: saved")
                rows = inspect_store(cards)
                assert len(rows) == 2
                one,two = sorted(rows,key=lambda task:task["id"])
                assert one["goal"] == "card one 漢字" and one["form"] == 1
                assert two["goal"] == "card two 界" and two["form"] == 2
                assert one["next_action"] == "inspect first source"
                assert two["next_action"] == "inspect second source"
                assert [c["text"] for c in one["criteria"]] == ["criterion alpha","criterion beta"]
                assert [c["text"] for c in two["criteria"]] == ["criterion gamma"]
                assert lead_drafts(one)[0]["text"].endswith("pending one")
                assert lead_drafts(two)[0]["text"].endswith("pending two")
                card.command("tasks")
                card.send("\x1b[H\r")
                card.command("goal")
                wait_for(card, "card one 漢字")
                wait_for(card, "Storage: saved")
                card.command("goal")
                assert "pending one" in card.display() and "pending two" not in card.display()
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.send("\t")
                card.command("layout")
                wait_for(card, "Storage: saved")
                remembered = next(t for t in inspect_store(cards) if t["id"] == one["id"])
                assert remembered["navigation"]["linear"]
                assert remembered["navigation"]["focus"] == (2 if width == 120 else 1)
                card.capture("first-resumed")
                quit_app(card)
                card = launch("cards-restart-" + str(width), cards, size=size)
                card.command("goal")
                wait_for(card, "card one 漢字")
                card.command("goal")
                assert "pending one" in card.display()
                card.send("\x13")
                wait_for(card, "Storage: saved")
                card.command("tasks")
                card.send("\x1b[F\r")
                card.command("goal")
                wait_for(card, "card two 界")
                wait_for(card, "Storage: saved")
                card.command("goal")
                assert "pending two" in card.display() and "pending one" not in card.display()
                card.send("\x1b")
                card.send("\x1b[C\r")
                wait_for(card, "Storage: saved")
                card.command("complete")
                assert not next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("criteria")
                card.command("criterion-toggle")
                card.send("\x1b")
                wait_for(card, "Storage: saved")
                card.command("complete")
                card.send("\r")
                assert not next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("complete")
                card.send("\x1b[C\r")
                wait_for(card, "Storage: saved")
                assert next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("goal")
                card.send("\r")
                assert next(t for t in inspect_store(cards) if t["id"] == two["id"])["complete"]
                card.command("goal")
                card.send("\x1b[C\r")
                card.paste(" revised")
                card.send("\r")
                wait_for(card, "Storage: saved")
                current = next(t for t in inspect_store(cards) if t["id"] == two["id"])
                assert not current["complete"] and current["goal"] == "card two 界 revised"
                card.capture("reopened")
                quit_app(card)
                passed("two-task-draft-resume-complete-reopen-" + str(width))
            lifecycle = root / "lifecycle"
            secret = "ghp_" + "Ab9" * 12
            untouched = project / "user-owned.txt"
            untouched.write_bytes(b"user bytes\r\nnot managed by ev\n")
            life = launch("lifecycle", lifecycle, size=(60,18), extra=("--retention-days","0"))
            wait_for(life, "Storage: saved")
            life.send("\x1b")
            life.command("goal")
            life.paste("private 漢字 api_key=" + secret)
            life.send("\r")
            wait_for(life, "Storage: saved")
            first_rows = inspect_store(lifecycle)
            private_id = first_rows[0]["id"]
            assert secret not in json.dumps(first_rows) and first_rows[0]["redacted"]
            assert secret.encode() not in (lifecycle / "workbench.sqlite3").read_bytes()
            life.command("task-new")
            life.command("goal")
            life.paste("retained second task")
            life.send("\r")
            wait_for(life, "Storage: saved")
            life.command("goal")
            life.paste(" password=" + secret)
            life.send("\x13")
            wait_for(life, "Storage: saved")
            rows = inspect_store(lifecycle)
            other_id = next(row["id"] for row in rows if row["id"] != private_id)
            assert secret not in json.dumps(rows)
            assert lead_drafts(next(row for row in rows if row["id"] == other_id))
            export_copy = root / "user-export.json"
            export_copy.write_text(json.dumps(rows))
            export_before = export_copy.read_bytes()
            life.command("tasks")
            life.send("\x1b[H")
            life.capture("task-cards")
            life.command("archive")
            stored_when(lifecycle, lambda rows: next(row for row in rows if row["id"] == private_id)["archived"])
            life.command("archived")
            life.capture("archived")
            life.command("restore")
            stored_when(lifecycle, lambda rows: not next(row for row in rows if row["id"] == private_id)["archived"])
            life.command("archived")
            life.send("\x1b[H")
            life.command("delete")
            wait_for(life, "Review local task deletion")
            life.capture("neutral-delete-preview")
            life.send("\r")
            assert len(inspect_store(lifecycle)) == 2, "neutral confirmation deleted data"
            life.command("delete")
            life.send("\x1b[C\r")
            rows = stored_when(lifecycle, lambda rows: len(rows) == 1)
            assert rows[0]["id"] == other_id and lead_drafts(rows[0])
            assert export_copy.read_bytes() == export_before
            assert untouched.read_bytes() == b"user bytes\r\nnot managed by ev\n"
            life.command("archive")
            stored_when(lifecycle, lambda rows: rows[0]["archived"])
            quit_app(life)
            life = launch("lifecycle-restart", lifecycle, size=(60,18), extra=("--retention-days","0"))
            wait_for(life, "Storage: saved")
            assert "retained second task" not in life.display()
            life.send("\x1b")  # All cards are archived, so this is a fresh composer.
            life.command("tasks")
            life.command("archived")
            wait_for(life, "retained second task")
            life.command("prune")
            wait_for(life, "Review local task deletion")
            life.send("\x1b[C\r")
            stored_when(lifecycle, lambda rows: rows == [])
            assert export_copy.read_bytes() == export_before and untouched.exists()
            quit_app(life)
            passed("archive-restore-neutral-delete-retention-redaction-and-scope")

            quota_data = root / "quota"
            seed = launch("quota-schema", quota_data)
            wait_for(seed, "Storage: saved")
            seed.send("\x1b")
            quit_app(seed)
            with closing(sqlite3.connect(quota_data / "workbench.sqlite3")) as db:
                schema_bytes = db.execute("PRAGMA page_count").fetchone()[0] * db.execute("PRAGMA page_size").fetchone()[0]
            # Leave a fixed payload budget above actual schema cost. A fixed
            # total conflates quota behavior with newly added schema pages and
            # can leave no room for the explicit recovery/navigation update.
            quota_kib = max(64, (schema_bytes + 1023) // 1024 + 32)
            result["quota_kib"] = quota_kib
            quota = launch("quota", quota_data, extra=("--quota-kib", str(quota_kib)))
            wait_for(quota, "Storage: saved")
            quota.send("\x1b")
            failed_goal = None
            for index in range(16):
                if index:
                    quota.command("task-new")
                quota.command("goal")
                goal = "quota card " + str(index) + " 漢字 " + "x" * 3960
                quota.paste(goal)
                quota.send("\r")
                deadline = time.monotonic() + 8
                while "Storage: saved" not in quota.display() and "Storage: save failed" not in quota.display():
                    assert time.monotonic() < deadline, quota.display()
                    quota.pump(0.05)
                if "Storage: save failed" in quota.display():
                    failed_goal = goal
                    break
            assert failed_goal is not None, "configured task quota did not reject real data"
            before = inspect_store(quota_data)
            assert len(before) >= 2 and all(row["goal"] != failed_goal for row in before)
            quota.command("goal")
            quota.paste(" retained")
            quota.send("\x13")
            quota.command("inspect")
            quota.capture("quota-failure")
            quota.send("\x1b")
            quota.command("tasks")
            quota.send("\x1b[H")
            for remaining in range(len(before)-1, 0, -1):
                quota.command("delete")
                wait_for(quota, "Review local task deletion")
                quota.send("\x1b[C\r")
                stored_when(quota_data, lambda rows: len(rows) == remaining)
            quota.command("storage-retry")
            wait_for(quota, "Storage: saved")
            rows = inspect_store(quota_data)
            recovered = next(row for row in rows if row["goal"] == failed_goal)
            assert lead_drafts(recovered)[0]["text"] == failed_goal + " retained"
            assert {row["id"] for row in rows} == {before[-1]["id"], recovered["id"]}
            quota.send("\x1b")
            quota.command("goal")
            assert "retained" in quota.display()
            quota.send("\x13")
            wait_for(quota, "Storage: saved")
            quit_app(quota)
            passed("real-task-quota-retains-draft-and-explicit-delete-recovers")
        result["status"] = "passed"
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        for session in sessions: session.close()
        (output / "results.json").write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n")


if __name__ == "__main__":
    main()
