#!/usr/bin/env python3
"""Actual file creation through native HTTP tools and controlled Codex permissions.

An already-built binary runs in isolated PTYs with disposable homes/projects.
HTTP/JSONL peers request operations but never perform the tested writes. No model,
real credential, production endpoint, SQL write or user installation is used.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import threading
import time
import traceback
import unicodedata
from unittest.mock import patch

import cooperation_smoke
from native_connection_smoke import MATRIX, MODEL, Peer, decoded_payload, until
from response_smoke import SurfaceJourney, SurfaceSession, SURFACE
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available
from workbench_smoke import wait_for
from connection_smoke_helpers import focus_field, open_setup

ESC = '\x1b'
UP, HOME, DELETE = ESC + '[A', ESC + '[H', ESC + '[3~'
TOOL = 'eevee_write_file'
FIRST = 'generated 界/cafe\u0301.py'
EMPTY = 'generated 界/empty.txt'
LARGE = 'generated 界/large.py'
LARGE_CONTENT = ''.join(f'generated_{index:04} = \"' + 'literal-code-' * 8 + '\"\r\n' for index in range(200))
CONTENT = '# literal e\u0301 / café\r\nvalue = "public fixture"\r\n'
REPLACEMENT = CONTENT + 'second = 2\r\n'
CREATED = 'The reviewed fixture files were created.'
REPLACED = 'The reviewed existing file was replaced.'
DENIED = 'Denied fixture; no file was created.'
SIGNATURE = 'Zml4dHVyZS1vcGFxdWUtc2lnbmF0dXJl'


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def normalized(text):
    # Terminal-cell comparison only; source paths and wire/content bytes stay literal.
    return unicodedata.normalize('NFC', text)


def pieces(text):
    # Deliberately cut JSON escapes, names and Unicode source bytes across deltas.
    cuts = (1, 3, 8, 17, 31, len(text))
    start = 0
    for end in cuts:
        end = min(end, len(text))
        if end > start:
            yield text[start:end]
            start = end
    if start < len(text):
        yield text[start:]


class Gate:
    def __init__(self):
        self.ready = threading.Event()
        self.release = threading.Event()

    def hold(self, peer):
        self.ready.set()
        deadline = time.monotonic() + 15
        while not self.release.wait(.03):
            if peer.stopped.is_set():
                return False
            if time.monotonic() >= deadline:
                raise AssertionError('fixture terminal-frame gate was not released')
        return True


def tool_arguments(call):
    if 'arguments' in call:
        return call['arguments']
    return {'path': call['path'], 'content': call['content']}


def response_plan(peer, *, calls=(), text='', number=0, gate=None):
    """Produce real terminal SSE, with no provider response ID on purpose."""
    protocol = peer.protocol
    calls = list(calls)

    def reply(handler, event, row):
        observed = []

        def emit(value, name=None):
            observed.append({'event': name, 'data': value})
            event(value, name)

        try:
            if protocol == 'responses':
                output = []
                if text:
                    emit({'type': 'response.output_text.delta', 'delta': text})
                    output.append({'type': 'message', 'role': 'assistant', 'content': [
                        {'type': 'output_text', 'text': text}]})
                for index, call in enumerate(calls, int(bool(text))):
                    args = json.dumps(tool_arguments(call))
                    item = {'type': 'function_call', 'id': 'item-' + call['id'],
                            'call_id': call['id'], 'name': call.get('name', TOOL), 'arguments': ''}
                    emit({'type': 'response.output_item.added', 'output_index': index, 'item': item})
                    for fragment in pieces(args):
                        emit({'type': 'response.function_call_arguments.delta', 'item_id': item['id'],
                              'output_index': index, 'delta': fragment})
                    item = item | {'arguments': args, 'status': 'completed'}
                    emit({'type': 'response.function_call_arguments.done', 'item_id': item['id'],
                          'output_index': index, 'arguments': args})
                    emit({'type': 'response.output_item.done', 'output_index': index, 'item': item})
                    output.append(item)
                if gate and not gate.hold(peer):
                    return
                emit({'type': 'response.completed', 'response': {
                    'model': MODEL, 'status': 'completed', 'output': output,
                    'usage': {'input_tokens': 100 + number, 'output_tokens': 10 + number}}})
            elif protocol == 'chat_completions':
                if text:
                    emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {'content': text},
                                                       'finish_reason': None}]})
                for index, call in enumerate(calls):
                    args = json.dumps(tool_arguments(call))
                    emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {'tool_calls': [
                        {'index': index, 'id': call['id'], 'type': 'function',
                         'function': {'name': call.get('name', TOOL), 'arguments': ''}}]},
                        'finish_reason': None}]})
                    for fragment in pieces(args):
                        emit({'choices': [{'index': 0, 'delta': {'tool_calls': [
                            {'index': index, 'function': {'arguments': fragment}}]},
                            'finish_reason': None}]})
                if gate and not gate.hold(peer):
                    return
                emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {},
                      'finish_reason': 'stop'}]})
                # Duplicate snapshots are one report, not extra consumption.
                usage = {'prompt_tokens': 100 + number, 'completion_tokens': 10 + number}
                for _ in range(2):
                    emit({'model': MODEL, 'choices': [], 'usage': usage})
                handler.wfile.write(b'data: [DONE]\n\n')
                handler.wfile.flush()
            elif protocol == 'messages':
                emit({'type': 'message_start', 'message': {'model': MODEL,
                      'usage': {'input_tokens': 100 + number, 'output_tokens': 0}}}, 'message_start')
                offset = 0
                if text:
                    emit({'type': 'content_block_start', 'index': 0,
                          'content_block': {'type': 'text', 'text': ''}}, 'content_block_start')
                    emit({'type': 'content_block_delta', 'index': 0,
                          'delta': {'type': 'text_delta', 'text': text}}, 'content_block_delta')
                    emit({'type': 'content_block_stop', 'index': 0}, 'content_block_stop')
                    offset = 1
                for index, call in enumerate(calls, offset):
                    emit({'type': 'content_block_start', 'index': index, 'content_block': {
                        'type': 'tool_use', 'id': call['id'], 'name': call.get('name', TOOL),
                        'input': {}}}, 'content_block_start')
                    args = json.dumps(tool_arguments(call))
                    for fragment in pieces(args):
                        emit({'type': 'content_block_delta', 'index': index,
                              'delta': {'type': 'input_json_delta', 'partial_json': fragment}},
                             'content_block_delta')
                    emit({'type': 'content_block_stop', 'index': index}, 'content_block_stop')
                if gate and not gate.hold(peer):
                    return
                emit({'type': 'message_delta', 'delta': {
                    'stop_reason': 'end_turn'},
                    'usage': {'output_tokens': 10 + number}}, 'message_delta')
                emit({'type': 'message_stop'}, 'message_stop')
            else:
                parts = ([{'text': text}] if text else []) + [
                    {'functionCall': {'id': call['id'], 'name': call.get('name', TOOL),
                                      'args': tool_arguments(call)},
                     'thoughtSignature': SIGNATURE}
                    for call in calls]
                emit({'modelVersion': MODEL, 'candidates': [{'index': 0,
                      'content': {'role': 'model', 'parts': parts}}]})
                if gate and not gate.hold(peer):
                    return
                emit({'modelVersion': MODEL, 'candidates': [{'index': 0, 'finishReason': 'STOP'}],
                      'usageMetadata': {'promptTokenCount': 100 + number,
                                        'candidatesTokenCount': 10 + number}})
        except (OSError, BrokenPipeError, ConnectionResetError):
            if not peer.stopped.is_set():
                with peer.lock:
                    peer.errors.append('unexpected fixture response closure')
        except Exception as error:
            with peer.lock:
                peer.errors.append(str(error))
        finally:
            row['tool_fixture_events'] = observed

    return ('custom', reply)


def tool_results(protocol, body):
    if protocol == 'responses':
        return {row['call_id']: row['output'] for row in body['input']
                if row.get('type') == 'function_call_output'}
    if protocol == 'chat_completions':
        return {row['tool_call_id']: row['content'] for row in body['messages']
                if row.get('role') == 'tool'}
    if protocol == 'messages':
        return {part['tool_use_id']: part.get('content') for row in body['messages']
                if isinstance(row.get('content'), list) for part in row['content']
                if part.get('type') == 'tool_result'}
    return {part['functionResponse'].get('id', part['functionResponse']['name']):
            part['functionResponse']['response'] for row in body['contents']
            for part in row.get('parts', []) if 'functionResponse' in part}


def call_ids(protocol, body):
    if protocol == 'responses':
        return [row['call_id'] for row in body['input'] if row.get('type') == 'function_call']
    if protocol == 'chat_completions':
        return [call['id'] for row in body['messages'] for call in row.get('tool_calls', [])]
    if protocol == 'messages':
        return [part['id'] for row in body['messages'] if isinstance(row.get('content'), list)
                for part in row['content'] if part.get('type') == 'tool_use']
    return [part['functionCall'].get('id') for row in body['contents']
            for part in row.get('parts', []) if 'functionCall' in part]


def advertised_tools(protocol, body):
    entries = body.get('tools', [])
    if protocol == 'gemini':
        return {entry['name'] for group in entries for entry in group.get('functionDeclarations', [])}
    return {entry.get('function', entry)['name'] for entry in entries}


def file_review_pending(session):
    view = session.display()
    return any(button in view for button in ('[ Allow once ]', '[ 한 번 허용 ]'))


class CreationJourney(SurfaceJourney):
    # Shared label-based navigation covers primary/advanced native forms while
    # this journey continues to assert actual tools, authority and file bytes.
    def setup_route(self):
        session = self.session
        fields = self.surface['fields']
        labels = list(fields) if self.route == 'compatible' else [fields[i] for i in (0, 1, 2, 5, 6)]
        open_setup(session, self.route, self.language, advanced=True)
        before = len(self.peer.snapshot('POST'))
        metadata = len(self.peer.snapshot('GET')) + 1
        for label in labels:
            field_name = dict(zip(fields, ('key', 'endpoint', 'model', 'protocol', 'auth', 'storage', 'save')))[label]
            focus_field(session, field_name, self.language)
            assert '> ' + label + ':' in session.display(), session.display()
            if label == fields[0] and not self.keyless:
                self.private_paste(self.peer.secret)
            elif label == fields[1]:
                session.send('\x15')
                session.paste(self.peer.base)
            elif label == fields[2]:
                session.send('\x15')
                session.paste(MODEL)
            elif label == fields[3] and self.protocol == 'responses':
                session.send(' ')
            elif label == fields[4] and self.keyless:
                session.send(' ')
            elif label == fields[6]:
                session.send('\r')
                break
        self.choose_saved_model(metadata)
        assert len(self.peer.snapshot('POST')) == before

    def paths(self, paths):
        before = self.snapshot()
        posts = len(self.peer.snapshot('POST'))
        def open_paths():
            if 'JSON' in self.session.display():
                return True
            self.workspace()
            self.session.command('paths')
            return 'JSON' in self.session.display()
        until(self.session, open_paths, 'settled path selector did not open', timeout=40)
        self.session.send(UP * 16 + HOME + DELETE * 4096)
        self.session.paste(json.dumps(paths, ensure_ascii=False))
        self.session.send('\r')
        self.workspace()
        assert len(self.peer.snapshot('POST')) == posts, 'path selection dispatched inference'
        assert self.snapshot()['executions'] == before['executions'], 'path selection ran a command'
        self.capture('selected-paths-' + str(posts))

    def begin(self, request, plans, *, writing=True):
        self.write_request(request)
        before = len(self.peer.snapshot('POST'))
        for plan in plans:
            self.peer.plans.put(plan)
        self.session.send('\r')
        approved = False

        def submitted():
            nonlocal approved
            if len(self.peer.snapshot('POST')) >= before + 1:
                return True
            view = self.session.display()
            if not approved and permission_run_available(self.session):
                self.capture('request-scope-before-inference-' + str(before))
                self.session.send('y')
                approved = True
            elif not approved and inspect_permission_scope(self.session):
                pass
            elif 'Alt+A' in view:
                self.session.send(ESC + 'a')
            return False

        until(self.session, submitted, 'owned scoped implementation did not dispatch', timeout=40)
        claims = self.snapshot()['claims']
        assert len(claims) == 1 and bool(claims[0]['writing']) is writing, 'native operation bypassed its actual lease mode'
        body = self.peer.snapshot('POST')[before]['body']
        expected = {'eevee_list_files', 'eevee_read_file'}
        if writing:
            expected |= {'eevee_write_file', 'eevee_edit_file'}
        assert advertised_tools(self.protocol, body) == expected, 'native operation advertised the wrong file capability'
        return before

    def permission(self, path, *, allow, kind='Create'):
        # No workspace/approval command may manufacture the initial presentation.
        until(self.session, lambda: file_review_pending(self.session),
              'owned file approval did not open automatically', timeout=40)
        reviews = getattr(self, '_file_reviews', 0)
        self._file_reviews = reviews + 1
        self.capture('automatic-review-' + path.replace('/', '-')[:70])
        target = self.project / path
        original = target.read_bytes() if target.is_file() else None
        before = len(self.peer.snapshot('POST'))
        if reviews == 0:
            self.session.send('ayn')
            self.session.paste('a\ny')
            self.session.send('\x1b[121;5u\x1b[121;1:2u\x1b[13;1:2u')
            assert file_review_pending(self.session), 'arriving typing engaged file consent'
            assert len(self.peer.snapshot('POST')) == before, 'arriving typing answered file consent'
            assert (target.read_bytes() if target.is_file() else None) == original, 'arriving typing changed file bytes'
            self.capture('typing-paste-repeat-cannot-engage-or-answer')
        if reviews == 1:
            self.session.send(ESC)
            self.session.pump(.3)
            assert not file_review_pending(self.session), 'deferred identity reopened itself'
            assert len(self.peer.snapshot('POST')) == before, 'Escape answered a deferred file proposal'
            self.capture('deferred-without-answer-or-reopen')
        # Inspect the actual change directly; navigation then chooses the
        # one-shot decision without an extra engagement-only summary.
        if reviews == 0:
            self.session.send('\r')
            self.capture('direct-content-inspection')
            assert len(self.peer.snapshot('POST')) == before
            assert (target.read_bytes() if target.is_file() else None) == original
            self.session.send('d')
        elif reviews == 1:
            self.session.send('\r')  # Reopen the deferred owner.
        wanted = normalized(path)
        until(self.session, lambda: wanted in self.session.display()
              and any(word in self.session.display() for word in ('Allow once', '한 번', '1회')),
              'exact file permission was not visible: ' + path, timeout=40)
        view = self.session.display()
        assert kind in view or (kind == 'Create' and '생성' in view) or (kind == 'Replace' and any(label in view for label in ('수정', '교체', 'Modify')))
        self.capture(('allow-' if allow else 'deny-') + path.replace('/', '-')[:70])
        if reviews == 0:
            self.session.send('\r\r')
            assert len(self.peer.snapshot('POST')) == before, 'neutral Enter answered the file request'
            assert (target.read_bytes() if target.is_file() else None) == original, 'neutral inspection changed the file'
            self.capture('enter-inspects-without-answer')
            self.session.send('d')  # Leave optional technical inspection; exact summary must render again.
        self.session.send(ESC + ('[C' if allow else '[D'))
        self.session.send('\r')

    def settled(self, answer, post_count):
        def complete():
            view = self.session.display()
            if any(title in view for title in ('Evolution!', '진화했어요!')):
                self.capture('evolution-popup-after-safe-settlement-' + str(post_count))
                self.session.send(ESC)
                return False
            return (len(self.peer.snapshot('POST')) == post_count
                    and not self.snapshot()['claims'] and 'Storage: saved' in view
                    and (answer is None or answer in view))
        until(self.session, complete,
              'native tool result and durable ownership did not settle', timeout=40)
        self.capture('settled-' + str(post_count))
        assert self.snapshot()['executions'] == 0, 'file implementation fabricated a local command'
        self.workspace()

    def usage(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            return [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_usage ORDER BY rowid')]

    def receipts(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            return db.execute('SELECT count(*) FROM move_dispatches').fetchone()[0]


def native_case(binary, output, root, size, ascii_mode, language, variant, advanced):
    route, protocol, keyless, _persist = variant
    output.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'route': route, 'protocol': protocol,
              'language': language, 'size': list(size), 'ascii': ascii_mode,
              'cases': [], 'limits': 'Real UI/local writes with synthetic loopback protocol; not live-provider conformance'}
    journey = None
    gate = Gate()
    with Peer(protocol, 'sk-public-file-fixture-only', keyless, retain_body_bytes=True) as peer:
        try:
            journey = CreationJourney(binary, output, root, size, ascii_mode, route, protocol,
                                      keyless, False, peer, language=language)
            wait_for(journey.session, 'Storage: saved')
            assert not any(journey.project.iterdir()), 'creation fixture did not start with an empty project'
            journey.setup_route()
            request = 'Generate three project files; propose their paths and preserve literal CRLF/Unicode and an empty file.'
            calls = [{'id': 'create-first', 'path': FIRST, 'content': CONTENT},
                     {'id': 'create-empty', 'path': EMPTY, 'content': ''},
                     {'id': 'create-large', 'path': LARGE, 'content': LARGE_CONTENT}]
            intro = 'Preparing the requested project files.'
            first = journey.begin(request, [response_plan(peer, calls=calls, text=intro, gate=gate),
                                            response_plan(peer, text=CREATED, number=1)])
            until(journey.session, gate.ready.is_set, 'fragmented tool round did not reach terminal gate')
            assert not (journey.project / FIRST).parent.exists(), 'stream fragments created a parent directory'
            assert not (journey.project / EMPTY).exists(), 'stream fragments created a file'
            assert not (journey.project / LARGE).exists(), 'stream fragments created generated code'
            journey.capture('fragmented-arguments-no-effects-before-terminal')
            gate.release.set()
            journey.permission(FIRST, allow=True)
            until(journey.session, lambda: (journey.project / FIRST).is_file(), 'approved first creation missing')
            assert (journey.project / FIRST).read_bytes() == CONTENT.encode()
            assert not (journey.project / EMPTY).exists()
            assert len(peer.snapshot('POST')) == first + 1, 'batch continued before every permission settled'
            journey.permission(EMPTY, allow=True)
            until(journey.session, lambda: (journey.project / EMPTY).is_file(), 'approved empty creation missing')
            assert (journey.project / EMPTY).read_bytes() == b''
            assert not (journey.project / LARGE).exists()
            assert len(peer.snapshot('POST')) == first + 1, 'batch continued before the large-file permission'
            journey.permission(LARGE, allow=True)
            journey.settled(CREATED, first + 2)
            assert (journey.project / LARGE).read_bytes() == LARGE_CONTENT.encode()
            continuation = peer.snapshot('POST')[first + 1]['body']
            results = tool_results(protocol, continuation)
            assert len(results) == 3 and set(call_ids(protocol, continuation)) == {'create-first', 'create-empty', 'create-large'}, continuation
            rendered_results = json.dumps(results, ensure_ascii=False)
            assert all(path in rendered_results for path in (FIRST, EMPTY, LARGE)), 'real tool results lost exact paths'
            if protocol == 'gemini':
                assert SIGNATURE in json.dumps(continuation), 'opaque call signature was not retained for continuation'
            reports = journey.usage()
            measurements = [m for report in reports for m in report.get('measurements', [])]
            assert len(measurements) == 2, reports
            assert sorted((m['attributable']['input'], m['attributable']['output'])
                          for m in measurements) == [(100, 10), (101, 11)], reports
            assert all(m['report']['identity']['response'] is None for m in measurements), reports
            assert len({json.dumps(m['report']['identity'], sort_keys=True) for m in measurements}) == 2
            assert journey.receipts() == 1, 'tool continuation authored another request receipt'
            result['cases'].append('no-preselection empty-project generation; nested Unicode/CRLF/large/empty files; terminal-only exact approvals; shared parents; distinct round usage')

            inspect_request = 'List the project files, read the generated Python source, and edit only its value using the observed digest.'
            edited_answer = 'The observed source was edited.'
            edited_content = CONTENT.replace('"public fixture"', '"edited fixture"')
            before = journey.begin(inspect_request, [
                response_plan(peer, calls=[{'id': 'discover-source', 'name': 'eevee_list_files',
                    'arguments': {'query': 'generated', 'offset': 0}}], number=4),
                response_plan(peer, calls=[{'id': 'read-source', 'name': 'eevee_read_file',
                    'arguments': {'path': FIRST, 'start_line': 1, 'end_line': 0}}], number=5),
                response_plan(peer, calls=[{'id': 'edit-source', 'name': 'eevee_edit_file',
                    'arguments': {'path': FIRST, 'sha256': hashlib.sha256(CONTENT.encode()).hexdigest(),
                                  'old_text': '"public fixture"', 'new_text': '"edited fixture"'}}], number=6),
                response_plan(peer, text=edited_answer, number=7)])
            until(journey.session, lambda: len(peer.snapshot('POST')) == before + 3,
                  'project discovery and source reading did not continue to an edit proposal')
            assert (journey.project / FIRST).read_bytes() == CONTENT.encode(), 'reading or proposing edited the source'
            listed = json.dumps(tool_results(protocol, peer.snapshot('POST')[before + 1]['body']), ensure_ascii=False)
            assert normalized(FIRST) in normalized(listed), 'non-Git discovery omitted the generated file'
            observed = json.dumps(tool_results(protocol, peer.snapshot('POST')[before + 2]['body']), ensure_ascii=False)
            assert hashlib.sha256(CONTENT.encode()).hexdigest() in observed, 'read result did not contain the actual source digest'
            assert 'public fixture' in observed, 'read result did not contain the actual working code'
            journey.permission(FIRST, allow=True, kind='Replace')
            journey.settled(edited_answer, before + 4)
            assert (journey.project / FIRST).read_bytes() == edited_content.encode(), 'hash-bound exact edit lost unrelated bytes or CRLF'
            result['cases'].append('ordinary non-Git list/read/unique hash-bound edit loop preserves unrelated bytes and requires exact replacement approval')

            journey.paths([FIRST])
            next_request = 'Replace only the selected file with the second reviewed fixture content.'
            before = journey.begin(next_request, [response_plan(peer, calls=[
                {'id': 'replace-first', 'path': FIRST, 'content': REPLACEMENT}], number=2),
                response_plan(peer, text=REPLACED, number=3)])
            _, history = decoded_payload(protocol, peer.snapshot('POST')[before]['body'])
            assert history == [('user', request), ('assistant', intro + '\n\n' + CREATED),
                               ('user', inspect_request), ('assistant', edited_answer),
                               ('user', next_request)], history
            journey.permission(FIRST, allow=True, kind='Replace')
            journey.settled(REPLACED, before + 2)
            assert (journey.project / FIRST).read_bytes() == REPLACEMENT.encode()
            assert (journey.project / EMPTY).read_bytes() == b''
            result['cases'].append('later existing-file replacement preserves unrelated file and only public canonical history')

            if advanced:
                corrected = 'corrected/module.py'
                corrected_content = 'corrected = True\r\n'
                corrected_answer = 'The corrected proposal was created.'
                journey.paths([corrected])
                before = journey.begin('Respect the optional exact target restriction; correct any refused proposal.', [
                    response_plan(peer, calls=[
                        {'id': 'batch-valid', 'path': corrected, 'content': 'must not run\r\n'},
                        {'id': 'batch-outside', 'path': 'not-in-restriction.txt', 'content': 'forbidden\r\n'}], number=4),
                    response_plan(peer, calls=[
                        {'id': 'batch-corrected', 'path': corrected, 'content': corrected_content}], number=5),
                    response_plan(peer, text=corrected_answer, number=6)])
                until(journey.session, lambda: len(peer.snapshot('POST')) == before + 2,
                      'whole-batch refusal did not reach the next model round')
                assert not (journey.project / corrected).parent.exists(), 'valid sibling executed beside an invalid proposal'
                assert not (journey.project / 'not-in-restriction.txt').exists(), 'exact restriction was widened'
                refused = tool_results(protocol, peer.snapshot('POST')[before + 1]['body'])
                assert len(refused) == 2, 'not every refused batch call received a correlated result'
                journey.permission(corrected, allow=True)
                journey.settled(corrected_answer, before + 3)
                assert (journey.project / corrected).read_bytes() == corrected_content.encode()
                assert not (journey.project / 'not-in-restriction.txt').exists()
                result['cases'].append('optional restriction; invalid whole batch performs zero writes; correlated refusal permits one newly reviewed model correction')

            denied = 'not-created/denied.txt'
            journey.paths([])
            before = journey.begin('Propose a new filename without a preselected target; deny its creation.', [response_plan(peer, calls=[
                {'id': 'deny-create', 'path': denied, 'content': 'must not appear\n'}], number=7),
                response_plan(peer, text=DENIED, number=8)])
            journey.permission(denied, allow=False)
            journey.settled(DENIED, before + 2)
            assert not (journey.project / denied).parent.exists(), 'denial created a directory or file'
            denied_wire = tool_results(protocol, peer.snapshot('POST')[before + 1]['body'])
            assert denied_wire, 'denial was not returned to the same provider call'
            result['cases'].append('neutral denial creates no directories/files and continues with an actual correlated denial result')

            # Ordinary planning must not strand a later short implementation
            # request in read-only mode or discard the plan it refers to.
            planned_path = 'planned/한글/saved.txt'
            planned_content = 'implemented from the retained plan\r\n'
            plan_request = ('새 프로젝트 구현 계획 세워줘' if language == 'ko'
                            else 'Plan the implementation of a new project.')
            plan_answer = f'Create {planned_path} with exact content: {planned_content}'
            gate.ready.clear()
            gate.release.clear()
            before = journey.begin(plan_request, [
                response_plan(peer, text=plan_answer, gate=gate, number=20)], writing=False)
            until(journey.session, gate.ready.is_set, 'ordinary plan response')
            assert not (journey.project / planned_path).parent.exists()
            gate.release.set()
            journey.settled('Create planned/', before + 1)
            implementation = '구현해줘' if language == 'ko' else 'Implement it.'
            before = journey.begin(implementation, [
                response_plan(peer, calls=[{'id': 'implement-retained-plan',
                    'path': planned_path, 'content': planned_content}], number=21),
                response_plan(peer, text='Planned file saved.', number=22)])
            sent = json.dumps(peer.snapshot('POST')[before]['body'], ensure_ascii=False)
            assert planned_path in sent, 'short implementation lost the actual preceding plan'
            assert not (journey.project / planned_path).exists()
            journey.permission(planned_path, allow=True)
            journey.settled('Planned file saved.', before + 2)
            assert (journey.project / planned_path).read_bytes() == planned_content.encode()
            result['cases'].append('ordinary plan then short implementation retains plan, acquires writer tools, reviews and saves actual nested bytes')

            if advanced:
                collision = 'collision.txt'
                before = journey.begin('Propose a file whose path will change before approval.', [response_plan(peer, calls=[
                    {'id': 'collide-create', 'path': collision, 'content': 'must not overwrite\n'}], number=6)])
                until(journey.session, lambda: file_review_pending(journey.session),
                      'collision proposal did not open automatically')
                (journey.project / collision).write_bytes(b'external user bytes\n')
                journey.permission(collision, allow=True)
                journey.settled(None, before + 1)
                assert (journey.project / collision).read_bytes() == b'external user bytes\n'
                result['cases'].append('a file appearing after preview/permission is preserved, never silently changed from Create to Replace')

                files_before = {str(path.relative_to(journey.project)): hashlib.sha256(path.read_bytes()).hexdigest()
                                for path in journey.project.rglob('*') if path.is_file()}
                journey.session.command('role-plan')
                journey.workspace()
                gate.ready.clear()
                gate.release.clear()
                answer = 'Read-only fixture complete.'
                before = journey.begin('Inspect the request without granting any file changes.',
                                       [response_plan(peer, text=answer, gate=gate, number=10)], writing=False)
                until(journey.session, gate.ready.is_set, 'read-only response did not reach terminal gate')
                gate.release.set()
                journey.settled(answer, before + 1)
                assert {str(path.relative_to(journey.project)): hashlib.sha256(path.read_bytes()).hexdigest()
                        for path in journey.project.rglob('*') if path.is_file()} == files_before
                result['cases'].append('switching to Plan removes write tools/lease and preserves every generated byte')

            journey.session.idle()
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            gate.release.set()
            save(output / 'http-requests.json', peer.snapshot())
            if journey:
                save(output / 'owned-state.json', journey.snapshot())
                journey.session.close()
            save(output / 'results.json', result)
    return result


class CodexCreationJourney(cooperation_smoke.Journey):
    def __init__(self, binary, output, size, language):
        self.language, self.surface = language, SURFACE[language]

        def session(argv, cwd, environment, dimensions, directory):
            environment = environment | {'EV_COOP_EDIT_PATH': FIRST, 'EV_COOP_EDIT_CONTENT': CONTENT}
            return SurfaceSession(argv, cwd, environment, dimensions, directory, language=language)

        with patch.object(cooperation_smoke, 'Session', session):
            super().__init__(binary, output, size, 'create-' + language)

    def workspace(self):
        for _ in range(12):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if 'Alt+A' in footer or footer.startswith(tuple(self.surface[key] for key in ('home', 'edit', 'draft'))):
                return 'Type Request /' if footer.startswith(self.surface['home']) else 'Enter Send /'
            self.session.send(ESC)
        raise AssertionError('controlled request owner not reachable\n' + self.session.display())

    def until(self, predicate, reason, timeout=40, approve_preview=False):
        approved = False

        def check():
            nonlocal approved
            if predicate():
                return True
            view = self.session.display()
            if approve_preview and not approved and permission_run_available(self.session):
                self.session.send('y')
                approved = True
            elif approve_preview and not approved and inspect_permission_scope(self.session):
                pass
            elif approve_preview and 'Alt+A' in view:
                self.session.send(ESC + 'a')
            return False

        until(self.session, check, reason, timeout)


def codex_case(binary, output, size, language):
    result = {'status': 'failed', 'route': 'codex-controlled', 'language': language,
              'size': list(size), 'cases': [], 'scope': 'Actual Eevee writes, deterministic JSONL peer; not live Codex conformance'}
    journey = None
    try:
        journey = CodexCreationJourney(binary, output, size, language)
        journey.setup(count=1)
        journey.session.command('eevee')
        journey.session.command('role-implement')
        journey.workspace()
        assert not (journey.project / FIRST).parent.exists()
        before_original = journey.source.read_bytes()
        for marker, allow in (('COOP_CREATE_DENY', False), ('COOP_CREATE_ALLOW', True)):
            journey.request(1, marker)
            assert all(row['writing'] for row in journey.snapshot()[2]), 'controlled creation lost writer lease'
            journey.prompt(marker, 'edit')
            journey.until(lambda: file_review_pending(journey.session),
                          'controlled file review did not open automatically')
            journey.session.send('\r')
            journey.until(lambda: normalized(FIRST) in journey.session.display(),
                          'controlled exact new-file content inspection')
            assert not (journey.project / FIRST).parent.exists(), 'permission preparation created target parents'
            journey.session.capture(marker.lower() + '-exact-scope')
            if not allow:
                journey.session.send('\r\r')
                assert not journey.peers('answer', marker), 'neutral inspection denied the controlled write'
                assert not (journey.project / FIRST).parent.exists()
            for _ in range(48):
                journey.session.send(ESC + '[6~')
            journey.session.send('d')
            journey.until(lambda: file_review_pending(journey.session),
                          'controlled file decision controls')
            journey.session.send('a' if allow else 'n')
            journey.until(lambda: bool(journey.peers('answer', marker)), 'controlled tool result did not reach exact peer')
            answer = journey.peers('answer', marker)[-1]['result']
            assert answer['success'] is allow, answer
            if allow:
                journey.until(lambda: (journey.project / FIRST).is_file(), 'controlled creation did not occur')
                assert (journey.project / FIRST).read_bytes() == CONTENT.encode()
            else:
                assert not (journey.project / FIRST).parent.exists(), 'controlled denial created a parent'
            journey.finish(marker)
            journey.until(lambda: not journey.claims(), 'controlled creation ownership did not settle')
            journey.workspace()
            journey.until(lambda: 'Completed ' + marker in journey.session.display(),
                          'controlled final answer was hidden behind earlier tool observations')
            assert journey.source.read_bytes() == before_original, 'new-file tool modified an unrelated existing file'
            result['cases'].append('controlled direct file review with neutral inspection and ' + ('allow' if allow else 'explicit denial'))
        marker = 'COOP_SOURCE_CHAIN'
        journey.request(1, marker)
        journey.publish(marker + '.prompt', json.dumps([
            {'name': 'eevee_list_files', 'arguments': {'query': 'generated', 'offset': 0}},
            {'name': 'eevee_read_file', 'arguments': {'path': FIRST, 'start_line': 1, 'end_line': 0}},
            {'name': 'eevee_edit_file', 'arguments': {'path': FIRST,
                'sha256': hashlib.sha256(CONTENT.encode()).hexdigest(),
                'old_text': '"public fixture"', 'new_text': '"edited fixture"'}}]))
        journey.until(lambda: len(journey.peers('prompt', marker)) == 3,
                      'controlled source tools did not continue from discovery/read to edit')
        answers = journey.peers('answer', marker)
        assert len(answers) == 2 and all(row['result']['success'] for row in answers), answers
        assert 'public fixture' in json.dumps(answers[1]), 'controlled read did not return real source bytes'
        assert hashlib.sha256(CONTENT.encode()).hexdigest() in json.dumps(answers[1]), 'controlled read omitted source digest'
        assert (journey.project / FIRST).read_bytes() == CONTENT.encode(), 'controlled observation changed source'
        journey.until(lambda: file_review_pending(journey.session),
                      'controlled edit review did not open automatically')
        journey.session.send('\r')
        journey.until(lambda: normalized(FIRST) in journey.session.display(),
                      'controlled exact edit content inspection')
        for _ in range(48):
            journey.session.send(ESC + '[6~')
        journey.session.send('d')
        journey.until(lambda: file_review_pending(journey.session),
                      'controlled edit decision controls')
        journey.session.send('a')
        journey.until(lambda: len(journey.peers('answer', marker)) == 3, 'controlled edit did not return its result')
        assert journey.peers('answer', marker)[-1]['result']['success'] is True
        assert (journey.project / FIRST).read_bytes() == CONTENT.replace('"public fixture"', '"edited fixture"').encode()
        assert journey.source.read_bytes() == before_original, 'controlled edit changed unrelated dirty content'
        journey.finish(marker)
        journey.until(lambda: not journey.claims(), 'controlled source tool ownership did not settle')
        journey.workspace()
        journey.until(lambda: 'Completed ' + marker in journey.session.display(),
                      'controlled final source answer was hidden behind tool observations')
        result['cases'].append('ordinary controlled discovery/read/hash-bound edit with real CRLF bytes and one-shot consent')
        journey.until(lambda: not journey.session.display().rstrip().splitlines()[-1].strip().startswith(journey.surface['draft']),
                      'controlled writer UI did not finish settling')
        journey.session.command('role-plan')
        journey.workspace()
        marker = 'COOP_SOURCE_READONLY'
        journey.request(1, marker)
        claims = journey.snapshot()[2]
        assert len(claims) == 1 and not claims[0]['writing'], 'controlled Plan acquired writing authority'
        assert set(journey.peers('session')[-1]['tools']) == {'eevee_list_files', 'eevee_read_file'}, 'controlled Plan advertised mutation tools'
        journey.publish(marker + '.prompt', json.dumps([
            {'name': 'eevee_list_files', 'arguments': {'query': 'generated', 'offset': 0}},
            {'name': 'eevee_read_file', 'arguments': {'path': FIRST, 'start_line': 1, 'end_line': 0}}]))
        journey.until(lambda: len(journey.peers('answer', marker)) == 2, 'controlled readonly source calls did not settle')
        answers = journey.peers('answer', marker)
        assert all(row['result']['success'] for row in answers), answers
        actual_content = CONTENT.replace('"public fixture"', '"edited fixture"').encode()
        assert hashlib.sha256(actual_content).hexdigest() in json.dumps(answers[1])
        assert 'edited fixture' in json.dumps(answers[1])
        assert normalized(FIRST) in normalized(json.dumps(answers[0], ensure_ascii=False))
        assert (journey.project / FIRST).read_bytes() == actual_content
        assert journey.source.read_bytes() == before_original
        journey.finish(marker)
        journey.until(lambda: not journey.claims(), 'controlled readonly ownership did not settle')
        result['cases'].append('controlled Plan advertises only read/list, uses a nonwriting lease and returns actual dirty CRLF source bytes/digest without approval or mutation')
        journey.quit()
        result['status'] = 'passed'
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
    finally:
        if journey:
            journey.close()
        save(output / ('codex-' + language + '-' + str(size[0]) + '.json'), result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--language', default='en', choices=('en', 'ko'))
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    before = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = {'status': 'failed', 'binary': str(binary), 'sha256': before,
              'language': args.language, 'journeys': [],
              'limits': ['Synthetic protocol peers are not live model/billing conformance.',
                         'Native ChatGPT official endpoint is not redirected to a fixture.',
                         'PTY/ConPTY proof is not physical GUI/mouse certification.']}
    try:
        for size, ascii_mode in (((120, 36), False), ((60, 18), True)):
            for variant in MATRIX:
                label = '{}x{}-{}-{}-{}'.format(*size, args.language, variant[0], variant[1])
                with tempfile.TemporaryDirectory(prefix='ev-file-create-漢字-') as temporary:
                    result = native_case(binary, output / label, Path(temporary).resolve(), size,
                                         ascii_mode, args.language, variant,
                                         advanced=(size[0] == (120 if args.language == 'en' else 60)))
                report['journeys'].append(result)
            if os.name == 'posix':
                report['journeys'].append(codex_case(binary, output, size, args.language))
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == before, 'supplied binary changed during qualification'
        report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    except Exception as error:
        report.update(error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
