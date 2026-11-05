#!/usr/bin/env python3
"""Actual Devin native OAuth-key/Connect/protobuf + host-tool PTY acceptance.

Requires an already-built binary. Uses synthetic credentials, isolated HOME and
an empty provider-program PATH. The loopback peer requests tools; only Eevee
writes the project and launches the actual supplied Python executable. This is
local wire/ownership evidence, not paid-account or live-model qualification.
"""
from __future__ import annotations
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import struct
import sys
import tempfile
import threading
import traceback
from unittest.mock import patch

from response_smoke import SurfaceJourney
from native_connection_smoke import until
from connection_smoke_helpers import focus_field, fill_text
from terminal_smoke import confirm_exit, permission_run_available, inspect_permission_scope
from workbench_smoke import wait_for

MODEL = 'fixture-exact-router-high'
ACTUAL = 'fixture-exact-concrete-high'
SECRET = 'synthetic-devin-fixture-key-8f24'
JWT = 'synthetic-user-jwt'
ASSIGNMENT = 'synthetic-assignment-jwt'
SOURCE = 'native_devin_proof.py'
CONTENT = 'print("DEVIN-NATIVE-COMMAND:42")\n'
FINAL = 'DEVIN_NATIVE_FIRST_COMPLETE'
FOLLOW = 'DEVIN_NATIVE_FOLLOWUP_COMPLETE'
API = '/exa.api_server_pb.ApiServerService/'
AUTH = '/exa.auth_pb.AuthService/GetUserJwt'
ESC = '\x1b'


def varint(number):
    out = bytearray()
    while number >= 128:
        out.append((number & 127) | 128)
        number >>= 7
    out.append(number)
    return bytes(out)


def integer(field, number):
    return varint(field << 3) + varint(number)


def blob(field, value):
    if isinstance(value, str):
        value = value.encode()
    return varint((field << 3) | 2) + varint(len(value)) + value


def fields(data):
    offset = 0
    result = {}

    def number():
        nonlocal offset
        value = 0
        for shift in range(0, 70, 7):
            assert offset < len(data), 'truncated protobuf varint'
            byte = data[offset]
            offset += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
        raise AssertionError('overflowing protobuf varint')

    while offset < len(data):
        key = number()
        assert key >> 3
        wire = key & 7
        if wire == 0:
            value = number()
        elif wire in (1, 2, 5):
            count = number() if wire == 2 else 8 if wire == 1 else 4
            assert count <= len(data) - offset, 'truncated protobuf field'
            value = data[offset:offset + count]
            offset += count
        else:
            raise AssertionError('unsupported protobuf wire')
        result.setdefault(key >> 3, []).append(value)
    return result


def one(message, field, default=b''):
    values = message.get(field, [default])
    assert len(values) == 1, 'duplicate protobuf scalar'
    return values[0]


def text(message, field):
    return one(message, field).decode()


def frame(payload, flags=0):
    return bytes([flags]) + struct.pack('>I', len(payload)) + payload


def delta(*, answer='', call=None, stop=0, number=0):
    value = blob(1, 'fixture-response-' + str(number)) + blob(23, ACTUAL)
    if answer:
        value += blob(3, answer)
    if call:
        identity, name, arguments = call
        value += blob(6, blob(1, identity) + blob(2, name) + blob(3, arguments))
    if stop:
        value += integer(5, stop)
    value += blob(7, integer(2, 100) + integer(3, 20) + integer(4, 3) + integer(5, 4))
    return frame(value)


def prompts(message):
    result = []
    for raw in message.get(3, []):
        row = fields(raw)
        result.append({'id': text(row, 1), 'source': one(row, 2, 0), 'text': text(row, 3),
                       'tool_call_id': text(row, 7), 'thinking': text(row, 11),
                       'calls': [{'id': text(fields(call), 1), 'name': text(fields(call), 2),
                                  'arguments': text(fields(call), 3)} for call in row.get(6, [])]})
    return result


class Peer:
    def __init__(self):
        self.secret = SECRET
        self.lock = threading.Lock()
        self.requests, self.errors = [], []
        self.plans = queue.Queue()
        self.stopped, self.cancel_started, self.cancel_closed = (threading.Event() for _ in range(3))
        self.assignments = {}
        self.auth_base_override = ''
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_args):
                pass

            def reply(self, payload, streaming=False):
                self.send_response(200)
                self.send_header('Content-Type', 'application/connect+proto' if streaming else 'application/proto')
                self.send_header('Content-Length', str(len(payload)))
                self.send_header('Connection', 'close')
                self.end_headers()
                # Deliberately split Connect headers and protobuf UTF-8 boundaries.
                for offset in range(0, len(payload), 3):
                    self.wfile.write(payload[offset:offset + 3])
                self.wfile.flush()
                self.close_connection = True

            def do_POST(self):
                try:
                    count = int(self.headers.get('Content-Length', '0'))
                    assert 0 < count <= 8 * 1024 * 1024
                    raw = self.rfile.read(count)
                    chat = self.path == API + 'GetChatMessage'
                    if chat:
                        assert raw[0] == 0 and int.from_bytes(raw[1:5], 'big') == len(raw) - 5
                        raw = raw[5:]
                    message = fields(raw)
                    meta = fields(one(message, 1))
                    assert text(meta, 3) == SECRET, 'selected credential not in protobuf metadata'
                    assert 'Authorization' not in self.headers, 'native metadata also leaked bearer header'
                    row = {'method': 'POST', 'path': self.path, 'credential_matches': True}
                    if chat:
                        assert text(meta, 21) == JWT
                        assert text(message, 21) == ACTUAL, 'router wire identity guessed or not assigned'
                        cascade = text(message, 16)
                        assert text(message, 26) == self_assignment(cascade)
                        row.update(prompts=prompts(message), cascade=cascade, model=text(message, 21),
                                   tools=[text(fields(tool), 1) for tool in message.get(10, [])])
                    with fixture.lock:
                        fixture.requests.append(row)
                    if self.path == API + 'GetCliModelConfigs':
                        info = integer(22, 3) + integer(25, 1) + integer(13, 4096)
                        config = blob(1, MODEL) + blob(22, MODEL) + blob(23, info)
                        hidden = blob(22, 'internal-never-visible') + blob(23, integer(22, 6))
                        disabled = blob(22, 'disabled-never-visible') + integer(4, 1)
                        self.reply(blob(1, config) + blob(1, hidden) + blob(1, disabled))
                    elif self.path == AUTH:
                        self.reply(blob(1, JWT) + blob(2, fixture.auth_base_override))
                    elif self.path == API + 'AssignModel':
                        assert text(message, 2) == MODEL
                        cascade = text(message, 3)
                        with fixture.lock:
                            fixture.assignments[cascade] = ASSIGNMENT
                        self.reply(blob(1, blob(1, ASSIGNMENT) + blob(2, ACTUAL)))
                    elif chat:
                        mode, value = fixture.plans.get(timeout=2)
                        row['mode'] = mode
                        number = len(fixture.chats())
                        if mode == 'call':
                            identity, name, args = value
                            encoded = json.dumps(args, ensure_ascii=False)
                            split = len(encoded) // 2
                            payload = delta(call=(identity, name, encoded[:split]), number=number)
                            # Native supports cumulative argument snapshots.
                            payload += delta(call=('', '', encoded), stop=10, number=number)
                            self.reply(payload + frame(b'{}', 2), True)
                        elif mode == 'answer':
                            self.reply(delta(answer=value, stop=2, number=number) + frame(b'{}', 2), True)
                        elif mode == 'error':
                            args = json.dumps({'path': 'must-not-exist.txt', 'content': 'no authority'})
                            payload = delta(call=('late_error', 'eevee_write_file', args), stop=10, number=number)
                            self.reply(payload + frame(json.dumps({'error': {'code': 'permission_denied', 'message': 'fixture failure'}}).encode(), 2), True)
                        elif mode == 'cancel':
                            self.send_response(200)
                            self.send_header('Content-Type', 'application/connect+proto')
                            self.send_header('Connection', 'close')
                            self.end_headers()
                            self.wfile.write(delta(answer='Devin partial never canonical', number=number))
                            self.wfile.flush()
                            fixture.cancel_started.set()
                            while not fixture.stopped.wait(.05):
                                try:
                                    self.wfile.write(frame(b''))
                                    self.wfile.flush()
                                except OSError:
                                    fixture.cancel_closed.set()
                                    break
                            self.close_connection = True
                        else:
                            raise AssertionError('unexpected response plan ' + mode)
                    else:
                        raise AssertionError('unexpected native RPC ' + self.path)
                except Exception as error:
                    with fixture.lock:
                        fixture.errors.append(str(error))
                    self.close_connection = True

        def self_assignment(cascade):
            with fixture.lock:
                return fixture.assignments[cascade]

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.base = 'http://127.0.0.1:' + str(self.server.server_port)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .05})
        self.thread.start()

    def snapshot(self, method=None):
        with self.lock:
            return [dict(row) for row in self.requests if method is None or row['method'] == method]

    def chats(self):
        return [row for row in self.snapshot() if row['path'] == API + 'GetChatMessage']

    def close(self):
        self.stopped.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        assert not self.thread.is_alive(), 'Devin peer did not stop'


class Journey(SurfaceJourney):
    def setup(self):
        session = self.session
        wait_for(session, 'Storage: saved')
        session.command('connect devin')
        until(session, lambda: 'oauth' in session.display().lower(), 'Devin native account method picker')
        session.send(ESC + '[B')
        session.send('\r')
        wait_for(session, 'API key:')
        focus_field(session, 'key')
        self.private_paste(SECRET)
        focus_field(session, 'advanced')
        session.send('\r')
        fill_text(session, 'endpoint', self.peer.base)
        fill_text(session, 'model', MODEL)
        focus_field(session, 'storage')
        session.send(' ')  # Explicit process-session storage, no persistent fixture secret.
        focus_field(session, 'save')
        self.capture('review-native-loopback-connection')
        session.send('\r')
        wait_for(session, MODEL)
        until(session, lambda: len(self.peer.snapshot()) == 1, 'one explicit native metadata POST')
        wait_for(session, '1 matches')
        assert not self.peer.chats(), 'connection setup dispatched inference'
        assert 'internal-never-visible' not in session.display()
        session.send(MODEL)
        session.send('\r')
        self.workspace()

    def submit(self, request, plans, *, expect_chat=True):
        self.write_request(request)
        before = len(self.peer.chats())
        before_auth = sum(row['path'] == AUTH for row in self.peer.snapshot())
        for plan in plans:
            self.peer.plans.put(plan)
        self.session.send('\r', settle=False)
        approved = False

        def submitted():
            nonlocal approved
            if len(self.peer.chats()) > before:
                return True
            if not expect_chat and sum(row['path'] == AUTH for row in self.peer.snapshot()) > before_auth:
                return True
            if permission_run_available(self.session) and not approved:
                self.session.send('y', settle=False)
                approved = True
            elif not approved and inspect_permission_scope(self.session):
                pass
            elif 'Alt+A' in self.session.display():
                self.session.send(ESC + 'a', settle=False)
            return False

        until(self.session, submitted, 'Devin native request dispatch', timeout=40)
        return before

    def settled(self, answer=None):
        until(self.session, lambda: not self.snapshot()['claims'] and (answer is None or answer in self.session.display()),
              'native logical request and store settlement', timeout=60)
        self.workspace()


def run(binary, output, root, size, python):
    peer = Peer()
    journey = None
    result = {'size': list(size), 'status': 'failed'}
    try:
        clean = {key: value for key, value in os.environ.items()
                 if not key.startswith(('DEVIN_', 'WINDSURF_', 'CODEIUM_'))}
        with patch.dict(os.environ, clean, clear=True):
            journey = Journey(binary, output, root, size, size == (60, 18), 'devin', 'devin', False, False, peer)
        session = journey.session
        journey.setup()
        wait_for(session, 'AUTO-APPROVE')
        peer.auth_base_override = 'https://unreviewed.invalid/private-api'
        journey.submit('Refuse a server-directed unreviewed destination before sending task content.', [], expect_chat=False)
        journey.settled()
        assert not peer.chats(), 'foreign auth destination dispatched a chat request'
        assert journey.snapshot()['executions'] == 0
        assert not list(journey.project.iterdir()), 'foreign auth destination caused a host effect'
        assert 'unreviewed.invalid' not in session.display(), 'untrusted routing URL entered diagnostics'
        journey.capture('foreign-auth-destination-refused-before-chat')
        peer.auth_base_override = ''
        first = 'Create the requested proof program and run it through the host command tool.'
        command = {'cwd': '', 'command': {'Direct': {'program': str(python), 'args': ['-I', SOURCE]}}}
        journey.submit(first, [
            ('call', ('write_proof', 'eevee_write_file', {'path': SOURCE, 'content': CONTENT})),
            ('call', ('run_proof', 'eevee_run_command', command)), ('answer', FINAL)])
        journey.settled(FINAL)
        assert (journey.project / SOURCE).read_bytes() == CONTENT.encode()
        assert journey.snapshot()['executions'] == 1, 'real host command was not retained exactly once'
        chats = peer.chats()
        assert len(chats) == 3
        results = {row['tool_call_id']: json.loads(row['text']) for row in chats[2]['prompts'] if row['source'] == 4}
        assert results['write_proof']['status'] == 'success'
        assert results['run_proof']['status'] == 'success'
        assert results['run_proof']['run']['exit_code'] == 0
        assert 'DEVIN-NATIVE-COMMAND:42' in results['run_proof']['stdout']
        assert all(row['cascade'] == chats[0]['cascade'] for row in chats)
        assert {'eevee_write_file', 'eevee_run_command', 'todo'} <= set(chats[0]['tools'])
        journey.capture('native-real-file-command-completion')
        follow = 'Retain my first Request and the prior public answer; no repeated effects.'
        journey.submit(follow, [('answer', FOLLOW)])
        journey.settled(FOLLOW)
        retained = [(row['source'], row['text']) for row in peer.chats()[-1]['prompts']]
        assert retained == [(1, first), (2, FINAL), (1, follow)], retained
        assert journey.snapshot()['executions'] == 1
        journey.capture('native-followup-public-history')
        journey.submit('Exercise a late stream failure without file authority.', [('error', None)])
        journey.settled()
        assert not (journey.project / 'must-not-exist.txt').exists()
        journey.capture('late-error-no-tool-effect')
        before = journey.submit('Cancel the incomplete wire without replaying effects.', [('cancel', None)])
        until(session, peer.cancel_started.is_set, 'actual native stream pending')
        session.command('cancel', settle=False)
        until(session, peer.cancel_closed.is_set, 'cancel closes native response socket')
        journey.settled()
        session.pump(.3)
        assert len(peer.chats()) == before + 1, 'cancel automatically replayed model request'
        assert journey.snapshot()['executions'] == 1
        assert not (journey.project / 'must-not-exist.txt').exists()
        assert not peer.errors, peer.errors
        assert all(secret not in session.text for secret in (SECRET, JWT, ASSIGNMENT))
        journey.capture('cancelled-reader-no-replay')
        session.command('quit')
        confirm_exit(session)
        result.update(status='passed', cases=['foreign-auth-destination-no-chat-or-effects', 'authenticated-metadata-exact-visible-model', 'GetUserJwt',
                      'router-AssignModel', 'real-host-file-and-command', 'followup-public-roles',
                      'late-trailer-error-no-effect', 'cancel-no-replay'], requests=peer.snapshot())
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc(), peer_errors=peer.errors, requests=peer.snapshot())
        if journey:
            journey.capture('failed')
    finally:
        if journey:
            journey.close()
        peer.close()
        (output / 'results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--python', type=Path, default=Path(sys.executable))
    args = parser.parse_args()
    binary, output, python = args.binary.resolve(), args.output.resolve(), args.python.resolve()
    assert binary.is_file() and python.is_file()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    results = []
    with tempfile.TemporaryDirectory(prefix='eevee-devin-native-') as temporary:
        for size in ((120, 36), (60, 18)):
            name = str(size[0]) + 'x' + str(size[1])
            evidence = output / name
            evidence.mkdir()
            root = Path(temporary) / name
            root.mkdir()
            results.append(run(binary, evidence, root, size, python))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    summary = {'status': 'passed' if all(row['status'] == 'passed' for row in results) else 'failed',
               'binary_sha256': digest, 'results': results,
               'limits': ['Controlled local native wire/host effects only; no live Devin entitlement, browser account, model understanding or billing claim.']}
    (output / 'results.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'status': summary['status'], 'output': str(output)}))
    return 0 if summary['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
