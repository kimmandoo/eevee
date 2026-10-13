#!/usr/bin/env python3
"""No-model ACP handshake/session evidence; never authenticate or inspect credentials.

Pass an inspected executable explicitly with --trusted-executable. Runs in a disposable
cwd, drains/discards stderr, refuses all client requests, and never sends session/prompt.
Provider-owned config/auth effects can still occur; this is not a sandbox.
"""
import argparse
import json
import os
import selectors
import signal
import subprocess
import tempfile
import time

LIMIT = 2 * 1024 * 1024


def supports(capabilities, method):
    if method == "session/load":
        return capabilities.get("loadSession") is True
    if method == "session/list":
        return isinstance(capabilities.get("sessionCapabilities", {}).get("list"), dict)
    return False

def version_probe(executable):
    process = subprocess.Popen([executable, "--version"], cwd=tempfile.gettempdir(),
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    output = bytearray()
    deadline = time.monotonic() + 30
    try:
        while time.monotonic() < deadline:
            for key, _ in selector.select(.1):
                chunk = os.read(key.fd, 4096)
                if not chunk:
                    if process.wait(timeout=1) != 0:
                        raise RuntimeError("Trusted version probe failed; raw output withheld")
                    return output.decode("utf-8", errors="strict").strip()
                output.extend(chunk)
                if len(output) > 16384:
                    raise RuntimeError("Version output limit exceeded; raw output withheld")
        raise RuntimeError("Trusted version probe timed out")
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=5)
        selector.close()


class Peer:
    def __init__(self, argv, cwd):
        self.process = subprocess.Popen(argv, cwd=cwd, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                        start_new_session=True)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b""
        self.sequence = 0
        self.methods = []

    def send(self, value):
        self.process.stdin.write((json.dumps(value) + "\n").encode())
        self.process.stdin.flush()

    def call(self, method, params):
        self.sequence += 1
        identity = self.sequence
        self.methods.append(method)
        self.send({"jsonrpc": "2.0", "id": identity, "method": method, "params": params})
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            for key, _ in self.selector.select(.1):
                chunk = os.read(key.fd, 65536)
                if not chunk:
                    raise RuntimeError("Peer exited without correlated response")
                self.buffer += chunk
                if len(self.buffer) > LIMIT:
                    raise RuntimeError("ACP frame limit exceeded")
                while b"\n" in self.buffer:
                    line, self.buffer = self.buffer.split(b"\n", 1)
                    try:
                        frame = json.loads(line)
                    except (ValueError, UnicodeError):
                        raise RuntimeError("Nonprotocol stdout; raw bytes withheld") from None
                    if not isinstance(frame, dict) or frame.get("jsonrpc") != "2.0":
                        raise RuntimeError("Invalid ACP envelope")
                    if "method" in frame and "id" in frame:
                        if frame["method"] == "session/request_permission":
                            self.send({"jsonrpc": "2.0", "id": frame["id"],
                                       "result": {"outcome": {"outcome": "cancelled"}}})
                        else:
                            self.send({"jsonrpc": "2.0", "id": frame["id"], "error": {
                                "code": -32601, "message": "Probe does not implement client callbacks"}})
                    elif frame.get("id") == identity:
                        return frame
        raise RuntimeError("ACP request deadline expired")

    def close(self):
        try:
            os.killpg(self.process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.process.wait(timeout=5)
        self.selector.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", choices=("opencode", "gemini"), required=True)
    parser.add_argument("--trusted-executable", required=True,
                        help="Exact already inspected native provider executable; deliberate execution consent")
    parser.add_argument("--session-list", action="store_true",
                        help="Exercise negotiated optional list, or demonstrate refusal without invocation")
    args = parser.parse_args()
    executable = os.path.realpath(args.trusted_executable)
    expected = "1.18.0" if args.agent == "opencode" else "0.28.0"
    observed = version_probe(executable)
    if observed != expected:
        raise RuntimeError("Provider version mismatch/nonversion output; raw bytes withheld")
    launch = [executable, "acp", "--pure"] if args.agent == "opencode" else [executable, "--experimental-acp"]
    with tempfile.TemporaryDirectory(prefix="eevee-acp-probe-") as cwd:
        peer = Peer(launch, cwd)
        try:
            response = peer.call("initialize", {"protocolVersion": 1, "clientInfo": {
                "name": "eevee-probe", "version": "1"}, "clientCapabilities": {
                    "fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}})
            initialized = response.get("result", {})
            if initialized.get("protocolVersion") != 1:
                raise RuntimeError("ACP v1 negotiation refused")
            raw_capabilities = initialized.get("agentCapabilities", {})
            # Record known capability shapes only, not arbitrary credential-bearing extension metadata.
            capabilities = {}
            if isinstance(raw_capabilities.get("loadSession"), bool):
                capabilities["loadSession"] = raw_capabilities["loadSession"]
            for group, keys in (("promptCapabilities", ("image", "audio", "embeddedContext")),
                                ("mcpCapabilities", ("http", "sse"))):
                values = raw_capabilities.get(group, {})
                if isinstance(values, dict):
                    fields = {key: values[key] for key in keys if isinstance(values.get(key), bool)}
                    if fields:
                        capabilities[group] = fields
            session_caps = raw_capabilities.get("sessionCapabilities", {})
            if isinstance(session_caps, dict):
                fields = {key: {} for key in ("list", "fork", "resume", "close")
                          if isinstance(session_caps.get(key), dict)}
                if fields:
                    capabilities["sessionCapabilities"] = fields
            record = {"agent": args.agent, "version": observed, "launch_flags": launch[1:],
                      "protocol": 1, "capabilities": capabilities,
                      "auth_method_ids": [method.get("id") for method in initialized.get("authMethods", [])],
                      "client_callbacks": "fs=false,terminal=false; unsupported requests refused",
                      "boundary": "external provider policy, not a sandbox",
                      "model_prompts_sent": 0}
            if args.session_list:
                if supports(capabilities, "session/list"):
                    result = peer.call("session/list", {"cwd": cwd})
                    record["session_list"] = {"invoked": True, "success": "result" in result,
                                              "error_code": result.get("error", {}).get("code")}
                else:
                    record["session_list"] = {"invoked": False, "refusal": "Peer did not negotiate session/list"}
            session = peer.call("session/new", {"cwd": cwd, "mcpServers": []})
            if "result" in session:
                result = session["result"]
                record["session_new"] = {"success": True, "session_id": result.get("sessionId"),
                                         "modes": [mode.get("id") for mode in result.get("modes", {}).get("availableModes", [])],
                                         "model": next((option.get("currentValue") for option in result.get("configOptions", [])
                                                        if option.get("category") == "model"), None)}
            else:
                error = session.get("error", {})
                # Do not echo provider error text/credential-bearing fields; retain safe source classification.
                record["session_new"] = {"success": False, "error_code": error.get("code"),
                                         "reason": "Provider authentication/configuration rejected session; raw details withheld"}
            record["invoked_methods"] = peer.methods
            print(json.dumps(record, ensure_ascii=False, indent=2))
        finally:
            peer.close()


if __name__ == "__main__":
    main()
