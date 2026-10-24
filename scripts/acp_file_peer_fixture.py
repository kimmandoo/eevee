#!/usr/bin/env python3
"""Deterministic ACP-v1 test peer; no provider/auth/network calls or claimed conformance.

Invoked only by the isolated Rust ACP integration test. It requests actual host
file callbacks and checks their wire responses; it never writes project files.
"""
import json
import os
from pathlib import Path
import sys


def record(direction, value):
    path = os.environ.get("EEVEE_ACP_FILE_LOG")
    if path:
        with open(path, "a", encoding="utf-8") as stream:
            stream.write(json.dumps(dict(direction=direction, packet=value)) + "\n")


def emit(value):
    record("peer", value)
    print(json.dumps(dict(jsonrpc="2.0", **value)), flush=True)


def receive():
    line = sys.stdin.readline()
    if not line:
        raise SystemExit(0)
    value = json.loads(line)
    record("client", value)
    return value


def main():
    name = Path(sys.argv[0]).name
    if "--version" in sys.argv or "version" in sys.argv:
        print("devin 3000.11.3 (012345abcdef)" if name == "devin" else "1.18.0")
        return
    if os.environ.get("EEVEE_ACP_FILE_TEST_CHILD") != "1":
        raise SystemExit("This peer is only for isolated ACP integration tests")
    session = "owned-acp-files"
    root = None
    while True:
        packet = receive()
        method, identity = packet.get("method"), packet.get("id")
        if method == "initialize":
            caps = packet["params"]["clientCapabilities"]
            assert caps["fs"] == dict(readTextFile=True, writeTextFile=True)
            assert caps["terminal"] is True
            emit(dict(id=identity, result=dict(protocolVersion=1, agentCapabilities={})))
        elif method == "session/new":
            root = Path(packet["params"]["cwd"])
            emit(dict(id=identity, result=dict(sessionId=session)))
        elif method == "session/prompt":
            payload = json.loads(packet["params"]["prompt"][0]["text"])
            mode = os.environ.get("EEVEE_ACP_FILE_MODE", payload["request"])
            prompt_id = identity
            break
        else:
            raise AssertionError(method)

    serial = 0

    def call(method, **params):
        nonlocal serial
        serial += 1
        request_id = "file-" + str(serial)
        emit(dict(id=request_id, method=method, params=dict(sessionId=session, **params)))
        while True:
            answer = receive()
            if answer.get("method") == "session/cancel":
                emit(dict(id=prompt_id, result=dict(stopReason="cancelled")))
                raise SystemExit(0)
            if answer.get("id") == request_id:
                return answer

    def report(status):
        emit(dict(method="session/update", params=dict(sessionId=session, update=dict(
            sessionUpdate="tool_call_update", toolCallId="external-edit", status=status))))

    if os.environ.get("EEVEE_ACP_GENERIC_PERMISSION") == "1":
        for decision in ("cancelled", "selected"):
            result = call("session/request_permission",
                          toolCall=dict(toolCallId="generic-" + decision,
                                        title="External fixture operation", kind="other",
                                        status="pending", rawInput=dict(scope="fixture-only")),
                          options=[dict(optionId="once", name="Allow external operation once",
                                        kind="allow_once")])
            expected = dict(outcome=decision)
            if decision == "selected":
                expected["optionId"] = "once"
            assert result.get("result", {}).get("outcome") == expected, result

    emit(dict(method="session/update", params=dict(sessionId=session, update=dict(
        sessionUpdate="tool_call", toolCallId="external-edit", title="External diff observation",
        kind="edit", status="in_progress", content=[dict(type="diff", path=str(root / "reported.rs"),
        oldText=None, newText="fn observed_only() {}\n")]))))
    report("failed" if mode == "reported-failed" else "completed")

    before = call("fs/read_text_file", path=str(root / "existing.txt"))
    assert before["result"]["content"] == "original\r\nkeep dirty bytes\r\n"
    partial = call("fs/read_text_file", path=str(root / "existing.txt"), line=2, limit=50)
    assert partial["result"]["content"] == "keep dirty bytes\r\n"
    if mode in ("happy", "reported-failed", "duplicate", "disconnect"):
        result = call("fs/write_text_file", path=str(root / "nested" / "new.txt"), content="actual new file\n")
        assert result.get("result") == {}, result
        if mode == "duplicate":
            emit(dict(id="file-3", method="fs/write_text_file", params=dict(sessionId=session,
                      path=str(root / "nested" / "new.txt"), content="must not replay\n")))
            receive()
            return
        if mode == "disconnect":
            return
        result = call("fs/read_text_file", path=str(root / "nested" / "new.txt"))
        assert result["result"]["content"] == "actual new file\n"
        result = call("fs/write_text_file", path=str(root / "existing.txt"),
                      content="edited\r\nkeep dirty bytes\r\n")
        assert result.get("result") == {}, result
    elif mode in ("outside", "protected", "symlink", "excluded"):
        target = {"outside": root.parent / "outside.txt", "protected": root / ".eevee-data" / "blocked.txt",
                  "symlink": root / "linked.txt", "excluded": root / "excluded.txt"}[mode]
        result = call("fs/read_text_file", path=str(target))
        assert "error" in result, result
        result = call("fs/write_text_file", path=str(target), content="must not write\n")
        assert "error" in result, result
    else:
        target = root / "nested" / "new.txt" if mode == "parents" else root / "existing.txt"
        result = call("fs/write_text_file", path=str(target), content="must not report success\n")
        assert "error" in result, result
    if mode == "cancel":
        while receive().get("method") != "session/cancel":
            pass
        emit(dict(id=prompt_id, result=dict(stopReason="cancelled")))
    else:
        emit(dict(id=prompt_id, result=dict(stopReason="end_turn")))
    # Retain a live owned process until the client observes the terminal frame.
    while sys.stdin.readline():
        pass


if __name__ == "__main__":
    main()
