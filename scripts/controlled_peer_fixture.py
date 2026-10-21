#!/usr/bin/env python3
"""Deterministic controlled-write Codex JSONL peer; not provider conformance.

Disposable smoke fixtures use --fake-peer and the EV_COOP_* environment only.
The runtime still performs the real reviewed create-or-replace write and durable journal.
"""
import json
import os
from pathlib import Path
import re
import select
import sys


def peer():
    if "--version" in sys.argv:
        print("codex-cli 0.152.1")
        return
    control = Path(os.environ["EV_COOP_CONTROL"])
    log = Path(os.environ["EV_COOP_LOG"])
    thread, turn = "coop-thread-" + str(os.getpid()), "coop-turn-" + str(os.getpid())
    marker, completed, asked = None, False, False
    observation_count, observations_sent, turn_count = 0, 0, 0
    cwd = os.getcwd()
    def record(kind, **fields):
        with log.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(dict(kind=kind, marker=marker, pid=os.getpid(), **fields)) + "\n")
    def emit(value):
        print(json.dumps(value), flush=True)
    def event(method, **params):
        emit(dict(method=method, params=dict(threadId=thread, turnId=turn, **params)))
    def finish(status="completed"):
        nonlocal completed
        if completed:
            return
        completed = True
        # Eevee may reap the transport immediately after the terminal frame.
        # Log intent first; journeys separately await real durable settlement.
        record("finished", status=status)
        if status == "completed":
            event("item/completed", item=dict(id="answer", type="agentMessage", text="Completed " + marker))
        event("turn/completed", turn=dict(id=turn, status=status, items=[]))
    buffered = b""
    while True:
        if b"\n" not in buffered:
            ready, _, _ = select.select([sys.stdin], [], [], 0.03)
            if ready:
                chunk = os.read(sys.stdin.fileno(), 65536)
                if not chunk:
                    return
                buffered += chunk
        if b"\n" in buffered:
            line, buffered = buffered.split(b"\n", 1)
            packet = json.loads(line)
            method, identity = packet.get("method"), packet.get("id")
            params = packet.get("params", {})
            if method is None and identity is not None:
                record("answer", request_id=identity, result=packet.get("result"), error=packet.get("error"))
                continue
            if identity is None:
                continue
            if method == "initialize": result = dict(userAgent="cooperation-fixture")
            elif method == "account/read": result = dict(account=None, requiresOpenaiAuth=False)
            elif method == "config/read": result = dict(config=dict(model="fixture-coop"))
            elif method == "model/list": result = dict(data=[dict(model="fixture-coop", displayName="Fixture cooperation", isDefault=True)], nextCursor=None)
            elif method in ("thread/start", "thread/resume"):
                thread = params.get("threadId", thread)
                record("session", method=method, thread=thread)
                cwd = params["cwd"]
                result = dict(thread=dict(id=thread), cwd=cwd, model=params.get("model", "fixture-coop"), reasoningEffort=params.get("config", {}).get("model_reasoning_effort"), approvalPolicy="on-request", approvalsReviewer="user", sandbox=dict(type="readOnly", networkAccess=False))
            elif method == "turn/start":
                text = "\n".join(item.get("text", "") for item in params.get("input", []))
                markers = re.findall(r"COOP_[A-Z0-9_]+", text)
                marker = markers[-1] if markers else "COOP_UNKNOWN"
                completed, asked = False, False
                turn_count += 1
                turn = f"coop-turn-{os.getpid()}-{turn_count}"
                result = dict(turn=dict(id=turn, status="inProgress", items=[]))
                emit(dict(id=identity, result=result))
                record("started", text=text)
                event("item/started", item=dict(id="answer", type="agentMessage", text=""))
                event("item/agentMessage/delta", itemId="answer", delta="Streaming " + marker + ". ")
                # An observed provider item is deliberately NOT a local run.
                batch = control / (marker + ".observations")
                observation_count = int(batch.read_text()) if batch.exists() else 1
                if not 1 <= observation_count <= 300:
                    raise ValueError("Fixture observation count must be between 1 and 300")
                observations_sent = 0
                continue
            elif method == "turn/interrupt":
                emit(dict(id=identity, result={}))
                finish("interrupted")
                continue
            else:
                emit(dict(id=identity, error=dict(code=-32601, message="Unsupported fixture method " + str(method))))
                continue
            emit(dict(id=identity, result=result))
        if not marker or completed:
            continue
        if observations_sent < observation_count:
            gate = control / (marker + ".observe-next")
            allowed = int(gate.read_text()) if gate.exists() else 1
            if observations_sent < allowed:
                item_id = "observation" if observation_count == 1 else f"observation-{observations_sent}"
                event("item/completed", item=dict(id=item_id, type="commandExecution", command="fixture observation only", cwd=cwd, status="completed", aggregatedOutput=marker + " public observation", exitCode=0))
                observations_sent += 1
                if observations_sent == observation_count and observation_count > 1:
                    record("observed_batch", count=observation_count)
        prompt = control / (marker + ".prompt")
        if prompt.exists() and not asked:
            asked = True
            mode = prompt.read_text().strip()
            request_id = marker + "-request"
            params = dict(threadId=thread, turnId=turn, itemId="question", callId="edit-call")
            if mode == "edit":
                method = "item/tool/call"
                params.update(tool="eevee_write_file", arguments=dict(path=os.environ["EV_COOP_EDIT_PATH"], content=os.environ["EV_COOP_EDIT_CONTENT"]))
            elif mode == "question":
                method = "item/tool/requestUserInput"
                params.update(isBlocking=True, questions=[dict(id="choice", header="Owner input", question="Literal answer for " + marker, options=[], isOther=True, isSecret=False)])
            elif mode == "approval":
                method = "item/commandExecution/requestApproval"
                params.update(command="echo " + marker, cwd=cwd, reason="One-time approval for " + marker, availableDecisions=["decline", "accept"])
            else:
                raise ValueError("Unsupported controlled fixture prompt: " + mode)
            emit(dict(id=request_id, method=method, params=params))
            record("prompt", request_id=request_id, mode=mode)
        if (control / (marker + ".finish")).exists():
            finish()


if __name__ == "__main__":
    if "--fake-peer" not in sys.argv:
        raise SystemExit("This test peer requires --fake-peer")
    peer()
