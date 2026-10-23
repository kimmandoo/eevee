#!/usr/bin/env python3
"""Credential-free native HTTP/TUI qualification against an explicitly supplied binary.

Deterministic loopback SSE peers exercise transport, canonical payload ownership,
private setup and cancellation, not model understanding or live-provider conformance.
No installed provider program, private account, credential store or external service
is used. ChatGPT's pinned official endpoint is deliberately outside this loopback
matrix: its serializer/unit tests and auth_smoke.py --fixture are separate gates.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import re
import socket
import sqlite3
import stat
import tempfile
import threading
import time
import traceback

from terminal_smoke import Session, confirm_exit, terminal_environment, WINDOWS
from workbench_smoke import wait_for
from connection_smoke_helpers import fill_text, focus_field, open_setup

ESC = '\x1b'
HOME = ESC + '[H'
DELETE = ESC + '[3~'
MODEL = 'fixture-selected'
PARTIAL = 'Fixture partial output, never canonical.'
REASONING = 'FIXTURE_PRIVATE_REASONING_NOT_USER_OUTPUT'
FACTS = ('My first fact is amber-713.', 'My second fact is cedar-829.',
         'List both earlier facts and your earlier replies exactly.')
ANSWERS = ('Fixture authoritative reply A: quartz-137.',
           'Fixture authoritative reply B: orbit-241.',
           'Fixture authoritative reply C: no semantic claim.')
MATRIX = (
    ('openai', 'responses', False, True),
    ('anthropic', 'messages', False, False),
    ('google', 'gemini', False, True),
    ('compatible', 'chat_completions', True, True),
    ('compatible', 'responses', False, False),
)
LIMITS = [
    'Loopback deterministic protocol fixtures only; no model understanding, provider entitlement, billing or live protocol conformance claimed.',
    'Real-account qualification blocked: no accounts, actual credentials or consent supplied; none read or used.',
    'Native ChatGPT fixed official endpoint is not redirected/proxied; loopback runtime does not qualify it. Run unit serializers and auth_smoke.py --fixture separately.',
    'History is process-local; no restart-resumable native conversation is claimed.',
    'Private local plaintext persistence is tested, not OS-keychain encryption.',
]


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def until(session, predicate, description, timeout=12):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert session.alive(), 'application exited: ' + description
        assert time.monotonic() < deadline, description + '\n' + session.display()
        session.pump(.05)


class Peer:
    """Own a bounded real HTTP listener; never retain authentication header values."""

    def __init__(self, protocol, secret, keyless, *, retain_body_bytes=False):
        self.protocol, self.secret, self.keyless = protocol, secret, keyless
        self.lock = threading.Lock()
        self.requests, self.errors, self.workers = [], [], []
        self.plans = queue.Queue()
        # Opt-in records acceptance keeps exact public request bodies and emits
        # supplied Chat usage trailers; existing connection journeys are unchanged.
        self.retain_body_bytes = retain_body_bytes
        self.usage_plans = queue.Queue()
        self.stopped = threading.Event()
        self.cancel_started, self.cancel_closed = threading.Event(), threading.Event()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

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
                            fixture.errors.append(type(error).__name__)

            def record(self, body):
                headers = {key.lower(): value for key, value in self.headers.items()}
                sensitive = ('authorization', 'x-api-key', 'x-goog-api-key', 'cookie', 'chatgpt-account-id')
                expected_name = ('x-api-key' if protocol == 'messages' else
                                 'x-goog-api-key' if protocol == 'gemini' else 'authorization')
                expected_value = fixture.secret if expected_name != 'authorization' else 'Bearer ' + fixture.secret
                present = [key for key in sensitive if key in headers]
                auth_ok = not present if fixture.keyless else (
                    present == [expected_name] and headers.get(expected_name) == expected_value)
                row = {'method': self.command, 'path': self.path, 'body': body,
                       'authentication_header_names': present, 'authentication_matches_selected_source': auth_ok,
                       'headers': {key: ('[REDACTED]' if key in sensitive else value)
                                   for key, value in headers.items()}}
                with fixture.lock:
                    fixture.requests.append(row)
                return row

            def reply(self, status, body):
                encoded = json.dumps(body).encode()
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(encoded)))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(encoded)
                self.close_connection = True

            def do_GET(self):
                self.record(None)
                if self.path not in ('/v1/models', '/changed/v1/models'):
                    self.reply(404, {'error': 'unexpected fixture metadata route'})
                    return
                if protocol == 'gemini':
                    body = {'models': [{'name': MODEL, 'displayName': MODEL,
                                        'supportedGenerationMethods': ['generateContent']}]}
                else:
                    body = {'data': [{'id': MODEL, 'name': MODEL}], 'has_more': False}
                self.reply(200, body)

            def do_POST(self):
                size = int(self.headers.get('Content-Length', '0'))
                if not 0 < size <= 1024 * 1024:
                    self.record(None)
                    self.reply(400, {'error': 'unbounded fixture body'})
                    return
                raw_body = self.rfile.read(size)
                row = self.record(json.loads(raw_body))
                if fixture.retain_body_bytes:
                    row['body_bytes_hex'] = raw_body.hex()
                try:
                    usage_plan = fixture.usage_plans.get_nowait()
                except queue.Empty:
                    usage_plan = None
                if usage_plan is not None and protocol != 'chat_completions':
                    with fixture.lock:
                        fixture.errors.append('Chat usage plan supplied for a different protocol')
                    self.reply(409, {'error': 'usage plan protocol mismatch'})
                    return
                endpoint = {'responses': 'responses', 'chat_completions': 'chat/completions',
                            'messages': 'messages', 'gemini': 'models/' + MODEL + ':streamGenerateContent?alt=sse'}[protocol]
                if self.path not in ('/v1/' + endpoint, '/changed/v1/' + endpoint):
                    self.reply(404, {'error': 'unexpected fixture inference route'})
                    return
                try:
                    mode, answer = fixture.plans.get_nowait()
                except queue.Empty:
                    with fixture.lock:
                        fixture.errors.append('unplanned inference: hidden resend or fallback')
                    self.reply(409, {'error': 'no planned inference'})
                    return
                row['fixture_mode'] = mode
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Connection', 'close')
                self.end_headers()
                self.close_connection = True

                def event(value, name=None):
                    if usage_plan is not None:
                        row.setdefault('fixture_events', []).append({'event': name, 'data': value})
                    prefix = ('event: ' + name + '\n') if name else ''
                    self.wfile.write((prefix + 'data: ' + json.dumps(value) + '\n\n').encode())
                    self.wfile.flush()

                if mode == 'custom':
                    answer(self, event, row)
                    return

                text = answer if mode == 'complete' else PARTIAL
                if protocol == 'responses':
                    event({'type': 'response.output_text.delta', 'delta': text})
                elif protocol == 'chat_completions':
                    event({'model': MODEL, 'choices': [{'index': 0, 'delta': {
                        'content': text, 'reasoning_content': REASONING}, 'finish_reason': None}]})
                elif protocol == 'messages':
                    event({'type': 'message_start', 'message': {'model': MODEL}}, 'message_start')
                    event({'type': 'content_block_delta', 'index': 0,
                           'delta': {'type': 'text_delta', 'text': text}}, 'content_block_delta')
                else:
                    event({'modelVersion': MODEL, 'candidates': [{'index': 0, 'content': {'parts': [
                        {'thought': True, 'text': REASONING}, {'text': text}]}}]})
                if mode == 'cancel':
                    fixture.cancel_started.set()
                    self.connection.settimeout(.1)
                    deadline = time.monotonic() + 12
                    while not fixture.stopped.is_set() and time.monotonic() < deadline:
                        try:
                            if self.connection.recv(1) == b'':
                                fixture.cancel_closed.set()
                                return
                        except socket.timeout:
                            continue
                        except (ConnectionResetError, ConnectionAbortedError):
                            fixture.cancel_closed.set()
                            return
                    if not fixture.stopped.is_set():
                        with fixture.lock:
                            fixture.errors.append('cancel did not close streaming socket within deadline')
                    return
                if mode == 'incomplete':
                    return  # EOF after real partial bytes, deliberately no successful terminal frame.
                if protocol == 'responses':
                    event({'type': 'response.completed', 'response': {
                        'model': MODEL, 'status': 'completed', 'output': [{'type': 'message',
                            'role': 'assistant', 'content': [{'type': 'output_text', 'text': answer}]}]}})
                elif protocol == 'chat_completions':
                    identity = {} if usage_plan is None else {'id': usage_plan['id']}
                    event({'model': MODEL, **identity, 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]})
                    if usage_plan is not None:
                        for usage in usage_plan['snapshots']:
                            event({'model': MODEL, **identity, 'choices': [], 'usage': usage})
                    self.wfile.write(b'data: [DONE]\n\n')
                    self.wfile.flush()
                elif protocol == 'messages':
                    event({'type': 'message_delta', 'delta': {'stop_reason': 'end_turn'}}, 'message_delta')
                    event({'type': 'message_stop'}, 'message_stop')
                else:
                    event({'modelVersion': MODEL, 'candidates': [{'index': 0, 'finishReason': 'STOP'}]})

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads, self.server.block_on_close = True, False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={'poll_interval': .05}, daemon=True)

    @property
    def base(self):
        return 'http://127.0.0.1:' + str(self.server.server_port) + '/v1'

    def snapshot(self, method=None):
        with self.lock:
            return [dict(row) for row in self.requests if method is None or row['method'] == method]

    def assert_clean(self):
        with self.lock:
            assert not self.errors, self.errors
        assert self.plans.empty(), 'planned request never consumed'
        assert self.usage_plans.empty(), 'planned usage frame never consumed by an actual request'
        assert all(row['authentication_matches_selected_source'] for row in self.snapshot()), \
            'request authentication does not match explicit native source'

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_error):
        self.stopped.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        with self.lock:
            workers = list(self.workers)
        deadline = time.monotonic() + 3
        for worker in workers:
            worker.join(timeout=max(0, deadline - time.monotonic()))
        assert not self.thread.is_alive() and not any(worker.is_alive() for worker in workers), \
            'loopback listener/worker cleanup exceeded deadline'


def content_text(content):
    if isinstance(content, str):
        return content
    assert isinstance(content, list), 'unexpected role content type'
    assert all(isinstance(part, dict) and isinstance(part.get('text'), str) for part in content)
    return ''.join(part['text'] for part in content)


def decoded_payload(protocol, body):
    if protocol == 'responses':
        assert body['stream'] is True and body['store'] is False
        policy, messages = body['instructions'], body['input']
    elif protocol == 'chat_completions':
        assert body['stream'] is True
        assert body['messages'][0]['role'] == 'system'
        assert sum(row['role'] == 'system' for row in body['messages']) == 1
        policy, messages = body['messages'][0]['content'], body['messages'][1:]
    elif protocol == 'messages':
        assert body['stream'] is True
        policy, messages = body['system'], body['messages']
    else:
        policy = content_text(body['systemInstruction']['parts'])
        messages = [{'role': 'assistant' if row['role'] == 'model' else row['role'],
                     'content': row['parts']} for row in body['contents']]
    return policy, [(row['role'], content_text(row['content'])) for row in messages]


class Journey:
    def __init__(self, binary, output, root, size, ascii_mode, route, protocol, keyless, persist, peer):
        self.output, self.root, self.peer = output, root, peer
        self.route, self.protocol = route, protocol
        self.keyless, self.persist = keyless, persist
        self.home, self.project, self.data = (root / name for name in ('home', 'project', 'data'))
        for path in (self.home, self.project, root / 'empty-bin'):
            path.mkdir(mode=0o700)
        self.secrets = [peer.secret, 'sk-fixture-native-replacement-8f43c20a']
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GEMINI_',
                               'GOOGLE_', 'AWS_', 'OPENCODE_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(self.home), USERPROFILE=str(self.home),
                           XDG_CONFIG_HOME=str(self.home / 'config'), XDG_DATA_HOME=str(self.home / 'data'),
                           XDG_CACHE_HOME=str(self.home / 'cache'), APPDATA=str(self.home / 'config'),
                           LOCALAPPDATA=str(self.home / 'cache'), CODEX_HOME=str(self.home / 'codex'),
                           GEMINI_CLI_HOME=str(self.home / 'gemini'), PATH=str(root / 'empty-bin'),
                           NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost', BROWSER='/bin/true')
        self.session = Session([str(binary), '--data-dir', str(self.data), '--images', 'off',
                                '--color', 'mono', *(['--ascii'] if ascii_mode else [])],
                               self.project, environment, size, output / 'tui')
        self.cases, self.assertions = [], []

    def redacted(self, text):
        for secret in self.secrets:
            text = text.replace(secret, '[REDACTED SYNTHETIC CREDENTIAL]')
        return text

    def capture(self, label):
        display = self.session.display()
        (self.output / (label + '.txt')).write_text(self.redacted(display) + '\n', encoding='utf-8')
        assert all(secret not in display for secret in self.secrets), 'credential visible on screen: ' + label

    def private_paste(self, secret):
        # Session.paste/send retain inputs verbatim. Write only this synthetic private
        # paste directly, with an explicit redacted event, before pumping any output.
        session = self.session
        session.events.append({'input': '[REDACTED synthetic private bracketed paste]'})
        payload = ESC + '[200~' + secret + ESC + '[201~'
        if WINDOWS:
            session.process.write(payload)
        else:
            pending = payload.encode()
            while pending:
                pending = pending[os.write(session.master, pending):]
        session.pump(settle=WINDOWS)
        assert all(value not in session.text for value in self.secrets), 'credential leaked to terminal bytes'

    def snapshot(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            tasks = [dict(row) for row in db.execute('SELECT * FROM tasks ORDER BY id')]
            claims = [dict(row) for row in db.execute('SELECT * FROM active_work')]
            journals = [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_journal ORDER BY rowid')]
            partners = [dict(row) for row in db.execute('SELECT * FROM partners ORDER BY id')]
            executions = db.execute('SELECT count(*) FROM executions').fetchone()[0]
        for task in tasks:
            task['partner_drafts'] = json.loads(task['partner_drafts'])
        return {'tasks': tasks, 'claims': claims, 'journals': journals,
                'partners': partners, 'executions': executions}

    def assert_owned_unchanged(self, before):
        def owned(snapshot):
            # Opening/returning from a sheet may autosave navigation metadata;
            # connection effects must preserve facts, exact drafts and evidence.
            return snapshot | {'tasks': [
                {key: value for key, value in task.items()
                 if key not in ('navigation', 'revision', 'updated_at')}
                for task in snapshot['tasks']]}
        after = self.snapshot()
        assert owned(after) == owned(before), \
            'connection changed task facts/drafts/partners/claims/journals/executions: ' + repr((owned(before), owned(after)))

    def draft(self):
        rows = self.snapshot()['tasks']
        return any(draft['field'] == 'Request' and draft['text'] == FACTS[0]
                   for task in rows for state in task['partner_drafts'].values()
                   for draft in state.get('drafts', []))

    def workspace(self):
        # Escape only temporary drawers; bounded navigation is not an inference retry.
        for _ in range(8):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if footer.startswith(('Type Request /', 'Enter Send /', 'Draft only /')):
                return footer
            self.session.send(ESC)
        raise AssertionError('Request owner not reachable\n' + self.session.display())

    def write_request(self, text):
        if self.workspace().startswith('Type Request /'):
            self.session.send('\r')
        self.session.send((ESC + '[A') * 16 + HOME + DELETE * 256)
        self.session.paste(text)
        self.session.send('\x13')
        until(self.session, lambda: any(
            draft['field'] == 'Request' and draft['text'] == text
            for task in self.snapshot()['tasks'] for state in task['partner_drafts'].values()
            for draft in state.get('drafts', [])), 'exact Request draft acknowledgement')


    def private_focus(self, label):
        assert '> ' + label + ':' in self.session.display(), \
            'private field navigation did not reach ' + label + '\n' + self.session.display()

    def setup(self, *, secret=None, base=None, persist=None, initial=False):
        session = self.session
        before_posts = len(self.peer.snapshot('POST'))
        expected_metadata = len(self.peer.snapshot('GET')) + 1
        open_setup(session, self.route, advanced=True)
        self.private_focus('API key')
        self.capture('setup-before-' + str(len(self.cases)))
        assert not session.screen.cursor.hidden, 'native private credential editor not focused'
        if secret is not None:
            self.private_paste(secret)
            wait_for(session, '********')
        self.capture('setup-masked-' + str(len(self.cases)))
        # Actual field labels own navigation; primary and advanced forms may
        # have different orders without changing this transport boundary.
        fill_text(session, 'endpoint', base or self.peer.base)
        fill_text(session, 'model', MODEL)
        focus_field(session, 'protocol' if self.route == 'compatible' else 'storage')
        self.private_focus('Protocol' if self.route == 'compatible' else 'Storage')
        if self.route == 'compatible':
            if initial and self.protocol == 'responses':
                session.send(' ')
            focus_field(session, 'auth')
            self.private_focus('Authentication')
            if initial and self.keyless:
                session.send(' ')
            focus_field(session, 'storage')
            self.private_focus('Storage')
        if (self.persist if persist is None else persist):
            session.send(' ')
        focus_field(session, 'save')
        self.private_focus('Save & choose model')
        wait_for(session, 'Save & choose model')
        self.capture('setup-explicit-save-and-choose-model-' + str(len(self.cases)))
        if secret is not None:
            self.peer.secret = secret  # The explicit Save also authorizes metadata for this new source.
        session.send('\r')
        self.choose_saved_model(expected_metadata)
        assert len(self.peer.snapshot('POST')) == before_posts, 'Save & choose model performed inference'
        self.capture('setup-saved-model-selected-no-inference-' + str(len(self.cases)))

    def choose_saved_model(self, expected_metadata):
        # Primary Save explicitly selects this route and opens this picker;
        # never detour through Connections or press Enter to use it again.
        wait_for(self.session, MODEL)
        until(self.session, lambda: len(self.peer.snapshot('GET')) == expected_metadata,
              'Save & choose model must issue exactly one native metadata request')
        wait_for(self.session, '1 matches')
        until(self.session, lambda: 'Loading metadata' not in self.session.display(),
              'actual metadata catalogue ready before selection')
        self.session.send(MODEL)
        wait_for(self.session, 'Search: ' + MODEL)
        self.capture('model-metadata-search')
        self.session.send('\r')
        self.workspace()
        assert len(self.peer.snapshot('GET')) == expected_metadata, 'metadata selection performed an unrequested extra fetch/retry'

    def assert_payload(self, row, expected, label):
        body = row['body']
        policy, messages = decoded_payload(self.protocol, body)
        assert messages == expected, label + ': wrong role/raw text/history: ' + repr(messages)
        assert isinstance(policy, str) and 'untrusted' in policy.lower() and 'tools' in policy.lower(), \
            label + ': missing separate system policy and untrusted task/source context'
        assert not any(answer in policy for answer in ANSWERS), 'assistant display/history copied into system policy'
        encoded = json.dumps(body, ensure_ascii=False)
        for forbidden in (*self.secrets, PARTIAL, REASONING, 'Request: ', 'Response: ',
                          'Eevee', 'Vaporeon', 'Jolteon', 'Flareon', 'Espeon', 'Umbreon',
                          'Leafeon', 'Glaceon', 'Sylveon', 'Previous request:', 'Current request:',
                          'Native response completed;'):
            assert forbidden not in encoded, label + ': private/display/noncanonical text in payload'
        assert body.get('model', MODEL) == MODEL, 'model picker did not control actual inference identity'
        assert row['authentication_matches_selected_source'], 'wrong native credential scope'
        self.assertions.append({'case': label, 'roles_and_exact_content': messages,
                                'separate_system_context': True, 'path': row['path']})

    def send(self, text, history, answer, label, mode='complete'):
        self.write_request(text)
        before = len(self.peer.snapshot('POST'))
        before_state = self.snapshot()
        self.peer.plans.put((mode, answer))
        self.capture(label + '-before-send')
        self.session.send('\r')
        until(self.session, lambda: len(self.peer.snapshot('POST')) == before + 1,
              label + ': ordinary send did not dispatch exactly once')
        self.assert_payload(self.peer.snapshot('POST')[-1], history + [('user', text)], label)
        if mode == 'cancel':
            until(self.session, self.peer.cancel_started.is_set, 'streaming cancellation boundary')
            self.capture(label + '-partial-before-cancel')
            self.session.command('cancel')
            until(self.session, self.peer.cancel_closed.is_set, 'cancel must close actual HTTP response socket')
        elif mode == 'complete':
            wait_for(self.session, answer)
        # Tool-free native turns have no remote thread/tool observations to
        # journal. Closing the local reader does not confirm remote cancellation:
        # the unknown-outcome footer must remain honest after local settlement.
        # Canonical history is checked by each subsequent exact real POST.
        expected_state = 'complete' if mode == 'complete' else 'partial'

        def settled():
            state = self.snapshot()
            view = self.session.display()
            response = any(re.search(r'\bResponse \d+/\d+.*\b' + expected_state + r'\b', line)
                           for line in view.splitlines())
            unknown = re.search(r'\b(?:Provider )?[Oo]utcome unknown\b', view) is not None
            activity_settled = unknown if mode != 'complete' else ('Local work idle' in view or unknown)
            return (not state['claims'] and 'Storage: saved' in view
                    and activity_settled and response)

        until(self.session, settled, label + ': native result/storage/claim settlement')
        after_state = self.snapshot()
        assert after_state['journals'] == before_state['journals'], \
            label + ': tool-free native turn fabricated durable provider-tool observations'
        assert after_state['executions'] == before_state['executions'] == 0, \
            label + ': native turn created local execution evidence'
        assert after_state['partners'] == before_state['partners'], label + ': partner ownership changed'
        assert [(task['id'], task['lead_partner']) for task in after_state['tasks']] == \
               [(task['id'], task['lead_partner']) for task in before_state['tasks']], \
            label + ': task/partner ownership changed during native settlement'
        self.assertions[-1]['settlement'] = {
            'response': expected_state, 'storage': 'saved', 'active_claims': [],
            'provider_tool_journal_unchanged': True, 'local_executions': 0,
            'remote_outcome': 'completed' if mode == 'complete' else 'unknown',
            'remote_cancellation_confirmed': False,
            'task_partner_ownership_unchanged': True,
        }
        self.session.pump(.3)
        assert len(self.peer.snapshot('POST')) == before + 1, 'hidden resend after ' + label
        self.capture(label + '-after-settlement')
        self.cases.append(label)
        return history + [('user', text), ('assistant', answer)] if mode == 'complete' else history

    def forget_override(self, task_history):
        """Exercise neutral/affirmative deletion of app sources, never provider stores."""
        session, peer = self.session, self.peer
        selected_file = self.home / '.eevee-connections' / (self.route + '.json')
        other_route = 'openai' if self.route == 'anthropic' else 'anthropic'
        other_file = self.home / '.eevee-connections' / (other_route + '.json')
        provider_file = self.home / 'codex' / 'auth.json'
        provider_file.parent.mkdir(mode=0o700, exist_ok=True)
        # Deliberately no token fields: this credential-free provider-owned file
        # must be preserved, and cannot authorize an external request on a bug.
        provider_bytes = b'{"fixture":"provider-owned configuration, never an access token"}\n'
        provider_file.write_bytes(provider_bytes)
        provider_file.chmod(0o600)
        before_network = len(peer.snapshot())
        before_state = self.snapshot()
        self.workspace()
        open_setup(session, other_route, advanced=True)
        self.private_paste(self.secrets[0])
        fill_text(session, 'endpoint', peer.base)
        fill_text(session, 'model', MODEL)
        focus_field(session, 'storage')
        session.send(' ')  # Explicit private-file storage for this second route.
        self.capture('other-route-before-save-only')
        session.send('\x13')  # Ctrl+S applies locally, without route selection or metadata.
        until(session, other_file.is_file, 'other route private Save-only persistence')
        self.workspace()
        assert len(peer.snapshot()) == before_network, 'Save-only performed network activity'
        self.assert_owned_unchanged(before_state)
        other_bytes = other_file.read_bytes()
        selected_bytes = selected_file.read_bytes() if selected_file.exists() else None
        self.capture('other-route-save-only-no-network-or-route-switch')

        def review_forget(label):
            self.workspace()
            open_setup(session, self.route, advanced=True)
            focus_field(session, 'forget')
            wait_for(session, 'Forget app connection')
            session.send('\r')
            wait_for(session, 'Forget app connection:')
            wait_for(session, '[Back]')
            self.capture(label)

        review_forget('forget-neutral-review')
        session.send('\r')  # Neutral Back is not deletion authorization.
        wait_for(session, 'Forget cancelled.')
        assert (selected_file.read_bytes() if selected_file.exists() else None) == selected_bytes
        assert other_file.read_bytes() == other_bytes and provider_file.read_bytes() == provider_bytes
        assert len(peer.snapshot()) == before_network, 'neutral Forget performed network activity'
        session.send(ESC)
        task_history = self.send('Neutral Forget preserves this exact chat and selected connection.',
                                 task_history, 'Fixture selected override survived neutral Back.',
                                 'forget-neutral-keeps-source-and-canonical-history')
        before_network = len(peer.snapshot())
        before_state = self.snapshot()
        review_forget('forget-stale-identity-review')
        replacement_file = selected_file.with_suffix('.fixture-replacement')
        replacement_bytes = selected_bytes if selected_bytes is not None else other_bytes
        replacement_file.write_bytes(replacement_bytes)
        replacement_file.chmod(0o600)
        os.replace(replacement_file, selected_file)
        try:
            session.send('\t')
            session.send('\r')  # Consent is for the old identity, never this new leaf.
            until(session, lambda: 'changed; review' in session.display().lower(),
                  'stale Forget must refuse the replacement identity')
            assert selected_file.read_bytes() == replacement_bytes, 'stale Forget deleted replacement file'
            assert other_file.read_bytes() == other_bytes and provider_file.read_bytes() == provider_bytes
            self.assert_owned_unchanged(before_state)
            assert len(peer.snapshot()) == before_network, 'stale Forget performed network activity'
            self.capture('forget-stale-identity-refused-no-network-or-owner-change')
        finally:
            if selected_bytes is None:
                selected_file.unlink(missing_ok=True)
        self.workspace()
        task_history = self.send('Stale Forget preserves the selected source and this exact chat.',
                                 task_history, 'Fixture selected override survived stale consent.',
                                 'forget-stale-keeps-source-and-canonical-history')
        before_network = len(peer.snapshot())
        before_state = self.snapshot()
        review_forget('forget-affirmative-exact-route-review')
        session.send('\t')
        session.send('\r')  # Deliberately select the exact-route deletion effect.
        wait_for(session, 'App override forgotten')
        self.workspace()
        assert not selected_file.exists(), 'affirmative Forget kept the selected own credential file'
        assert other_file.read_bytes() == other_bytes, 'Forget deleted or changed another route'
        assert provider_file.read_bytes() == provider_bytes, 'Forget touched provider-owned configuration'
        self.assert_owned_unchanged(before_state)
        assert len(peer.snapshot()) == before_network, 'Forget performed network activity'
        self.capture('forget-selected-source-only-other-routes-provider-store-retained')
        # Both private-file and session variants must actually lose the override;
        # filesystem absence alone cannot prove session removal. There is no API
        # environment source for this route, so a new ordinary send must block.
        request = 'After Forget no old app credential may authorize this request.'
        self.write_request(request)
        session.send('\r')
        wait_for(session, 'Connections')
        assert len(peer.snapshot()) == before_network, 'forgotten override/fallback transmitted a new request'
        assert not self.snapshot()['claims'], 'missing setup created provider work ownership'
        assert any(draft['field'] == 'Request' and draft['text'] == request
                   for task in self.snapshot()['tasks']
                   for state in task['partner_drafts'].values()
                   for draft in state.get('drafts', [])), 'missing setup discarded the exact request'
        self.capture('forget-session-and-file-source-gone-actionable-setup-no-send')
        self.workspace()
        self.cases.append('forget-exact-selected-route-neutral-stale-private-and-session-removal-no-network')

    def privacy(self):
        snapshot = self.snapshot()
        assert snapshot['executions'] == 0, 'native HTTP created local command execution evidence'
        encoded = json.dumps(snapshot)
        for secret in self.secrets:
            assert secret not in self.session.text + encoded, 'credential in terminal/Request/SQLite/journal'
            for path in self.data.rglob('*'):
                if path.is_file():
                    assert secret.encode() not in path.read_bytes(), 'credential in database/WAL/data artifact: ' + path.name
        assert REASONING not in self.session.text + encoded, 'private reasoning became user-facing/durable'
        assert not list(self.project.iterdir()), 'native transport changed isolated project'
        self.cases.append('credentials-absent-terminal-request-sqlite-wal-journals-and-project')

    def close(self):
        # A privacy failure remains a failure, but synthetic secrets are still not
        # retained in terminal/event evidence or a traceback containing a bad body.
        self.session.text = self.redacted(self.session.text)
        self.session.events = json.loads(self.redacted(json.dumps(self.session.events)))
        self.session.close()

    def run(self):
        session, peer = self.session, self.peer
        wait_for(session, 'Storage: saved')
        self.capture('first-use-before')
        session.paste(FACTS[0])
        session.send('\r')
        wait_for(session, 'Connections')
        until(session, self.draft, 'first-use exact draft saved before setup')
        assert not peer.snapshot(), 'unconfigured first use reached network'
        self.capture('first-use-blocked-retained-draft')
        self.workspace()
        self.setup(secret=None if self.keyless else peer.secret, initial=True)
        assert not peer.snapshot('POST'), 'Save & choose model performed inference'
        private_file = self.home / '.eevee-connections' / (self.route + '.json')
        if self.persist:
            assert private_file.is_file() and not private_file.is_symlink(), 'explicit private save missing'
            if os.name == 'posix':
                assert stat.S_IMODE(private_file.stat().st_mode) & 0o077 == 0, 'credential file is not owner-private'
            saved = json.loads(private_file.read_text())
            assert saved['base_url'] == peer.base and saved['model'] == MODEL
            assert saved['protocol'] == self.protocol and saved['keyless'] == self.keyless
            assert saved['secret'] == (None if self.keyless else peer.secret)
        else:
            assert not private_file.exists(), 'session-only setup unexpectedly persisted credentials'
        session.pump(.3)
        assert not peer.snapshot('POST'), 'setup/model auto-resent first-use draft'
        assert self.draft(), 'setup/model destroyed exact first-use saved draft'
        self.capture('first-use-after-setup-still-unsent')
        self.cases.append('first-use-save-mask-private-or-session-save-and-choose-model-no-auto-resend')
        history = []
        for number, (fact, answer) in enumerate(zip(FACTS, ANSWERS), 1):
            history = self.send(fact, history, answer, 'ordinary-turn-' + str(number))
        # The third request is checked for ALL exact earlier roles/facts/replies,
        # not a message count or an echoed fixture response.
        assert len(history) == 6
        history = self.send('Cancel this partial turn.', history, '', 'cancel-partial-excluded', 'cancel')
        history = self.send('After cancellation retain completed facts only.', history,
                            'Fixture complete after cancellation.', 'post-cancel-canonical-content')
        history = self.send('Let this stream end incomplete.', history, '', 'incomplete-output-excluded', 'incomplete')
        history = self.send('After incomplete retain completed facts only.', history,
                            'Fixture complete after incomplete.', 'post-incomplete-canonical-content')

        # Deliberate in-app account/key + endpoint mutation invalidates only this
        # route locally; it must not replay any submitted/draft request.
        before = len(peer.snapshot('POST'))
        self.workspace()
        replacement = self.secrets[1]
        self.setup(secret=None if self.keyless else replacement,
                   base=peer.base.replace('/v1', '/changed/v1'))
        self.workspace()
        session.pump(.3)
        assert len(peer.snapshot('POST')) == before, 'explicit connection mutation replayed old request'
        history = self.send('New explicit account or endpoint, no old local chat.', [],
                            'Fixture new explicit scope.', 'explicit-account-endpoint-mutation-clears-history')
        if self.persist:
            # External change is not an explicit in-app reset. Same path, real
            # safely-owned file: resolution must detect drift before old history
            # can be transmitted. No UI notification is fabricated by harness.
            stored = json.loads(private_file.read_text())
            if self.keyless:
                stored['base_url'] = peer.base
            else:
                stored['secret'] = self.secrets[0]
            private_file.write_text(json.dumps(stored))
            private_file.chmod(0o600)
            self.write_request('Do not transmit old context across unexpected scope drift.')
            before = len(peer.snapshot('POST'))
            session.send('\r')
            session.command('approvals')
            session.send('d')
            pages = []
            for page in range(32):
                view = session.display()
                pages.append(' '.join(view.split()))
                self.capture('unexpected-scope-drift-details-' + str(page))
                if ('ownership/account/endpoint changed' in ' '.join(pages)
                        and 'chat-new' in ' '.join(pages)):
                    break
                session.send(ESC + '[6~')
            assert 'ownership/account/endpoint changed' in ' '.join(pages), \
                'unexpected scope drift did not expose its exact old-context refusal'
            assert 'chat-new' in ' '.join(pages), 'scope refusal omitted the explicit local-reset remedy'
            session.pump(.3)
            assert len(peer.snapshot('POST')) == before, 'old-account context reached changed credential scope'
            self.capture('unexpected-scope-drift-refused-before-network')
            session.command('chat-new')
            until(session, lambda: not self.snapshot()['claims'], 'explicit chat-new cancellation settlement')
            self.workspace()
            if not self.keyless:
                peer.secret = self.secrets[0]
            history = self.send('Explicit chat-new permits a fresh scope.', [],
                                'Fixture fresh scope after explicit reset.', 'scope-drift-requires-explicit-chat-new')

        # Existing party controls expose genuinely distinct canonical owners.
        self.workspace()
        session.command('party')
        session.send('a')
        until(session, lambda: len(self.snapshot()['partners']) == 2, 'second real partner creation')
        partner = self.snapshot()['partners'][-1]['id']
        self.workspace()
        session.command('lead ' + str(partner))
        until(session, lambda: self.snapshot()['tasks'][0]['lead_partner'] == partner, 'second partner selection')
        self.workspace()
        # Fresh owners switch locally without review; an existing owned response
        # requires the exact provider-switch review before any subsequent send.
        session.command('provider-' + self.route)
        if 'y Switch' in session.display():
            session.send('y')
        self.workspace()
        self.send('Independent partner fact: harbor-463.', [], 'Fixture independent partner.',
                  'partner-isolation-no-other-owner-history')
        self.workspace()
        session.command('lead 1')
        until(session, lambda: self.snapshot()['tasks'][0]['lead_partner'] == 1, 'original partner restored')
        self.workspace()
        history = self.send('Original owner resumes only its own chat.', history,
                            'Fixture original owner restored.', 'partner-return-preserves-own-exact-history')
        self.workspace()
        session.command('task-new')
        self.workspace()
        session.command('provider-' + self.route)
        if 'y Switch' in session.display():
            session.send('y')
        self.workspace()
        task_history = self.send('Independent task fact: meadow-587.', [], 'Fixture independent task.',
                                 'task-isolation-no-prior-task-history')
        self.forget_override(task_history)
        self.privacy()
        peer.assert_clean()
        self.workspace()
        session.command('quit')
        # Cancellation intentionally records unknown remote outcome; acknowledge
        # only that necessary exit review, not another inference approval.
        confirm_exit(session, owned_work=True)
        assert session.wait_exit() == 0
        self.cases.append('bounded-clean-exit-terminal-modes-restored-no-provider-programs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, help='Already-built native Eevee executable')
    parser.add_argument('--output', required=True, help='Retained screenshots/events/payload assertions/results directory')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'binary': str(binary), 'binary_sha256': None,
              'cases': [], 'limits': LIMITS, 'failures': []}
    try:
        result['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
        for size, ascii_mode in (((120, 36), False), ((60, 18), True)):
            for route, protocol, keyless, persist in MATRIX:
                name = f'{size[0]}x{size[1]}-{route}-{protocol}'
                case_output = output / name
                case_output.mkdir(parents=True, exist_ok=True)
                case = {'name': name, 'size': list(size), 'glyphs': 'ascii' if ascii_mode else 'unicode',
                        'route': route, 'protocol': protocol, 'keyless': keyless,
                        'storage': 'private-local-plaintext' if persist else 'session',
                        'status': 'failed', 'cases': [], 'limits': LIMITS,
                        'binary_sha256': result['binary_sha256']}
                journey, peer = None, None
                try:
                    with tempfile.TemporaryDirectory(prefix='ev-native-fixture-') as temporary:
                        with Peer(protocol, 'sk-fixture-native-initial-3a92e71c', keyless) as peer:
                            try:
                                journey = Journey(binary, case_output, Path(temporary), size, ascii_mode,
                                                  route, protocol, keyless, persist, peer)
                                journey.run()
                                case['status'] = 'passed-credential-free-runtime'
                            finally:
                                if journey is not None:
                                    try:
                                        journey.capture('final-screen')
                                    finally:
                                        journey.close()
                except BaseException:
                    case['status'] = 'failed'
                    failure = traceback.format_exc()
                    if journey is not None:
                        failure = journey.redacted(failure)
                    case['failure'] = failure
                    result['failures'].append({'name': name, 'failure': failure})
                finally:
                    if journey is not None:
                        case['cases'] = journey.cases
                        save_json(case_output / 'payload-assertions.json', journey.assertions)
                    if peer is not None:
                        retained = json.dumps(peer.snapshot(), ensure_ascii=False)
                        if journey is not None:
                            retained = journey.redacted(retained)
                        save_json(case_output / 'http-requests.json', json.loads(retained))
                        case['peer_errors'] = peer.errors
                    save_json(case_output / 'results.json', case)
                    result['cases'].append(case)
        assert not result['failures'], 'native runtime cases failed; all failures retained in --output'
        result['status'] = 'passed-credential-free-runtime-with-real-account-blockers'
    except BaseException:
        result['failure'] = traceback.format_exc()
        raise
    finally:
        save_json(output / 'results.json', result)


if __name__ == '__main__':
    main()
