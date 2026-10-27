#!/usr/bin/env python3
"""Actual native WebSocket/TUI qualification against an already-built ev.

Run: python3 scripts/native_websocket_smoke.py --binary /absolute/path/to/ev \
    --output /absolute/path/to/new-evidence-directory

Uses the existing pyte/wcwidth PTY (ConPTY on Windows) harness. The HTTP upgrade,
WebSocket framing, masked client messages, fragmentation and controls use only
Python's standard library. Compatible Responses setup is performed through the
real connection UI, not a production endpoint override. No build, live account,
model, external service, SQL mutation or provider program is involved.
"""
from __future__ import annotations

import argparse
import base64
from contextlib import closing
import hashlib
import json
from pathlib import Path
import queue
import socket
import sqlite3
import struct
import tempfile
import time
import traceback
from unittest.mock import patch

import response_smoke
from file_creation_smoke import CreationJourney, file_review_pending, inspect_to_bottom, normalized, save
from native_connection_smoke import MODEL, Peer, content_text, until
from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available
from workbench_smoke import wait_for

ESC = '\x1b'
DOWN, RIGHT, LEFT = ESC + '[B', ESC + '[C', ESC + '[D'
FRAME_LIMIT = 16 * 1024 * 1024
FIRST, SECOND = 'first-界.txt', 'second-café.txt'
CONTENTS = ('first café e\u0301\r\n', 'second 한글 東京\r\n')
REQUEST = 'Create first-界.txt and second-café.txt with the exact reviewed Unicode contents.'
ANSWER = 'WS_CHAIN_COMPLETE_界'
PARTIAL = 'WS_PARTIAL_UNCONFIRMED_界'
FUTURE = 'Future界e\u0301-draft-end'
FAILURES = {
    'cancel': 'Cancelled',
    'permission-close': 'Failed',  # Tool-call response completed; retirement preceded approval/effect.
    'stale': 'Unknown',
    'error': 'Failed',
    'unattributed-error': 'Unknown',
    'overflow': 'Unknown',
    'disconnect': 'Unknown',
    'unauthorized': 'Failed',
    'redirect': 'Failed',
}


class WebSocket:
    """Bounded RFC6455 peer; one socket reader, including control frames."""
    def __init__(self, connection, initial=b''):
        self.socket = connection
        self.pending = bytearray(initial)
        self.fragment = bytearray()
        self.fragment_opcode = None
        self.closed = False
        self.pongs = []
        connection.settimeout(.05)

    def frame(self, opcode, payload=b'', final=True):
        size = len(payload)
        prefix = bytes([(0x80 if final else 0) | opcode])
        prefix += (bytes([size]) if size < 126 else
                   b'\x7e' + struct.pack('!H', size) if size <= 65535 else
                   b'\x7f' + struct.pack('!Q', size))
        self.socket.sendall(prefix + payload)

    def event(self, packet, *, fragmented=True):
        raw = json.dumps(packet, ensure_ascii=False, separators=(',', ':')).encode()
        if not fragmented or len(raw) < 4:
            self.frame(1, raw)
            return
        # Split within a multi-byte scalar when available, and interleave a
        # control frame: a consumer must assemble before UTF-8/JSON admission.
        split = next((i + 1 for i, byte in enumerate(raw[:-1]) if byte >= 0xc0), len(raw) // 2)
        self.frame(1, raw[:split], False)
        self.frame(9, b'fragment-control')
        self.frame(0, raw[split:])

    def receive(self):
        # Parse only complete frames. A timeout never discards a partial header,
        # mask or payload. This is also what permits main-thread control actions.
        while True:
            if len(self.pending) >= 2:
                first, second = self.pending[:2]
                assert first & 0x70 == 0, 'client set reserved WebSocket bits'
                assert second & 0x80, 'client frame must be masked'
                size, offset = second & 127, 2
                extra = 2 if size == 126 else 8 if size == 127 else 0
                if len(self.pending) >= offset + extra:
                    if extra:
                        size = int.from_bytes(self.pending[offset:offset + extra], 'big')
                        offset += extra
                    assert size <= FRAME_LIMIT, 'unbounded client frame'
                    if len(self.pending) >= offset + 4 + size:
                        mask = self.pending[offset:offset + 4]
                        data = self.pending[offset + 4:offset + 4 + size]
                        del self.pending[:offset + 4 + size]
                        data = bytes(byte ^ mask[index % 4] for index, byte in enumerate(data))
                        opcode, final = first & 15, bool(first & 128)
                        if opcode >= 8:
                            assert final and size <= 125, 'invalid client control frame'
                            if opcode == 8:
                                self.closed = True
                                try:
                                    self.frame(8, data)
                                except OSError:
                                    pass
                                return None
                            if opcode == 9:
                                self.frame(10, data)
                            elif opcode == 10:
                                self.pongs.append(data.decode('ascii'))
                            else:
                                raise AssertionError('unknown client control opcode')
                            continue
                        assert opcode in (0, 1), 'client sent non-text message'
                        if opcode:
                            assert self.fragment_opcode is None, 'interleaved client messages'
                            self.fragment_opcode = opcode
                        else:
                            assert self.fragment_opcode == 1, 'orphan continuation frame'
                        assert len(self.fragment) + len(data) <= FRAME_LIMIT, 'unbounded client message'
                        self.fragment.extend(data)
                        if final:
                            packet = bytes(self.fragment)
                            self.fragment.clear()
                            self.fragment_opcode = None
                            return packet
                        continue
            try:
                chunk = self.socket.recv(65536)
            except socket.timeout:
                raise
            except (ConnectionResetError, ConnectionAbortedError):
                self.closed = True
                return None
            if not chunk:
                self.closed = True
                return None
            self.pending.extend(chunk)


class WebsocketPeer(Peer):
    """Metadata and upgrades share the exact configured origin/path."""
    def __init__(self):
        super().__init__('responses', 'sk-fixture-websocket-only-7319', False)
        self.scenarios = queue.Queue()
        self.connections = []
        inherited = self.server.RequestHandlerClass
        fixture = self

        class Handler(inherited):
            def do_GET(self):
                if self.path == '/v1/models':
                    return super().do_GET()
                row = self.record(None)
                if self.path != '/v1/responses':
                    self.reply(404, {'error': 'credential or redirect trap reached'})
                    return
                self.close_connection = True
                try:
                    scenario = fixture.scenarios.get_nowait()
                except queue.Empty:
                    fixture.errors.append('unplanned upgrade: reconnect/retry/replay')
                    self.reply(409, {'error': 'no planned connection'})
                    return
                state = {'scenario': scenario, 'upgrade': row, 'frames': [], 'closed': False,
                         'actions': queue.Queue(), 'pongs': [], 'sequence': 0}
                with fixture.lock:
                    fixture.connections.append(state)
                try:
                    assert row['authentication_matches_selected_source'], 'upgrade credential scope mismatch'
                    assert self.headers.get('Upgrade', '').lower() == 'websocket'
                    assert 'upgrade' in self.headers.get('Connection', '').lower()
                    assert self.headers.get('Sec-WebSocket-Version') == '13'
                    if scenario['mode'] in ('unauthorized', 'redirect'):
                        if scenario['mode'] == 'redirect':
                            self.send_response(307)
                            self.send_header('Location', fixture.base + '/credential-trap')
                            self.send_header('Content-Length', '0')
                            self.end_headers()
                        else:
                            self.reply(401, {'error': 'synthetic unauthorized'})
                        return
                    key = self.headers['Sec-WebSocket-Key']
                    assert len(base64.b64decode(key, validate=True)) == 16
                    accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
                    self.send_response(101)
                    self.send_header('Upgrade', 'websocket')
                    self.send_header('Connection', 'Upgrade')
                    self.send_header('Sec-WebSocket-Accept', accept)
                    self.end_headers()
                    self.wfile.flush()
                    wire = WebSocket(self.connection)
                    state['pongs'] = wire.pongs
                    deadline = time.monotonic() + 90
                    while not fixture.stopped.is_set() and time.monotonic() < deadline:
                        try:
                            action = state['actions'].get_nowait()
                        except queue.Empty:
                            action = None
                        if action == 'ping':
                            wire.frame(9, b'permission-wait')
                        elif action == 'close':
                            wire.frame(8, struct.pack('!H', 1000) + b'fixture close')
                        try:
                            raw = wire.receive()
                        except socket.timeout:
                            continue
                        if raw is None:
                            return
                        packet = json.loads(raw)
                        state['frames'].append({'body': packet, 'raw_hex': raw.hex()})
                        fixture.respond(wire, state)
                    if not fixture.stopped.is_set():
                        raise AssertionError('WebSocket connection did not retire before deadline')
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    # Expected only after a deliberate failure/oversize response.
                    if scenario['mode'] not in ('overflow', 'stale', 'error', 'disconnect'):
                        fixture.errors.append('unexpected WebSocket socket failure')
                except Exception:
                    fixture.errors.append(traceback.format_exc())
                finally:
                    state['closed'] = True

            def do_POST(self):
                # Observe any attempted HTTP fallback without offering SSE.
                size = int(self.headers.get('Content-Length', '0'))
                self.record(json.loads(self.rfile.read(size)) if 0 < size <= FRAME_LIMIT else None)
                fixture.errors.append('HTTP inference fallback attempted')
                self.reply(409, {'error': 'HTTP fallback forbidden'})

        self.server.RequestHandlerClass = Handler

    def emit(self, wire, state, kind, **fields):
        sequence = state['sequence']
        state['sequence'] += 1
        wire.event({'type': kind, 'sequence_number': sequence, **fields})

    def respond(self, wire, state):
        scenario = state['scenario']
        number = len(state['frames'])
        mode = scenario['mode']
        assert number <= (3 if mode == 'chain' else 1), 'extra response.create/replay'
        body = state['frames'][-1]['body']
        assert body['type'] == 'response.create' and body['store'] is False
        assert 'response' not in body and 'stream' not in body and 'background' not in body
        assert body['model'] == MODEL
        state['sequence'] = 0
        identity = scenario['id'] + '-' + str(number)
        if mode == 'disconnect':
            self.connection_shutdown(wire)
            return
        if mode == 'unattributed-error':
            self.emit(wire, state, 'error', error={
                'type': 'server_error', 'code': 'unattributed', 'message': 'No response receipt'})
            return
        self.emit(wire, state, 'response.created', response={'id': identity, 'status': 'in_progress', 'output': []})
        if mode == 'stale':
            self.emit(wire, state, 'response.output_text.delta', response_id='stale-foreign-id', delta=PARTIAL)
            return
        if mode == 'error':
            self.emit(wire, state, 'error', response_id=identity,
                      error={'type': 'server_error', 'code': 'fixture_error', 'message': 'Controlled WS failure'})
            return
        if mode == 'overflow':
            # Exceed the assembled limit across individually small fragments;
            # the peer never constructs a huge parsed JSON object.
            wire.socket.settimeout(2)
            wire.frame(1, b'{"type":"response.output_text.delta","delta":"', False)
            block = b'x' * 65536
            for _ in range(FRAME_LIMIT // len(block)):
                wire.frame(0, block, False)
            wire.frame(0, b'"}')
            wire.socket.settimeout(.05)
            return
        output = []
        call = None
        if mode == 'chain' and number <= 2:
            call = {'type': 'function_call', 'id': identity + '-item',
                    'call_id': identity + '-call', 'name': 'eevee_write_file',
                    'arguments': json.dumps({'path': (FIRST, SECOND)[number - 1],
                                              'content': CONTENTS[number - 1]}, ensure_ascii=False)}
        elif mode in ('cancel', 'permission-close'):
            call = {'type': 'function_call', 'id': identity + '-item',
                    'call_id': identity + '-call', 'name': 'eevee_write_file',
                    'arguments': json.dumps({'path': scenario['path'], 'content': 'must never apply\n'})}
        if call:
            self.emit(wire, state, 'response.output_item.added', response_id=identity, output_index=0,
                      item=call | {'arguments': ''})
            self.emit(wire, state, 'response.function_call_arguments.delta', response_id=identity,
                      item_id=call['id'], output_index=0, delta=call['arguments'])
            self.emit(wire, state, 'response.function_call_arguments.done', response_id=identity,
                      item_id=call['id'], output_index=0, arguments=call['arguments'])
            self.emit(wire, state, 'response.output_item.done', response_id=identity,
                      output_index=0, item=call | {'status': 'completed'})
            output.append(call | {'status': 'completed'})
        else:
            text = ANSWER if mode == 'chain' else scenario['answer']
            item = {'id': identity + '-message', 'type': 'message', 'role': 'assistant',
                    'status': 'completed', 'content': [{'type': 'output_text', 'text': text}]}
            self.emit(wire, state, 'response.output_item.added', response_id=identity,
                      output_index=0, item=item | {'content': []})
            self.emit(wire, state, 'response.output_text.delta', response_id=identity,
                      item_id=item['id'], output_index=0, content_index=0, delta=text)
            self.emit(wire, state, 'response.output_item.done', response_id=identity, output_index=0, item=item)
            output.append(item)
        self.emit(wire, state, 'response.completed', response={'id': identity, 'status': 'completed',
                  'model': MODEL, 'output': output, 'usage': {'input_tokens': 100, 'output_tokens': 10}})

    @staticmethod
    def connection_shutdown(wire):
        wire.socket.shutdown(socket.SHUT_RDWR)

    def frames(self):
        with self.lock:
            return [frame for state in self.connections for frame in state['frames']]

    def evidence(self):
        with self.lock:
            return [{key: value for key, value in state.items() if key != 'actions'}
                    for state in self.connections]


class WebsocketSession(SurfaceSession):
    def __init__(self, argv, cwd, environment, *args, **kwargs):
        # Production deliberately refuses unsupported proxy/custom-TLS policy.
        # Isolation must not accidentally turn this wire test into that refusal.
        environment = {key: value for key, value in environment.items()
                       if not key.lower().endswith('_proxy')
                       and key not in ('SSL_CERT_FILE', 'SSL_CERT_DIR')}
        super().__init__([*argv, '--native-transport', 'websocket'], cwd, environment, *args, **kwargs)


class WebsocketJourney(CreationJourney):
    def __init__(self, binary, output, root, size, peer):
        with patch.object(response_smoke, 'SurfaceSession', WebsocketSession):
            super().__init__(binary, output, root, size, False, 'compatible', 'responses', False, False, peer)

    def receipts_exact(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM outgoing_receipts ORDER BY rowid')]

    def begin_ws(self, scenario, request, *, preview=False):
        self.write_request(request)
        count = len(self.peer.connections)
        self.peer.scenarios.put(scenario)
        if preview:
            self.session.command('session-new')
            focused = False

            def prepared_review():
                nonlocal focused
                view = self.session.display()
                if 'd Details' in view:
                    return True
                if not focused and 'Alt+A Review' in view:
                    focused = True
                    self.session.send(ESC + 'a')
                return False

            until(self.session, prepared_review, 'prepared request review is not reachable', timeout=40)
            assert len(self.peer.connections) == count, 'local preparation connected before consent'
            self.session.send('d')
            self.preview_rows = self.inspect_packet()
            assert len(self.peer.connections) == count, 'payload inspection dispatched'
            self.capture('frozen-initial-preview-before-dispatch')
            self.session.send('d')
        else:
            self.session.send('\r', settle=False)
        approved = False

        def submitted():
            nonlocal approved
            if len(self.peer.connections) == count + 1:
                return True
            if not approved and permission_run_available(self.session):
                self.session.send('y', settle=False)
                approved = True
            elif not approved and inspect_permission_scope(self.session):
                pass
            elif 'Alt+A' in self.session.display():
                self.session.send(ESC + 'a')
            return False

        until(self.session, submitted, 'exact explicit WS connection did not start', timeout=40)
        state = self.peer.connections[-1]
        if scenario['mode'] not in ('unauthorized', 'redirect'):
            until(self.session, lambda: bool(state['frames']), 'initial response.create not received')
        return state

    def inspect_packet(self):
        # Read the actual rendered details viewport one line at a time. This
        # compares the consumer-visible frozen packet, not source-code strings.
        screen = self.session.screen
        panels = []
        for top in range(screen.lines - 1):
            for left in range(screen.columns):
                cell = screen.buffer[top][left].data
                closing = {'╔': ('╗', '╚', '╝'), '┌': ('┐', '└', '┘'),
                           '╭': ('╮', '╰', '╯')}.get(cell)
                if closing is None:
                    continue
                right = next((x for x in range(left + 1, screen.columns)
                              if screen.buffer[top][x].data == closing[0]), None)
                if right is None:
                    continue
                bottom = next((y for y in range(top + 1, screen.lines)
                               if screen.buffer[y][left].data == closing[1]
                               and screen.buffer[y][right].data == closing[2]), None)
                if bottom is not None:
                    panels.append(((right - left) * (bottom - top), top, left, right, bottom))
        assert panels, 'focused preparation sheet border not visible'
        _, top, left, right, bottom = max(panels)
        first, height = top + 2, bottom - top - 7
        assert height > 0, 'details viewport unavailable'
        collected, previous = [], None
        for _ in range(12000):
            screen = self.session.screen
            current = [''.join(screen.buffer[y][x].data for x in range(left + 1, right)).rstrip()
                       for y in range(first, first + height)]
            if previous is None:
                collected.extend(current)
            elif current != previous:
                overlap = next((n for n in range(len(current), 0, -1) if collected[-n:] == current[:n]), 0)
                assert overlap, 'detail scrolling skipped visible payload rows'
                collected.extend(current[overlap:])
            if any('EXACT OUTGOING PAYLOAD END' in line for line in collected):
                start = next(i for i, line in enumerate(collected) if 'EXACT OUTGOING PAYLOAD BEGIN' in line)
                end = next(i for i, line in enumerate(collected) if 'EXACT OUTGOING PAYLOAD END' in line)
                assert end > start + 1
                save(self.output / 'frozen-preview-visible-rows.json', collected)
                return ''.join(''.join(collected[start + 1:end]).split())
            assert previous != current, 'details stopped before the complete outgoing packet'
            previous = current
            self.session.send(DOWN)
        raise AssertionError('frozen payload inspection exceeded bounded rows')

    def approval(self, path):
        until(self.session, lambda: file_review_pending(self.session), 'fresh exact effect approval not presented', timeout=40)
        assert not (self.project / path).exists(), 'effect happened before fresh approval'
        count = len(self.peer.frames())
        self.capture('neutral-fresh-approval-' + path)
        self.session.send('\r')
        inspect_to_bottom(self.session)
        self.session.send('d')
        until(self.session, lambda: file_review_pending(self.session) and normalized(path) in normalized(self.session.display()),
              'complete exact effect scope not visible')
        assert len(self.peer.frames()) == count and not (self.project / path).exists(), 'inspection granted effect'
        self.session.send(RIGHT + '\r', settle=False)

    def settled_ws(self, state, answer=None):
        def settled():
            view = self.session.display()
            if 'Evolution!' in view:
                self.session.send(ESC)
                return False
            return not self.snapshot()['claims'] and 'Storage: saved' in view and state['closed'] and (answer is None or answer in view)
        until(self.session, settled, 'WS logical request did not settle/close', timeout=40)
        self.workspace()

    def outcome(self, expected, label):
        self.session.command('response')
        self.session.send(ESC + '[F')
        until(self.session, lambda: any(line.strip().startswith(expected + ' ·')
              for line in self.session.display().splitlines()),
              'actual selected response outcome is not ' + expected + ': ' + label)
        self.capture('request-outcome-' + label + '-' + expected.lower())
        self.workspace()

    def assert_no_replay(self, connections, frames):
        self.session.pump(.4)
        assert len(self.peer.connections) == connections and len(self.peer.frames()) == frames, 'hidden retry/reconnect/replay'
        assert not self.peer.snapshot('POST'), 'HTTP fallback occurred'
        assert not self.peer.errors, self.peer.errors
        assert all(row['path'] in ('/v1/models', '/v1/responses') for row in self.peer.snapshot()), 'redirect or credential scope escaped'

    def run_failure(self, mode, receipt):
        expected_outcome = FAILURES[mode]
        scenario = {'mode': mode, 'id': mode, 'path': mode + '-must-not-exist.txt'}
        before_connections, before_frames = len(self.peer.connections), len(self.peer.frames())
        receipts_before = self.receipts_exact()
        state = self.begin_ws(scenario, 'Create ' + scenario['path'] + ' only after review; case ' + mode + ' 界.')
        if mode in ('cancel', 'permission-close'):
            until(self.session, lambda: file_review_pending(self.session), 'held effect approval missing', timeout=40)
            state['actions'].put('ping')
            until(self.session, lambda: 'permission-wait' in state['pongs'], 'control ping stalled behind permission wait')
            if mode == 'cancel':
                # A neutral permission sheet owns its keys. Defer without
                # answering, then cancel the still-owned request from home.
                frames = len(self.peer.frames())
                self.session.send(ESC)
                until(self.session, lambda: 'Request deferred' in self.session.display()
                      and not file_review_pending(self.session), 'permission was not neutrally deferred')
                assert len(self.peer.frames()) == frames and not (self.project / scenario['path']).exists(), 'deferral answered or executed permission'
                self.session.command('cancel')
            else:
                state['actions'].put('close')
        self.settled_ws(state)
        self.outcome(expected_outcome, mode)
        assert not (self.project / scenario['path']).exists(), 'cancel/failure applied an unapproved effect'
        self.assert_no_replay(before_connections + 1, before_frames + (0 if mode in ('unauthorized', 'redirect') else 1))
        assert self.receipts_exact()[0] == receipt, 'later outcome rewrote original frozen packet/receipt'
        if mode in ('unauthorized', 'redirect'):
            assert self.receipts_exact() == receipts_before, 'upgrade rejection falsely recorded a model dispatch'
        self.capture('no-retry-fallback-replay-' + mode)
        return mode + ': actual ' + expected_outcome + ' consumer; no unapproved effect/reconnect/HTTP fallback/replay'

    def run_hazard(self, mode):
        # Concrete transport/auth failures deliberately block this connection
        # owner. Each independent hazard gets a fresh disposable application;
        # never clear its diagnostic or sticky uncertainty to reach another case.
        wait_for(self.session, 'Storage: saved')
        self.setup_route()
        self.session.command('role-implement')
        self.workspace()
        answer = 'WS_BASELINE_' + mode + '_界'
        baseline = self.begin_ws({'mode': 'complete', 'id': 'baseline-' + mode, 'answer': answer},
                                 'Reply with the baseline marker only.')
        self.settled_ws(baseline, answer)
        until(self.session, lambda: len(self.receipts_exact()) == 1, 'hazard baseline receipt not committed')
        receipt = self.receipts_exact()[0]
        raw = bytes.fromhex(baseline['frames'][0]['raw_hex'])
        record = json.loads(receipt['record'])
        assert receipt['source'].encode() == raw
        assert record['bytes'] == len(raw) and bytes(record['sha256']) == hashlib.sha256(raw).digest()
        assert sum(section['bytes'] for section in record['sections']) + record['envelope_bytes'] == len(raw)
        case = self.run_failure(mode, receipt)
        self.write_request('Unsent future draft after ' + mode + ' 界.')
        self.capture('retained-failure-barrier-and-unsent-future-' + mode)
        self.assert_no_replay(2, 1 + (0 if mode in ('unauthorized', 'redirect') else 1))
        assert self.snapshot()['executions'] == 0, 'failure invented command execution'
        self.session.send('\x03')
        confirm_exit(self.session, owned_work=True)
        assert self.session.wait_exit() == 0
        return ['fresh disposable owner with exact successful baseline receipt; no remediation or uncertainty reset', case]

    def run_preflight(self):
        # Advanced saved-fact preparation is a separate disposable application,
        # never a substitute for ordinary Request continuity in run_ws().
        wait_for(self.session, 'Storage: saved')
        self.setup_route()
        goal = 'Neutral local fact for exact WebSocket preview qualification.'
        request = 'Reply with the neutral preview marker only; do not use tools.'
        answer = 'WS_ADVANCED_PREFLIGHT_界'
        self.session.command('goal')
        self.session.paste(goal)
        self.session.send('\r')
        until(self.session, lambda: any(task['goal'] == goal for task in self.snapshot()['tasks'])
              and 'Storage: saved' in self.session.display(), 'advanced preflight goal not committed')
        self.session.command('role-plan')
        self.workspace()
        state = self.begin_ws({'mode': 'complete', 'id': 'advanced-preflight', 'answer': answer},
                              request, preview=True)
        initial = state['frames'][0]
        raw = bytes.fromhex(initial['raw_hex'])
        assert ''.join(raw.decode().split()) == self.preview_rows, 'initial wire differs from frozen visible preparation packet'
        assert [(row['role'], content_text(row['content'])) for row in initial['body']['input']] == [('user', request)]
        assert 'previous_response_id' not in initial['body']
        self.settled_ws(state, answer)
        until(self.session, lambda: len(self.receipts_exact()) == 1, 'advanced initial receipt not retained')
        receipt = self.receipts_exact()[0]
        record = json.loads(receipt['record'])
        assert receipt['source'].encode() == raw, 'advanced receipt differs from its frozen initial frame'
        assert record['bytes'] == len(raw) and bytes(record['sha256']) == hashlib.sha256(raw).digest()
        assert sum(section['bytes'] for section in record['sections']) + record['envelope_bytes'] == len(raw)
        self.assert_no_replay(1, 1)
        self.capture('advanced-neutral-frozen-preview-wire-receipt-equality')
        self.session.send('\x03')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        return ['isolated neutral saved-goal advanced preview: no pre-consent connection; visible frozen packet equals actual frame; exact durable receipt/hash/sections']

    def run_ws(self):
        wait_for(self.session, 'Storage: saved')
        self.setup_route()
        self.session.command('role-implement')
        self.workspace()
        assert not self.peer.connections, 'setup opened inference WebSocket'
        # Enter is the ordinary Request path. It owns goal initialization and
        # preserves completed public context; /session-new is not used here.
        state = self.begin_ws({'mode': 'chain', 'id': 'chain'}, REQUEST)
        initial = state['frames'][0]
        assert 'previous_response_id' not in initial['body']
        assert [(row['role'], content_text(row['content'])) for row in initial['body']['input']] == [('user', REQUEST)]
        self.approval(FIRST)
        until(self.session, lambda: len(state['frames']) == 2, 'first approved result did not continue on the same connection')
        assert (self.project / FIRST).read_bytes() == CONTENTS[0].encode()
        assert not (self.project / SECOND).exists(), 'first consent authorized second effect'
        until(self.session, lambda: file_review_pending(self.session), 'second fresh approval absent')
        state['actions'].put('ping')
        until(self.session, lambda: 'permission-wait' in state['pongs'], 'ping not serviced during actual permission wait')
        self.approval(SECOND)
        self.settled_ws(state, ANSWER)
        assert (self.project / SECOND).read_bytes() == CONTENTS[1].encode()
        assert len(self.peer.connections) == 1 and len(state['frames']) == 3
        for index, frame in enumerate(state['frames'][1:], 1):
            packet = frame['body']
            assert packet['previous_response_id'] == 'chain-' + str(index), 'wrong completed response anchor'
            assert len(packet['input']) == 1, 'continuation replayed full input or prior tool output'
            result = packet['input'][0]
            assert result['type'] == 'function_call_output' and result['call_id'] == 'chain-' + str(index) + '-call'
            output = json.loads(result['output']) if isinstance(result['output'], str) else result['output']
            assert output['status'] == 'success' and output['effect'] == 'applied', 'continuation did not carry actual acknowledged effect success'
        until(self.session, lambda: len(self.receipts_exact()) == 1, 'one logical outgoing receipt not committed')
        receipt = self.receipts_exact()[0]
        raw = bytes.fromhex(initial['raw_hex'])
        record = json.loads(receipt['record'])
        assert receipt['source'].encode() == raw, 'durable receipt replaced frozen initial frame with a continuation'
        assert record['bytes'] == len(raw) and bytes(record['sha256']) == hashlib.sha256(raw).digest()
        assert sum(section['bytes'] for section in record['sections']) + record['envelope_bytes'] == len(raw)
        self.capture('completed-two-approved-effects-one-handshake')
        self.outcome('Completed', 'chain')
        self.assert_no_replay(1, 3)
        cases = ['one upgrade; two exact-ID output-only continuations; fresh approvals and literal Unicode/CRLF effects',
                 'ordinary Request initial frame has only the authored user input; exact frozen durable receipt/hash/sections survive continuations',
                 'fragmented UTF-8/JSON and interleaved controls; ping while permission waits']
        # Cancellation has no connection-failure diagnostic, so this same owner
        # can prove failed-request exclusion on a later ordinary explicit Send.
        # Independent transport/auth/protocol hazards run in run_hazard instead.
        cases.append(self.run_failure('cancel', receipt))
        # A later deliberate Send must create a new connection with only bounded
        # completed public context, never failed requests, tool items or anchors.
        state = self.begin_ws({'mode': 'complete', 'id': 'fresh', 'answer': 'WS_FRESH_PUBLIC_界'},
                              'Continue.')
        packet = state['frames'][0]['body']
        assert 'previous_response_id' not in packet
        assert all(row.get('role') in ('user', 'assistant') and 'call_id' not in row for row in packet['input'])
        history = json.dumps(packet['input'], ensure_ascii=False)
        assert ANSWER in history and FIRST in history
        assert all(mode + '-must-not-exist.txt' not in history for mode in FAILURES)
        self.settled_ws(state, 'WS_FRESH_PUBLIC_界')
        self.write_request(FUTURE)
        self.session.send(LEFT * 5 + '\x13')
        def drafts():
            return [draft for task in self.snapshot()['tasks'] for partner in task['partner_drafts'].values()
                    for draft in partner.get('drafts', []) if draft['field'] == 'Request']
        cursor = len(FUTURE.encode()) - 5
        until(self.session, lambda: any(d['text'] == FUTURE and d['cursor'] == cursor for d in drafts()), 'future Unicode draft/caret not acknowledged')
        before = drafts()
        stable_connections, stable_frames = len(self.peer.connections), len(self.peer.frames())
        self.session.command('receipts')
        wait_for(self.session, 'Outgoing receipts')
        self.session.send(ESC)
        self.workspace()
        self.session.send('\x13')
        until(self.session, lambda: drafts() == before and 'Storage: saved' in self.session.display(), 'inspection altered future draft/caret')
        self.session.paste('Z')
        self.session.send('\x13')
        until(self.session, lambda: any(d['text'] == FUTURE[:-5] + 'Z' + FUTURE[-5:] for d in drafts()), 'actual editor did not retain middle caret')
        self.capture('future-unicode-draft-middle-caret-preserved')
        self.assert_no_replay(stable_connections, stable_frames)
        cases.append('new explicit Send opens fresh connection with completed-public-only context; future Unicode draft/caret preserved')
        assert self.snapshot()['executions'] == 0, 'file tools invented command execution'
        self.session.send('\x03')
        confirm_exit(self.session, owned_work=True)
        assert self.session.wait_exit() == 0
        return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = {'status': 'failed', 'binary': str(binary), 'sha256': digest, 'journeys': [],
              'limits': ['Deterministic loopback/TUI proof, not live provider entitlement, billing, caching or model understanding.',
                         'Transport overflow tests assembled fragmented-message bounds, not every cumulative/queue quota.',
                         'No TLS/proxy/live-account conformance or physical GUI certification is claimed.']}
    modes = ('ordinary-retention', 'advanced-preflight', *(mode for mode in FAILURES if mode != 'cancel'))
    for size, mode in ((size, mode) for size in ((120, 36), (60, 18)) for mode in modes):
        directory = output / ('%dx%d-unicode-%s' % (*size, mode))
        directory.mkdir()
        result = {'status': 'failed', 'size': list(size), 'unicode': True, 'mode': mode}
        peer, journey = None, None
        try:
            with tempfile.TemporaryDirectory(prefix='ev-websocket-界-') as temporary:
                with WebsocketPeer() as peer:
                    try:
                        journey = WebsocketJourney(binary, directory, Path(temporary).resolve(), size, peer)
                        if mode == 'advanced-preflight':
                            result['cases'] = journey.run_preflight()
                        elif mode == 'ordinary-retention':
                            result['cases'] = journey.run_ws()
                        else:
                            result['cases'] = journey.run_hazard(mode)
                        result['status'] = 'passed'
                    finally:
                        if journey:
                            journey.close()
                        save(directory / 'wire.json', peer.evidence())
                        save(directory / 'http.json', peer.snapshot())
                        assert not peer.errors, peer.errors
        except Exception as error:
            result.update(status='failed', error=str(error), traceback=traceback.format_exc())
        save(directory / 'result.json', result)
        report['journeys'].append(result)
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'supplied binary changed'
    report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
