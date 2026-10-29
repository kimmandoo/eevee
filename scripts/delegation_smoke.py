#!/usr/bin/env python3
"""XC02 real-binary read-only delegation through PTY and loopback Chat Completions.

Run against an already-built executable:
  python3 scripts/delegation_smoke.py --binary /absolute/path/to/ev \
      --output /absolute/path/to/new-evidence-directory

Uses the existing pyte/wcwidth PTY helpers (ConPTY on Windows), disposable homes,
project/data directories, a threaded keyless HTTP peer and read-only SQLite
queries. Fixture setup initializes a standalone Git project and exposes only
the installed Git executable for native source-policy checks. The peer never
executes tools or writes the sources under test. No build, paid/live request,
external provider program or SQL mutation is involved. Failure retains terminal
screens/events/ANSI, HTTP bodies, SQLite observations and the isolated workspace
under --output rather than deleting the evidence.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import sqlite3
import subprocess
import threading
import time
import traceback

import file_creation_smoke as files
from native_connection_smoke import MODEL, Peer, until
from response_smoke import ALT_END
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available
from workbench_smoke import wait_for

ESC = '\x1b'
DOWN, PAGE_DOWN = ESC + '[B', ESC + '[6~'
SIZES = ((120, 36), (60, 18))
DELEGATE = 'eevee_delegate'
READ_TOOLS = {'eevee_list_files', 'eevee_read_file', 'eevee_structural_search'}
EFFECT_TOOLS = {'eevee_write_file', 'eevee_edit_file', 'eevee_delete_file',
                'eevee_rename_file', 'eevee_run_command', DELEGATE, 'todo',
                'eevee_artifact_list', 'eevee_artifact_read', 'eevee_artifact_search',
                'eevee_artifact_locations'}
SOURCES = {
    'alpha.txt': 'ALPHA_ACTUAL_SOURCE_43f1\r\nvalue = "café e\u0301"\r\n',
    'beta.txt': 'BETA_ACTUAL_SOURCE_75c2\nvalue = "한글界"\n',
    'gamma.txt': 'GAMMA_ACTUAL_SOURCE_98d3\nvalue = "東京"\n',
    'unchanged.txt': 'PUBLIC_UNRELATED_DIRTY_SOURCE_6d14\r\n',
}
WRITE_BYTES = 'FORBIDDEN_CHILD_WRITE_MUST_NEVER_REACH_DISK\n'
LIMITS = [
    'Deterministic compatible Chat Completions only; no paid/live model, provider entitlement, billing or model-understanding claim.',
    'The controlled peer proves application tool/ownership/cancellation boundaries, not an OS sandbox or every provider dialect.',
    'PTY/ConPTY screen captures are not physical GUI, mouse, image or live-platform certification.',
    'This journey does not exercise user-partner capacity contention or the 64-round/120-second/16-KiB exhaustion cases; the review discloses these limits.',
]


def text_key(text):
    """Ignore viewport wrapping only; wire, source and receipt bytes stay exact."""
    return re.sub(r'\s+', '', files.normalized(text))


def permission_text(session):
    """Read only the actual permission panel, not the workspace behind it.

    Like native_websocket_smoke's packet inspector, locate the rendered box
    from cells; no fixed terminal coordinates or hidden application state.
    """
    screen = session.screen
    corners = {'┌': ('┐', '└', '┘'), '╔': ('╗', '╚', '╝'),
               '╭': ('╮', '╰', '╯'), '+': ('+', '+', '+')}
    for top in range(screen.lines):
        for left in range(screen.columns):
            closing = corners.get(screen.buffer[top][left].data)
            if closing is None:
                continue
            right = next((x for x in range(left + 1, screen.columns)
                          if screen.buffer[top][x].data == closing[0]), None)
            if right is None:
                continue
            title = ''.join(screen.buffer[top][x].data for x in range(left + 1, right))
            if 'Permission required' not in title:
                continue
            bottom = next((y for y in range(top + 1, screen.lines)
                           if screen.buffer[y][left].data == closing[1]
                           and screen.buffer[y][right].data == closing[2]), None)
            if bottom is not None:
                return '\n'.join(''.join(screen.buffer[y][x].data
                                        for x in range(left + 1, right))
                                 for y in range(top + 1, bottom))
    raise AssertionError('the exact delegation permission panel was not rendered')


def tools(body):
    return {row['function']['name'] for row in body.get('tools', [])}


def results(body):
    return [row for row in body['messages'] if row.get('role') == 'tool']


def usage_totals(record):
    return tuple(sum(measurement['attributable'].get(field) or 0
                     for measurement in record['measurements'])
                 for field in ('input', 'output'))


def project_bytes(project):
    return {str(path.relative_to(project)): path.read_bytes().hex()
            for path in sorted(project.rglob('*')) if path.is_file()}


class Round:
    """One lead round with independent per-child HTTP gates, never fake workers."""
    def __init__(self, mode):
        self.mode = mode
        self.calls = [
            {'id': f'{mode}-delegate-{index}', 'name': DELEGATE, 'arguments': {
                'role': 'review' if index == 1 else 'explore',
                'instruction': f'XC02_{mode.upper()}_CHILD_{index}: inspect {path} independently and report its literal marker.',
                'paths': [path],
            }}
            for index, path in enumerate(tuple(SOURCES)[:3])
        ]
        self.started = [threading.Event() for _ in self.calls]
        self.read_release = threading.Event()
        self.final_ready = [threading.Event() for _ in self.calls]
        self.final_release = [threading.Event() for _ in self.calls]
        self.finished = [threading.Event() for _ in self.calls]
        self.closed = [threading.Event() for _ in self.calls]
        self.parent_ready = threading.Event()
        self.parent_release = threading.Event()
        self.parent_rounds = 0
        self.child_rounds = [0, 0, 0]
        self.completion_order = []
        self.rows = []

    def marker(self, index):
        return f'XC02_{self.mode.upper()}_CHILD_{index}'

    def summary(self, index):
        return f'XC02_{self.mode.upper()}_SUMMARY_{index}: observed {self.calls[index]["arguments"]["paths"][0]} without effects.'

    def evidence(self):
        return {'mode': self.mode, 'calls': self.calls, 'parent_rounds': self.parent_rounds,
                'child_rounds': self.child_rounds, 'completion_order': self.completion_order,
                'started': [event.is_set() for event in self.started],
                'final_ready': [event.is_set() for event in self.final_ready],
                'finished': [event.is_set() for event in self.finished],
                'closed': [event.is_set() for event in self.closed]}


class DelegationPeer(Peer):
    def __init__(self):
        super().__init__('chat_completions', 'synthetic-delegation-unused', True,
                         retain_body_bytes=True)
        self.rounds = []
        self.active = None
        inherited = self.server.RequestHandlerClass
        fixture = self

        class Handler(inherited):
            def do_POST(self):
                row = None
                try:
                    size = int(self.headers.get('Content-Length', '0'))
                    assert 0 < size <= 8 * 1024 * 1024, 'unbounded delegation HTTP request'
                    raw = self.rfile.read(size)
                    row = self.record(json.loads(raw))
                    row['body_bytes_hex'] = raw.hex()
                    row['arrived_ns'] = time.monotonic_ns()
                    row['events'] = []
                    assert self.path == '/v1/chat/completions', 'unexpected inference endpoint'
                    assert row['authentication_matches_selected_source'], 'fixture request included unselected credentials'
                    assert row['body']['model'] == MODEL and row['body']['stream'] is True
                    scenario = fixture.active
                    assert scenario is not None, 'unplanned inference before an explicit Send'
                    with fixture.lock:
                        scenario.rows.append(row)
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    self.close_connection = True

                    def emit(value):
                        row['events'].append(value)
                        self.wfile.write(('data: ' + json.dumps(value, ensure_ascii=False) + '\n\n').encode())
                        self.wfile.flush()

                    fixture.respond(self, emit, row, scenario)
                except Exception:
                    with fixture.lock:
                        fixture.errors.append(traceback.format_exc())
                finally:
                    if row is not None:
                        row['handler_retired_ns'] = time.monotonic_ns()

        self.server.RequestHandlerClass = Handler

    def begin(self, mode):
        scenario = Round(mode)
        self.rounds.append(scenario)
        self.active = scenario
        return scenario

    def gate(self, event, label):
        deadline = time.monotonic() + 75
        while not event.wait(.03):
            if self.stopped.is_set():
                return False
            assert time.monotonic() < deadline, 'fixture gate was not released: ' + label
        return True

    def reply(self, handler, emit, row, *, identity, calls=(), text='', usage):
        def packet(delta, finish=None):
            emit({'id': identity, 'model': MODEL, 'choices': [
                {'index': 0, 'delta': delta, 'finish_reason': finish}]})
        if text:
            packet({'content': text})
        for index, call in enumerate(calls):
            packet({'tool_calls': [{'index': index, 'id': call['id'], 'type': 'function',
                    'function': {'name': call['name'], 'arguments': ''}}]})
            for fragment in files.pieces(json.dumps(call['arguments'], ensure_ascii=False)):
                packet({'tool_calls': [{'index': index, 'function': {'arguments': fragment}}]})
        packet({}, 'tool_calls' if calls else 'stop')
        counters = {'prompt_tokens': usage[0], 'completion_tokens': usage[1],
                    'total_tokens': sum(usage)}
        # Two identical provider snapshots must still produce one attributable
        # measurement, not two billed/aggregate contributions.
        for _ in range(2):
            emit({'id': identity, 'model': MODEL, 'choices': [], 'usage': counters})
        handler.wfile.write(b'data: [DONE]\n\n')
        handler.wfile.flush()
        row['completed_ns'] = time.monotonic_ns()
        row['reported_usage'] = counters

    def respond(self, handler, emit, row, scenario):
        body = row['body']
        if DELEGATE in tools(body):
            with self.lock:
                number = scenario.parent_rounds
                scenario.parent_rounds += 1
            row.update(kind='parent', round=number, scenario=scenario.mode)
            assert number < 2, 'lead replayed or issued an unplanned inference'
            if number == 0:
                assert not results(body), 'fresh lead request inherited old tool results'
                self.reply(handler, emit, row, identity=scenario.mode + '-parent-0',
                           calls=scenario.calls, usage=(10, 1))
                return
            assert scenario.mode == 'success', 'cancelled lead continued inference'
            returned = results(body)
            assert [item['tool_call_id'] for item in returned] == [call['id'] for call in scenario.calls], \
                'lead received delegated results in completion order instead of call order'
            row['delegation_results'] = [json.loads(item['content']) for item in returned]
            assert all(item.get('status') == 'success' and item.get('outcome') == 'completed'
                       for item in row['delegation_results']), 'lead received an untruthful child terminal status/outcome'
            assert len({item['child_request_id'] for item in row['delegation_results']}) == 3, 'lead child result identities were duplicated'
            for call, outcome in zip(scenario.calls, row['delegation_results']):
                assert outcome.get('call_id') == call['id'], 'delegated outcome lost its exact originating call identity'
                assert outcome.get('path') == '', 'non-filesystem delegation omitted its explicit empty target'
                assert isinstance(outcome.get('message'), str) and outcome['message'], \
                    'delegated outcome omitted the strict native result message'
            for index, item in enumerate(returned):
                encoded = item['content'].encode()
                assert len(encoded) <= 16 * 1024, 'delegated tool result exceeded 16 KiB'
                assert scenario.summary(index) in item['content'], 'lead lost or cross-attributed a child summary'
                assert all(scenario.summary(other) not in item['content']
                           for other in range(3) if other != index), 'child summaries were conflated'
            scenario.parent_ready.set()
            if self.gate(scenario.parent_release, 'parent final summary'):
                self.reply(handler, emit, row, identity='success-parent-1',
                           text='XC02_PARENT_ORDERED_COMPLETE', usage=(20, 2))
            return

        messages = json.dumps(body['messages'], ensure_ascii=False)
        indexes = [index for index in range(3) if scenario.marker(index) in messages]
        assert len(indexes) == 1, 'child packet inherited sibling instructions or omitted its own instruction'
        index = indexes[0]
        advertised = tools(body)
        assert READ_TOOLS <= advertised, 'child omitted the native source-read tools'
        assert not advertised & EFFECT_TOOLS, 'child advertised effects, commands, artifacts, todo or recursive delegation'
        assert advertised == READ_TOOLS, 'isolated child advertised an inherited catalog/MCP tool'
        with self.lock:
            number = scenario.child_rounds[index]
            scenario.child_rounds[index] += 1
        row.update(kind='child', child=index, round=number, scenario=scenario.mode)
        assert number < 2, 'child exceeded the planned two inference rounds'
        path = scenario.calls[index]['arguments']['paths'][0]
        if number == 0:
            assert not results(body), 'child inherited another run\'s conversation/tool results'
            assert [message['role'] for message in body['messages']] == ['system', 'user'], 'child inherited a parent conversation rather than a fresh packet'
            scenario.started[index].set()
            if scenario.mode == 'cancel':
                emit({'id': f'cancel-child-{index}', 'model': MODEL, 'choices': [
                    {'index': 0, 'delta': {'content': f'XC02_CHILD_{index}_WAITING'}, 'finish_reason': None}]})
                handler.connection.settimeout(.1)
                deadline = time.monotonic() + 35
                while not self.stopped.is_set() and time.monotonic() < deadline:
                    try:
                        if handler.connection.recv(1) == b'':
                            scenario.closed[index].set()
                            row['locally_closed_ns'] = time.monotonic_ns()
                            return
                    except socket.timeout:
                        continue
                    except (ConnectionResetError, ConnectionAbortedError):
                        scenario.closed[index].set()
                        row['locally_closed_ns'] = time.monotonic_ns()
                        return
                assert self.stopped.is_set(), 'parent cancel did not close its waiting child socket locally'
                return
            if not self.gate(scenario.read_release, 'three overlapping child reads'):
                return
            calls = [{'id': f'child-{index}-read', 'name': 'eevee_read_file', 'arguments': {
                'path': path, 'start_line': 1, 'end_line': 0}}]
            if index == 0:
                calls.append({'id': 'child-0-forbidden-write', 'name': 'eevee_write_file',
                              'arguments': {'path': path, 'content': WRITE_BYTES}})
            self.reply(handler, emit, row, identity=f'success-child-{index}-0',
                       calls=calls, usage=(30 + index * 20, 3 + index * 2))
            return
        assert scenario.mode == 'success', 'cancelled child continued inference'
        returned = {item['tool_call_id']: json.loads(item['content']) for item in results(body)}
        expected = {f'child-{index}-read'} | ({'child-0-forbidden-write'} if index == 0 else set())
        assert returned.keys() == expected, 'child received inherited/missing tool results'
        read = returned[f'child-{index}-read']
        assert read.get('status') == 'success', f'child source read was refused: {read!r}'
        assert read['content'] == SOURCES[path], f'child read content differs from actual source bytes: {path}'
        assert read['sha256'] == hashlib.sha256(SOURCES[path].encode()).hexdigest(), 'child read digest differs from source'
        if index == 0:
            refused = returned['child-0-forbidden-write']
            assert refused.get('status') not in ('success', 'completed', 'allowed'), 'child write was not refused'
            assert any(word in json.dumps(refused).lower() for word in ('refus', 'read-only', 'not allowed', 'disabled')), \
                'unadvertised child write did not return an explicit refusal result'
        scenario.final_ready[index].set()
        if self.gate(scenario.final_release[index], f'child {index} final response'):
            self.reply(handler, emit, row, identity=f'success-child-{index}-1',
                       text=scenario.summary(index), usage=(40 + index * 20, 4 + index * 2))
            with self.lock:
                scenario.completion_order.append(index)
            scenario.finished[index].set()


class DelegationJourney(files.CreationJourney):
    def state(self):
        state = self.snapshot()
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro',
                                     uri=True, timeout=10)) as db:
            db.row_factory = sqlite3.Row
            state['receipts'] = [dict(row) for row in db.execute('SELECT * FROM outgoing_receipts ORDER BY rowid')]
            state['usage'] = [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_usage ORDER BY rowid')]
        for row in state['receipts']:
            row['record'] = json.loads(row['record'])
        return state

    def wait(self, predicate, description, timeout=40):
        def checked():
            assert not self.peer.errors, '\n'.join(self.peer.errors)
            return predicate()
        until(self.session, checked, description, timeout=timeout)

    def begin_round(self, mode):
        scenario = self.peer.begin(mode)
        self.write_request('Investigate the public sources independently using three read-only helpers; report findings only. ' + mode)
        self.session.send('\r', settle=False)
        initial_approved = False
        self.review_pages = []
        self.review_scope_pages = []
        self.review_approvals = 0

        def children_started():
            nonlocal initial_approved
            if all(event.is_set() for event in scenario.started):
                return True
            if self.review_approvals:
                # The granted review can remain in the last rendered frame
                # until child startup. Never interpret that stale frame as
                # another grant; an unexpected second review times out.
                return False
            view = self.session.display()
            if 'Read-only delegation' in view or self.review_pages:
                if not self.review_pages or self.review_pages[-1] != view:
                    self.review_pages.append(view)
                    self.review_scope_pages.append(permission_text(self.session))
                    self.capture(f'{mode}-delegation-review-page-{len(self.review_pages):02}')
                if 'y Send selected' in view:
                    self.assert_review(scenario)
                    self.session.send(ESC + '[C', settle=False)
                    self.session.pump(.1)
                    assert 'Selected: Allow once' in self.session.display(), 'review did not select the exact one-shot decision'
                    self.capture(mode + '-delegation-selected-allow-once')
                    self.review_approvals += 1
                    self.session.send('y', settle=False)
                elif inspect_permission_scope(self.session):
                    pass
                elif 'Allow needs the complete visible scope' in view:
                    self.session.send(PAGE_DOWN, settle=False)
                elif 'Alt+A' in view:
                    self.session.send(ESC + 'a', settle=False)
            elif permission_run_available(self.session) and not initial_approved:
                # Some native operations retain a separate initial request review;
                # never treat that as the delegation-round grant.
                initial_approved = True
                self.capture(mode + '-initial-request-review')
                self.session.send('y', settle=False)
            elif not initial_approved and inspect_permission_scope(self.session):
                pass
            elif 'Alt+A' in view:
                self.session.send(ESC + 'a', settle=False)
            return False

        self.wait(children_started, 'three real child HTTP requests did not overlap')
        assert self.review_approvals == 1, 'three children were dispatched without one exact round review'
        files.save(self.output / (mode + '-review.json'), {'approvals': self.review_approvals,
                                                        'pages': self.review_pages,
                                                        'scope_pages': self.review_scope_pages})
        self.capture(mode + '-three-child-requests-overlapping')
        claims = self.state()['claims']
        assert claims and len({claim['request_id'] for claim in claims}) == len(claims), 'delegation lost or duplicated durable work ownership'
        initial = [row for row in self.peer.snapshot('POST')
                   if row.get('scenario') == mode and row.get('kind') == 'child' and row.get('round') == 0]
        assert len(initial) == 3 and not any('completed_ns' in row for row in initial), 'children ran serially instead of concurrently'
        assert len({row['child'] for row in initial}) == 3
        return scenario

    def assert_review(self, scenario):
        scope = text_key('\n'.join(self.review_scope_pages))
        for wanted in ('3 planned children', '64 model rounds', '120 s wall time',
                       '16 MiB cumulative request and stream bytes each',
                       '16 KiB result', 'no commands, writes or recursive delegation',
                       'Provider cost multiplies'):
            assert text_key(wanted) in scope, 'delegation review omitted child count/bounds: ' + wanted
        for index, call in enumerate(scenario.calls, 1):
            args = call['arguments']
            for wanted in (f'Child {index}', args['role'], args['instruction'], *args['paths']):
                assert text_key(wanted) in scope, 'delegation review omitted exact child scope: ' + wanted

    def work_rows(self, scenario):
        self.session.command('party-board', settle=False)
        self.wait(lambda: 'Party board' in self.session.display(), 'delegation work board did not open')
        pages = []
        for index in range(4):
            pages.append(self.session.display())
            self.capture(f'{scenario.mode}-work-row-{index}')
            self.session.send(DOWN, settle=False)
            self.session.pump(.15)
        joined = '\n'.join(pages)
        visible = text_key(joined)
        for index, call in enumerate(scenario.calls, 1):
            role = 'reviewing' if call['arguments']['role'] == 'review' else 'exploring'
            assert text_key(f'Helper {index}') in visible, 'work board omitted a real child row'
            assert text_key(role + ' for') in visible, 'work board omitted child role/parent attribution'
        assert text_key('read-only child') in visible, 'work board did not label child capabilities'
        files.save(self.output / (scenario.mode + '-work-rows.json'), pages)
        self.session.send(ESC, settle=False)

    def assert_no_effects(self, baseline):
        assert project_bytes(self.project) == baseline, 'delegated operation changed project bytes'
        assert self.state()['executions'] == 0, 'read-only child created a local command execution'

    def finish(self):
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session, owned_work=True)
        assert self.session.wait_exit() == 0, 'binary did not restore a clean terminal exit'


def assert_receipts(state, scenario):
    rows = [row for row in state['receipts']
            if row['record'].get('submitted', True)
            and any(call['id'] == row['record'].get('parent_call_id') for call in scenario.calls)]
    assert len(rows) == 3, 'each actual child needs its own durable receipt'
    parents = {row['record']['parent_request_id'] for row in rows}
    assert len(parents) == 1, 'sibling receipts do not share the actual parent request'
    parent_id = parents.pop()
    parent = next(row for row in state['receipts'] if row['request_id'] == parent_id)
    by_call = {row['record']['parent_call_id']: row for row in rows}
    assert set(by_call) == {call['id'] for call in scenario.calls}, 'child receipt call ownership was conflated'
    expected_ids = [by_call[call['id']]['request_id'] for call in scenario.calls]
    assert parent['record']['child_request_ids'] == expected_ids, 'parent receipt did not link every child in call order'
    assert parent['record'].get('parent_request_id') is None, 'lead receipt was misidentified as a child'
    for row in [parent, *rows]:
        record = row['record']
        assert record['request_id'] == row['request_id'] and record['task_id'] == parent['task_id']
        assert record.get('submitted', True), 'prepared delegated receipt was mistaken for actual submission evidence'
        assert row['source'] is not None, 'public fixture packet was not retained for exact receipt evidence'
        raw = row['source'].encode()
        matching = [request for request in scenario.rows if request['body_bytes_hex'] == raw.hex()]
        assert len(matching) == 1, 'receipt did not retain exactly its own frozen initial HTTP body'
        assert record['bytes'] == len(raw) and bytes(record['sha256']) == hashlib.sha256(raw).digest()
        assert sum(section['bytes'] for section in record['sections']) + record['envelope_bytes'] == len(raw)
        if record.get('parent_request_id'):
            assert record['writing'] is False, 'child receipt claimed writable authority'
            packet = json.dumps(matching[0]['body'], ensure_ascii=False)
            for identity in (parent_id, row['request_id'], record['parent_call_id']):
                assert identity in packet, 'frozen child packet omitted its explicit request/parent/call identity'
    if scenario.mode == 'success':
        lead = next(row for row in scenario.rows if row.get('kind') == 'parent' and row.get('round') == 1)
        assert [item['child_request_id'] for item in lead['delegation_results']] == expected_ids, 'lead result identities do not match durable child receipts'
        assert all(item['parent_request_id'] == parent_id for item in lead['delegation_results'])
    return parent_id, expected_ids


def success(journey, result):
    baseline = project_bytes(journey.project)
    scenario = journey.begin_round('success')
    journey.work_rows(scenario)
    journey.assert_no_effects(baseline)
    files.save(journey.output / 'success-overlap-storage.json', journey.state())
    scenario.read_release.set()
    journey.wait(lambda: all(event.is_set() for event in scenario.final_ready),
                 'child source reads/write refusal did not return real tool results')
    journey.assert_no_effects(baseline)
    for index in (2, 1, 0):
        scenario.final_release[index].set()
        journey.wait(lambda index=index: scenario.finished[index].is_set(), 'child final HTTP response did not finish')
        # A durable usage row proves the app consumed this terminal response,
        # rather than merely observing a server-side flush before the next release.
        identity = f'success-child-{index}-1'
        journey.wait(lambda identity=identity: any(
            measurement['report']['identity'].get('response') == identity
            for record in journey.state()['usage'] for measurement in record['measurements']),
            'completed child usage was not durably persisted')
    assert scenario.completion_order == [2, 1, 0], 'fixture failed to exercise reverse child completion'
    journey.wait(scenario.parent_ready.is_set, 'lead did not receive all ordered child results')
    assert len(scenario.rows) == 8, 'successful journey hid a retry, duplicate child or fallback request'
    scenario.parent_release.set()
    journey.wait(lambda: not journey.state()['claims'], 'successful delegation left claimed work')
    journey.wait(lambda: len(journey.state()['usage']) == 4, 'child usage was missing, duplicated or merged into the lead')
    state = journey.state()
    parent, children = assert_receipts(state, scenario)
    by_id = {record['request_id']: record for record in state['usage']}
    assert len(by_id) == 4 and set(by_id) == {parent, *children}, 'usage lacks exact distinct parent/child request ownership'
    expected = {parent: (30, 3), **{request: (70 + index * 40, 7 + index * 4)
                                        for index, request in enumerate(children)}}
    for request, totals in expected.items():
        record = by_id[request]
        assert usage_totals(record) == totals, 'duplicate usage snapshots were summed or child usage contaminated the parent'
        assert len(record['measurements']) == 2, 'two child/parent model rounds were not accounted exactly once'
    assert tuple(sum(usage_totals(record)[field] for record in by_id.values()) for field in range(2)) == (360, 36)
    journey.session.command('usage')
    wait_for(journey.session, 'SCOPE')
    def usage_visible():
        lines = journey.session.display().splitlines()
        def scope_row(label):
            return next((line for line in lines
                         if re.search(r'[║│|]\s*' + re.escape(label) + r'\s+', line)), '')
        task, launch, parent_row = (scope_row(label) for label in ('Task', 'This launch', 'Request (latest)'))
        if journey.session.screen.columns >= 90:
            combined = all(re.search(r'\b360\s+in\b.*\b36\s+out\b', line) for line in (task, launch))
            separate = re.search(r'\b30\s+in\b.*\b3\s+out\b', parent_row)
        else:
            combined = all('396 tok' in line for line in (task, launch))
            separate = '33 tok' in parent_row
        return bool(combined and separate)
    journey.wait(usage_visible, 'UI did not show separate raw parent usage and combined child usage exactly once')
    journey.capture('success-usage-parent-and-children-once')
    journey.session.send(ESC)
    journey.session.command('receipts')
    wait_for(journey.session, 'Outgoing receipts')
    journey.capture('success-linked-receipts-inspector')
    journey.session.send(ESC)
    journey.session.command('response')
    journey.session.send(ALT_END)
    wait_for(journey.session, 'XC02_PARENT_ORDERED_COMPLETE', timeout=35)
    journey.capture('success-ordered-parent-summary')
    journey.workspace()
    journey.assert_no_effects(baseline)
    files.save(journey.output / 'success-storage.json', state)
    result['cases'].extend(['one exact round review with all three instructions, paths and bounds',
                            'three actual overlapping child HTTP requests and visible work rows',
                            'real source reads plus executor-refused unadvertised child write with unchanged bytes',
                            'reverse child completion; lead receives three results in original call order',
                            'exact frozen child receipts linked to parent and parent to all children',
                            'four separate usage rows; duplicate provider snapshots aggregate once in the actual UI'])


def cancellation(journey, result):
    baseline = project_bytes(journey.project)
    original_usage = {record['request_id']: record for record in journey.state()['usage']}
    scenario = journey.begin_round('cancel')
    journey.work_rows(scenario)
    before = len(journey.peer.snapshot('POST'))
    files.save(journey.output / 'cancel-waiting-storage.json', journey.state())
    started = time.monotonic()
    journey.session.command('cancel', settle=False)
    journey.wait(lambda: all(event.is_set() for event in scenario.closed),
                 'lead cancel did not locally close all three waiting child requests', timeout=15)
    journey.wait(lambda: not journey.state()['claims'], 'cancel left durable work claimed', timeout=20)
    journey.wait(lambda: len(journey.state()['usage']) == 8, 'cancelled children did not retain separate usage coverage exactly once')
    state = journey.state()
    assert len(journey.peer.snapshot('POST')) == before, 'cancel retried or continued a parent/child inference'
    assert scenario.parent_rounds == 1 and scenario.child_rounds == [1, 1, 1]
    assert not scenario.completion_order and not any(event.is_set() for event in scenario.finished)
    assert len(scenario.rows) == 4, 'cancel journey hid a retry, duplicate child or fallback request'
    parent, children = assert_receipts(state, scenario)
    retained_usage = {record['request_id']: record for record in state['usage']}
    assert len(retained_usage) == 8 and set(retained_usage) == {*original_usage, parent, *children}
    assert all(retained_usage[request] == record for request, record in original_usage.items()), 'cancel altered earlier parent/child usage'
    assert usage_totals(retained_usage[parent]) == (10, 1), 'cancel merged child usage into its parent'
    for request in children:
        assert all(measurement['attributable'].get(field) is None
                   for measurement in retained_usage[request]['measurements']
                   for field in ('input', 'output')), 'unreported cancelled-child usage became a fabricated zero'
    journey.assert_no_effects(baseline)
    journey.session.command('party-board', settle=False)
    journey.wait(lambda: 'No partner owns working or waiting work.' in journey.session.display(),
                 'cancel left helper work rows active after durable claims were released')
    journey.capture('cancel-local-children-closed-no-claims-no-effects')
    files.save(journey.output / 'cancel-storage.json', state)
    result['cancel_seconds'] = time.monotonic() - started
    journey.session.send(ESC, settle=False)
    result['cases'].append('parent cancel locally closes all three held child sockets without retry, residual claims, commands or changed project bytes')


def run(binary, output, size):
    output.mkdir(parents=True, exist_ok=False)
    root = output / 'isolated'
    root.mkdir(mode=0o700)
    result = {'status': 'failed', 'size': list(size), 'route': 'compatible',
              'protocol': 'chat_completions', 'cases': [], 'isolated': str(root)}
    journey = None
    with DelegationPeer() as peer:
        try:
            journey = DelegationJourney(binary, output, root, size, size[0] == 60,
                                         'compatible', 'chat_completions', True, False, peer)
            wait_for(journey.session, 'Storage: saved', timeout=35)
            # Evidence may live below this checkout's ignored target directory.
            # Give the owned project its own real inventory boundary instead of
            # inheriting an ancestor's ignore rules or disabling native policy.
            git = shutil.which('git')
            assert git, 'installed Git is required for isolated native source-policy checks'
            git = Path(git).resolve()
            (root / 'empty-bin' / ('git.exe' if os.name == 'nt' else 'git')).symlink_to(git)
            environment = {key: value for key, value in os.environ.items()
                           if not key.upper().startswith('GIT_')}
            environment.update(HOME=str(journey.home), USERPROFILE=str(journey.home),
                               GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull)
            subprocess.run([str(git), 'init', '-q', '--template='], cwd=journey.project,
                           env=environment, check=True)
            result['source_fixture'] = {'root': str(journey.project), 'git': str(git),
                                        'inventory': 'standalone Git project; no ancestor ignore policy'}
            for path, content in SOURCES.items():
                (journey.project / path).write_bytes(content.encode())
            journey.setup_route()
            journey.session.command('role-plan')
            journey.workspace()
            success(journey, result)
            cancellation(journey, result)
            journey.finish()
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey is not None:
                journey.capture('failure')
                try:
                    files.save(output / 'failure-storage.json', journey.state())
                except Exception:
                    result['storage_evidence_error'] = traceback.format_exc()
        finally:
            if journey is not None:
                journey.close()
            files.save(output / 'http.json', peer.snapshot())
            files.save(output / 'peer.json', {'errors': peer.errors, 'rounds': [row.evidence() for row in peer.rounds]})
            files.save(output / 'result.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = {'status': 'failed', 'binary': str(binary), 'sha256': digest,
              'journeys': [], 'limits': LIMITS}
    try:
        for size in SIZES:
            report['journeys'].append(run(binary, output / ('%dx%d' % size), size))
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'supplied binary changed during qualification'
        report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    except Exception as error:
        report.update(error=str(error), traceback=traceback.format_exc())
    finally:
        files.save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
