#!/usr/bin/env python3
"""Opt-in bounded actual-binary route/model journey conformance; no live provider.

Every selectable route, native protocol, two contrasting advertised models and an
unknown exact model run at 120x36 and 60x18 with English/Korean requests. Controlled
peers perform actual source reads through production boundaries; no echo oracle.
Account HTTP stays fixed official: its UI/no-dispatch row is composed with the
private-boundary Rust account wire regression (see --account-wire-proof).
"""
from __future__ import annotations
import argparse
from contextlib import closing, nullcontext
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import sqlite3
import sys
import tempfile
import time
import traceback

import file_creation_smoke as files
from agent_coding_smoke import decode_result
from native_connection_smoke import Peer, until
from response_smoke import SurfaceSession, SURFACE
from terminal_smoke import terminal_environment, permission_run_available, inspect_permission_scope, confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
SOURCE = 'JOURNEY_SOURCE_한글界e\u0301\n'
STREAMING = 'GP10_STREAMING source inspection pending'
MODELS = ('journey-rich', 'journey-fixed', 'journey-unknown')
STATES = ('Connect', 'Model / effort', 'Write Request', 'Preview', 'Explicit send', 'Stream',
          'File / command approvals', 'Todo progress', 'Follow-up / continue', 'Cancel',
          'Resume / new session', 'Handoff', 'Receipt')
NATIVE = (('openai', 'responses'), ('anthropic', 'messages'), ('google', 'gemini'),
          ('compatible', 'responses'), ('compatible', 'chat_completions'))


def save(path, value):
    files.save(path, value)


def cell_words(value):
    # Preserve raw captures; ignore only frame cells when matching wrapped UI words.
    return ' '.join(value.translate(str.maketrans({char: ' ' for char in '│║┃|'})).split()).lower()


def protocol_peer(route):
    """Real JSONL transport, correlated source callback and terminal/cancel frames."""
    if '--version' in sys.argv or 'version' in sys.argv:
        print('devin 3000.11.3 (012345abcdef)' if route == 'devin' else 'codex-cli 0.152.1' if route == 'codex' else '1.18.0')
        return
    log = Path(os.environ['EEVEE_JOURNEY_LOG'])
    def record(kind, **fields):
        with log.open('a') as stream:
            stream.write(json.dumps(dict(kind=kind, **fields), ensure_ascii=False) + '\n')
    def emit(value):
        if route != 'codex':
            value = dict(jsonrpc='2.0', **value)
        record('wire', direction='peer', packet=value)
        print(json.dumps(value, ensure_ascii=False), flush=True)
    session, turn, waiting, prompt_id, hold = 'journey-owned-session', 'journey-turn', False, None, False
    project = Path.cwd()
    def stream_gate():
        release = Path(os.environ['EEVEE_JOURNEY_RELEASE'])
        if release.exists():
            return
        if route == 'codex':
            emit(dict(method='item/started', params=dict(threadId=session, turnId=turn, item=dict(id='streaming', type='agentMessage', text=''))))
            emit(dict(method='item/agentMessage/delta', params=dict(threadId=session, turnId=turn, itemId='streaming', delta=STREAMING)))
        else:
            emit(dict(method='session/update', params=dict(sessionId=session, update=dict(sessionUpdate='agent_message_chunk', content=dict(type='text', text=STREAMING)))))
        deadline = time.monotonic() + 60
        while not release.exists():
            assert time.monotonic() < deadline, 'actual streaming surface was not observed'
            time.sleep(.025)
    selected = os.environ.get('EEVEE_JOURNEY_MODEL', MODELS[0])
    thought = 'low'
    def acp_configuration():
        if selected != MODELS[0]:
            return dict(models=dict(currentModelId=selected,
                availableModels=[dict(modelId=model, name=model) for model in MODELS[:2]]))
        return dict(configOptions=[
            dict(id='model', name='Model', description='Controlled model selector', category='model',
                type='select', currentValue=selected,
                options=[dict(value=model, name=model, description='Controlled exact model') for model in MODELS[:2]]),
            dict(id='thought', name='Reasoning', description='Controlled rich-model effort only', category='thought_level',
                type='select', currentValue=thought,
                options=[dict(value=value, name=value, description='Controlled reasoning tier') for value in ('low', 'high', 'xhigh')]),
        ])
    for line in sys.stdin:
        packet = json.loads(line)
        record('wire', direction='client', packet=packet)
        method, identity, params = packet.get('method'), packet.get('id'), packet.get('params', {})
        if method is None and waiting and identity == 'source-read':
            value = decode_result(packet['result']) if route == 'codex' else packet['result']
            assert value['content'] == SOURCE, packet
            record('actual_read', result=packet)
            waiting = False
            if hold:
                continue
            if route == 'codex':
                emit(dict(method='item/completed', params=dict(threadId=session, turnId=turn, item=dict(id='answer', type='agentMessage', text='Controlled actual source read completed.'))))
                emit(dict(method='turn/completed', params=dict(threadId=session, turnId=turn, turn=dict(id=turn, status='completed', items=[]))))
            else:
                emit(dict(method='session/update', params=dict(sessionId=session, update=dict(sessionUpdate='agent_message_chunk', content=dict(type='text', text='Controlled actual source read completed.')))))
                emit(dict(id=prompt_id, result=dict(stopReason='end_turn')))
            record('finished')
            continue
        if method in ('turn/interrupt', 'session/cancel'):
            if identity is not None:
                emit(dict(id=identity, result={}))
            if route == 'codex':
                emit(dict(method='turn/completed', params=dict(threadId=session, turnId=turn, turn=dict(id=turn, status='interrupted', items=[]))))
            else:
                emit(dict(id=prompt_id, result=dict(stopReason='cancelled')))
            record('cancelled')
            continue
        if identity is None:
            continue
        if method == 'initialize':
            result = dict(userAgent='controlled-journey') if route == 'codex' else dict(protocolVersion=1, agentCapabilities=dict(loadSession=True))
        elif method == 'account/read':
            result = dict(account=None, requiresOpenaiAuth=False)
        elif method == 'config/read':
            result = dict(config=dict(model=selected))
        elif method == 'model/list':
            result = dict(data=[dict(model=model, displayName=model, isDefault=index == 0,
                supportedReasoningEfforts=[dict(reasoningEffort=e) for e in (['low', 'high', 'xhigh'] if index == 0 else [])])
                for index, model in enumerate(MODELS[:2])], nextCursor=None)
        elif method in ('thread/start', 'thread/resume'):
            project = Path(params['cwd'])
            selected = params.get('model', selected)
            result = dict(thread=dict(id=session), cwd=params['cwd'], model=selected,
                          reasoningEffort=params.get('config', {}).get('model_reasoning_effort'),
                          approvalPolicy='on-request', approvalsReviewer='user', sandbox=dict(type='readOnly', networkAccess=False))
        elif method in ('session/new', 'session/load'):
            project = Path(params['cwd'])
            result = dict(sessionId=session, **acp_configuration())
        elif method == 'session/set_model':
            selected = params['modelId']
            result = acp_configuration()
        elif method == 'session/set_config_option':
            if params['configId'] == 'model':
                selected = params['value']
            else:
                assert params['configId'] == 'thought' and params['value'] in ('low', 'high', 'xhigh'), packet
                thought = params['value']
            result = acp_configuration()
        elif method in ('turn/start', 'session/prompt'):
            text = '\n'.join(item.get('text', '') for item in params.get('input', params.get('prompt', [])))
            hold = 'GP10_HOLD' in json.loads(text)['request']
            record('submitted', model=selected, text=text)
            prompt_id, waiting = identity, True
            if route == 'codex':
                emit(dict(id=identity, result=dict(turn=dict(id=turn, status='inProgress', items=[]))))
                stream_gate()
                if selected == MODELS[0]:
                    emit(dict(method='turn/plan/updated', params=dict(threadId=session, turnId=turn, plan=[dict(step='Read source / 한글', status='in_progress')])))
                emit(dict(id='source-read', method='item/tool/call', params=dict(threadId=session, turnId=turn, itemId='source', callId='source', tool='eevee_read_file', arguments=dict(path='source.txt', start_line=1, end_line=0))))
            else:
                stream_gate()
                if selected == MODELS[0]:
                    emit(dict(method='session/update', params=dict(sessionId=session, update=dict(sessionUpdate='plan', entries=[dict(content='Read source / 한글', status='in_progress', priority='medium')]))))
                emit(dict(id='source-read', method='fs/read_text_file', params=dict(sessionId=session, path=str(project / 'source.txt'))))
            continue
        else:
            emit(dict(id=identity, error=dict(code=-32601, message='Unsupported controlled peer method ' + str(method))))
            continue
        emit(dict(id=identity, result=result))


class Journey:
    def __init__(self, session, data, output):
        self.session, self.data, self.output = session, data, output
        self.states = []
        self.observations = []
        self.account_preflight = None

    def workspace(self):
        for _ in range(20):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if 'Alt+A' in footer or footer.startswith(tuple(SURFACE[key] for key in ('home', 'edit', 'draft'))):
                return footer
            self.session.send(ESC)
        raise AssertionError('workspace unavailable\n' + self.session.display())

    def db(self, query):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            return db.execute(query).fetchall()

    def claims(self):
        return self.db('SELECT partner FROM active_work')

    def drafts(self):
        return [json.loads(row[0]) for row in self.db('SELECT partner_drafts FROM tasks')]

    def choose_auto(self, model):
        self.model = model
        # Reasoning owns Enter/Ctrl+K: navigate its real selectable row.
        self.session.send(ESC + '[H')
        for _ in range(34):
            if any('> auto' in ' '.join(line.split()) for line in self.session.display().splitlines()):
                self.state('Model / effort')
                self.session.send('\r')
                break
            self.session.send(ESC + '[B')
        else:
            raise AssertionError('selectable auto row missing\n' + self.session.display())
        def committed():
            rows = self.db('SELECT record FROM provider_preferences WHERE id=1')
            return bool(rows) and any(selection.get('model') == model and selection.get('effort') == {'mode': 'auto'}
                for selection in json.loads(rows[0][0]).get('selections', {}).values())
        until(self.session, committed, 'exact model/auto choice did not persist', timeout=40)

    def state(self, name, unavailable=None):
        # Validate cells from the invoked live surface, not the static journey.
        groups = {
            'Connect': [('Connections',), ('Enter',), ('Esc',)],
            'Model / effort': [('Reasoning',), ('> auto',), ('Enter',), ('Esc',)],
            'Write Request': [('Request',), ('Read source.txt',), ('Ctrl+S',)],
            'Preview': [('Next evolution',), ('Appearance:',), ('Effective operation:',), ('Effort:',), ('Todo step:',), ('Context:',), ('Model capabilities:',), ('Esc',)],
            'Explicit send': [('Request',), ('Enter Send',)],
            'Stream': [('GP10_STREAMING',), ('/cancel',)],
            'File / command approvals': [('No pending approvals or questions', 'Permission required', 'one-shot execution'), ('Enter', 'a/y')],
            'Todo progress': [('Todo progress',), ('Provider status',), ('edit',), ('adopt',), ('Esc',)],
            'Follow-up / continue': [('Request',), ('Continue explaining',), ('Ctrl+S',)],
            'Cancel': [('cancel', 'interrupt', 'unknown', 'Local work idle'), ('Enter Send', '/response', 'Esc')],
            'Resume / new session': [('Route:',), ('Session:',), ('Cancel',), ('Details',), ('Esc',)],
            'Handoff': [('Portable task handoff',), ('export-json',), ('import-handoff',), ('Esc',)],
            'Receipt': [('Outgoing receipts',), ('Route:',), ('Model:',), ('Effort:',), ('Todo:',), ('Usage:',), ('Outcome:',), ('s source',), ('p purge',), ('r retry',), ('Esc',)],
        }[name]
        if name == 'Preview' and self.model == MODELS[2]:
            groups.append(('unknown model capabilities',))
        if unavailable:
            assert self.account_preflight and not self.claims(), 'unavailable proof must follow actual account preflight refusal'
            if name == 'Stream':
                groups = [('Request', 'Response', 'connection'), ('Esc', 'Enter', '/response')]
            elif name == 'Receipt':
                groups = [('Outgoing receipts',), ('No retained outgoing receipts',), ('Esc',)]
            elif name == 'Resume / new session':
                groups = [('Request',), ('Continue explaining',), ('Enter Send',)]
            elif name == 'Cancel':
                groups = [('Cancel ALL owned work', 'Nothing to cancel'), ('Stay', 'Request'), ('Esc',)]
            else:
                raise AssertionError('unsupported state exception: ' + name)
        def ready():
            if name == 'Stream' and 'Evolution!' in self.session.display() and 'Keep typing' in self.session.display():
                self.session.send(ESC + '[D')
            if name == 'Resume / new session' and not unavailable and 'Enter to review (not approve)' in self.session.display():
                self.session.send('\r')  # Engage inspection only; no affirmative choice.
            visible = cell_words(self.session.display())
            count = 1 if name == 'Resume / new session' and not unavailable else 2
            return all(any(cell_words(marker) in visible for marker in group) for group in groups[:count])
        until(self.session, ready, 'actual ' + name + ' surface did not appear', timeout=40)
        views = [self.session.display()]
        if name in ('Preview', 'Todo progress', 'Resume / new session', 'Receipt'):
            for _ in range(8):
                self.session.send(ESC + '[6~')
                current = self.session.display()
                if current == views[-1]:
                    break
                views.append(current)
        observed = '\n'.join(views)
        matches = []
        for alternatives in groups:
            found = next((marker for marker in alternatives if cell_words(marker) in cell_words(observed)), None)
            assert found is not None, (name, alternatives, observed)
            matches.append(found)
        if name == 'Stream' and not unavailable:
            assert self.claims(), 'stream must retain actual owned work'
        if name == 'Cancel':
            assert not self.claims(), 'cancel must settle its actual owned work'
        self.states.append(name)
        self.observations.append(dict(state=name, observed_markers=matches,
            availability='not exercised: ' + unavailable if unavailable else 'observed',
            key_hints=[marker for marker in matches if marker in ('Enter', 'Esc', 'Ctrl+S', 'Enter Send', '/cancel', 'edit', 'adopt', 'Cancel', 'Details', 's source', 'p purge', 'r retry')],
            cells_sha256=hashlib.sha256(observed.encode()).hexdigest(), claims=len(self.claims())))
        save(self.output / 'observed-states.json', self.observations)
        self.session.capture(f'{len(self.states):02}-{name.replace(" / ", "-").replace(" ", "-")}')

    def request(self, text):
        footer = self.workspace()
        if footer.startswith(SURFACE['home']):
            self.session.send('\r')
        self.session.send((ESC + '[A') * 50 + ESC + '[H' + (ESC + '[3~') * 512)
        self.session.paste(text)
        self.session.send('\x13')
        until(self.session, lambda: text in json.dumps(self.drafts(), ensure_ascii=False), 'literal Unicode request did not save', timeout=40)

    def approve_until(self, predicate):
        approved = False
        def check():
            nonlocal approved
            if predicate():
                return True
            if not approved and permission_run_available(self.session):
                self.session.send('y', settle=False)
                approved = True
            elif not approved and inspect_permission_scope(self.session):
                pass
            elif 'Enter to review (not approve)' in self.session.display():
                self.session.send('\r')
            elif 'Alt+A' in self.session.display():
                self.session.send(ESC + 'a')
            return False
        until(self.session, check, 'controlled submission did not reach real boundary', timeout=75)

    def read_until(self, predicate, source, observe_approval=False):
        """Approve only the exact, fully exposed SDK Read input, once per turn."""
        views, pending, answered = [], False, False
        def check():
            nonlocal pending, answered
            if predicate():
                return True
            view = self.session.display()
            if not answered and (pending or 'Claude approval request:' in view):
                pending = True
                views.append(view)
                if observe_approval and 'File / command approvals' not in self.states:
                    self.state('File / command approvals')
                if 'Enter to review (not approve)' in view:
                    self.session.send('\r')
                elif 'a Allow once' in view:
                    compact = ''.join('\n'.join(views).translate(str.maketrans('', '', '│║┃|')).split())
                    assert '"tool":"Read"' in compact and str(source) in compact, 'not the exact source Read permission'
                    assert '"suppressAlwaysAllowRule":true' in compact, 'one-shot-only policy missing'
                    save(self.output / ('source-read-permission-' + str(len(self.db('SELECT record FROM outgoing_receipts'))) + '.json'),
                        dict(cells=views, tool='Read', path=str(source), decision='allow once'))
                    self.session.send(ESC + '[C')
                    self.session.capture('source-read-allow-once-selected')
                    self.session.send('\r', settle=False)
                    answered = True
                else:
                    self.session.send(ESC + '[6~')
            return False
        until(self.session, check, 'actual source Read/one-shot permission did not settle', timeout=75)


def row(binary, node, sidecar, output, route, protocol, model, size, account_wire_proof):
    output.mkdir(parents=True)
    result = dict(status='failed', route=route, protocol=protocol, model=model, size=list(size))
    session = None
    with tempfile.TemporaryDirectory(prefix='eevee-journey-한글-') as temporary:
        root = Path(temporary).resolve()
        project, home, runtime, data = (root / name for name in ('project', 'home', 'runtime', 'data'))
        for directory in (project, home, runtime):
            directory.mkdir()
        (project / 'source.txt').write_text(SOURCE)
        log = root / 'peer.jsonl'
        log.touch()
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'EEVEE_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_', 'DEVIN_', 'AWS_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
            XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'), CODEX_HOME=str(home / 'codex'),
            PATH=str(runtime) + os.pathsep + environment.get('PATH', ''), EV_PROVIDER=route,
            EEVEE_JOURNEY_MODEL=model,
            EEVEE_JOURNEY_LOG=str(log), EEVEE_JOURNEY_RELEASE=str(root / 'stream-release'), NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost')
        native = (route, protocol) in NATIVE
        context = Peer(protocol, 'controlled-journey-unused-key', route == 'compatible', retain_body_bytes=True) if native else nullcontext(None)
        with context as peer:
            if native:
                peer.expected_model = model
                prefix = dict(openai='OPENAI', anthropic='ANTHROPIC', google='GEMINI', compatible='EV_API')[route]
                environment.update({prefix + '_BASE_URL': peer.base, prefix + '_MODEL': model,
                    ('EV_API_KEY' if route == 'compatible' else prefix + '_API_KEY'): peer.secret})
                if route == 'compatible':
                    environment.update(EV_API_PROTOCOL=protocol, EV_API_KEYLESS='1')
                metadata = [dict(id=MODEL_ID, name=MODEL_ID, supported_reasoning_levels=[dict(effort=e) for e in (['low', 'high', 'xhigh'] if MODEL_ID == MODELS[0] else [])]) for MODEL_ID in MODELS[:2]]
                peer.catalogue = dict(models=[dict(name=m['id'], displayName=m['id'], supportedGenerationMethods=['generateContent'], **{k:v for k,v in m.items() if k not in ('id','name')}) for m in metadata]) if protocol == 'gemini' else dict(data=metadata, has_more=False)
            elif route == 'claude':
                package = root / 'sidecar'
                shutil.copytree(sidecar, package, ignore=shutil.ignore_patterns('.git', '.DS_Store'))
                sdk = package / 'node_modules/@anthropic-ai/claude-agent-sdk'
                shutil.copyfile(sdk / 'sdk.mjs', sdk / 'sdk-real.mjs')
                shutil.copyfile(Path(__file__).with_name('journey_sdk_peer.mjs'), sdk / 'sdk.mjs')
                environment.update(ANTHROPIC_API_KEY='controlled-not-real', EEVEE_CLAUDE_NODE=str(node), EEVEE_CLAUDE_SIDECAR=str(package / 'main.ts'))
            elif route != 'chatgpt':
                wrapper = runtime / ('codex' if route == 'codex' else route)
                wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' ' + shlex.quote(str(Path(__file__).resolve())) + ' --peer ' + route + ' "$@"\n')
                wrapper.chmod(0o700)
                environment['CODEX_API_KEY'] = 'controlled-not-real'
            def rows():
                return [json.loads(line) for line in log.read_text().splitlines() if line.strip()]
            def submitted():
                return len(peer.snapshot('POST')) if native else sum(item['kind'] == 'submitted' for item in rows())
            try:
                session = SurfaceSession([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono', *(['--ascii'] if size[0] == 60 else [])], project, environment, size, output / 'tui')
                app = Journey(session, data, output)
                wait_for(session, 'Storage: saved', timeout=40)
                session.command('journey')
                wait_for(session, 'Request journey')
                pages = ''
                for _ in range(8):
                    pages += session.display() + '\n'
                    session.send(ESC + '[6~')
                for name in STATES:
                    assert name in pages, (name, pages)
                save(output / 'journey-pages.json', dict(states=list(STATES), rendered=pages))
                app.workspace(); session.command('connect'); app.state('Connect'); app.workspace()
                if route != 'chatgpt':
                    session.command('models-refresh')
                    def metadata_seen():
                        return bool(peer.snapshot('GET')) if native else any(
                            item.get('kind') == 'metadata' or item.get('packet', {}).get('method') in ('model/list', 'session/new')
                            for item in rows())
                    app.approve_until(metadata_seen)
                    until(session, lambda: not app.claims(), 'metadata inspection did not settle', timeout=75)
                    app.workspace()
                session.command('model ' + model)
                wait_for(session, 'Reasoning')
                app.choose_auto(model); app.workspace()
                request = 'Read source.txt without changing files. 파일을 읽고 설명해줘. 界e\u0301'
                app.request(request); app.state('Write Request')
                drafts = app.drafts()
                session.command('evolution-preview'); wait_for(session, 'Next evolution'); app.state('Preview')
                assert drafts == app.drafts() and submitted() == 0
                app.workspace()
                if route == 'chatgpt':
                    app.state('Explicit send'); session.send('\r')
                    until(session, lambda: any(marker in session.display() for marker in (
                        'OpenAI account connection needs setup', 'No native account source selected',
                        'Choose a connection method; no task is sent.')),
                        'actual account preflight setup refusal missing', timeout=40)
                    app.account_preflight = session.display()
                    save(output / 'account-preflight.json', dict(cells=app.account_preflight,
                        submitted=submitted(), claims=app.claims(), fixed_endpoint_override=False))
                    assert submitted() == 0 and not app.claims()
                    app.workspace(); session.command('cancel'); app.workspace()
                    session.command('response'); app.state('Stream', unavailable='account connection refused before any transport submission'); app.workspace()
                    session.command('approvals'); app.state('File / command approvals'); app.workspace()
                    session.command('todo'); app.state('Todo progress'); app.workspace()
                    app.request('Continue explaining. 계속 설명해줘.'); app.state('Follow-up / continue')
                    app.workspace(); session.command('cancel'); app.state('Cancel', unavailable='no account request was dispatched; no owned transport to cancel'); app.workspace()
                    session.command('chat-new'); app.state('Resume / new session', unavailable='no connected account or provider-owned session'); app.workspace()
                    session.command('handoff'); app.state('Handoff'); app.workspace()
                    session.command('receipts'); app.state('Receipt', unavailable='no account request was dispatched')
                    assert app.states == list(STATES), app.states
                    result.update(states=app.states, account_wire_proof=str(account_wire_proof), limits='No account network/auth; fixed official endpoint unchanged. Rust private-boundary account wire evidence is separate.')
                else:
                    gate = files.Gate()
                    stream_gate = files.Gate()
                    read_results = []
                    def queue_native(hold=False):
                        if not native:
                            return
                        calls = [dict(id='read-source', name='eevee_read_file', arguments=dict(path='source.txt', start_line=1, end_line=0))]
                        if model == MODELS[0]:
                            calls.insert(0, dict(id='todo-init', name='todo', arguments=dict(operation='init', label='', steps=[dict(label='Read source / 한글', phase='investigate', status='active')])))
                        peer.plans.put(files.response_plan(peer, calls=calls, model=model,
                            text=STREAMING if not (root / 'stream-release').exists() else '',
                            gate=stream_gate if not (root / 'stream-release').exists() else None))
                        terminal = files.response_plan(peer, text='Controlled actual source read completed.', gate=gate if hold else None, model=model)
                        def checked(handler, event, packet):
                            results = files.tool_results(protocol, packet['body'])
                            value = results.get('read-source', results.get('eevee_read_file'))
                            value = json.loads(value) if isinstance(value, str) else value
                            if protocol == 'gemini':
                                value = value['output']  # Actual Gemini functionResponse envelope.
                            assert value['content'] == SOURCE, packet
                            read_results.append(value)
                            return terminal[1](handler, event, packet)
                        peer.plans.put(('custom', checked))
                    queue_native()
                    app.state('Explicit send'); session.send('\r', settle=False)
                    app.approve_until(lambda: submitted() > 0)
                    app.state('Stream')
                    stream_gate.release.set()
                    (root / 'stream-release').touch()
                    app.read_until(lambda: not app.claims() and ('Storage: saved' in session.display()), project / 'source.txt', observe_approval=True)
                    assert len(read_results) == 1 if native else sum(item['kind'] == 'actual_read' for item in rows()) == 1
                    app.workspace()
                    if 'File / command approvals' not in app.states:
                        session.command('approvals'); app.state('File / command approvals'); app.workspace()
                    session.command('todo'); app.state('Todo progress'); app.workspace()
                    progress = json.loads(app.db('SELECT workflows FROM tasks')[0][0]).get('progress')
                    if model == MODELS[0]:
                        assert progress and any(step['label'] == 'Read source / 한글' for step in progress['steps']), progress
                        assert progress['adopted_revision'] != progress['revision'], 'provider proposal silently adopted'
                    else:
                        assert not progress or not progress['steps'], 'invented todo source'
                    result['actual_progress'] = progress
                    app.request('Continue explaining. 계속 설명해줘.'); app.state('Follow-up / continue')
                    queue_native(); before = submitted(); session.send('\r', settle=False)
                    app.approve_until(lambda: submitted() > before)
                    app.read_until(lambda: not app.claims(), project / 'source.txt')
                    assert len(read_results) == 2 if native else sum(item['kind'] == 'actual_read' for item in rows()) == 2
                    app.request('Read source.txt without changing files. GP10_HOLD 취소 확인')
                    reads_before = sum(item['kind'] == 'actual_read' for item in rows())
                    queue_native(hold=True); before = submitted(); session.send('\r', settle=False)
                    app.approve_until(lambda: submitted() > before)
                    app.read_until(lambda: gate.ready.is_set() if native else sum(item['kind'] == 'actual_read' for item in rows()) > reads_before, project / 'source.txt')
                    assert app.claims(), 'held request completed before cancellation'
                    assert len(read_results) == 3 if native else sum(item['kind'] == 'actual_read' for item in rows()) == 3
                    app.workspace(); session.command('cancel')
                    gate.release.set()
                    until(session, lambda: not app.claims(), 'actual cancellation did not release ownership', timeout=75)
                    if route == 'claude':
                        finished = [event for event in rows() if event['kind'] == 'finished']
                        assert len(finished) == 3 and finished[-1]['interrupted'], 'SDK interrupt was not actually consumed'
                    app.state('Cancel')
                    app.workspace()
                    session.command('session-new')
                    def new_review_ready():
                        view = session.display()
                        if any(label in view for label in ('LOCAL PREVIEW', 'Run needs the complete scope', 'Enter to review (not approve)', '· y Run / n Cancel')):
                            return True
                        if 'Alt+A' in view:
                            session.command('approvals')
                        return False
                    until(session, new_review_ready, 'explicit new-session scope never became reviewable', timeout=75)
                    app.state('Resume / new session')
                    session.command('cancel')
                    until(session, lambda: not app.claims() and 'Storage: saved' in session.display(), 'new-session preparation did not cancel', timeout=75)
                    app.workspace()
                    session.command('handoff'); app.state('Handoff'); app.workspace()
                    session.command('receipts'); app.state('Receipt')
                    records = [json.loads(value[0]) for value in app.db('SELECT record FROM outgoing_receipts ORDER BY rowid')]
                    assert len(records) == 3, records
                    assert all(record['selection']['model'] == model and record['selection']['effort'] == {'mode': 'auto'} for record in records), records
                    assert [*dict.fromkeys(app.states)] == list(STATES), app.states
                    result.update(states=app.states, receipt_fields=sorted(records[0]), actual_source_sha256=hashlib.sha256((project / 'source.txt').read_bytes()).hexdigest())
                    save(output / 'receipts.json', records)
                save(output / 'wire.json', peer.snapshot() if native else rows())
                app.workspace(); session.command('quit')
                if session.alive():
                    session.capture('exit-review')
                confirm_exit(session, owned_work=True)
                assert session.wait_exit() == 0
                if native:
                    assert len(peer.snapshot('POST')) == 6, 'hidden or missing native request round'
                    assert all(packet['body'].get('model', model) == model for packet in peer.snapshot('POST')), 'requested native identity changed'
                    assert peer.plans.empty(), 'planned tool/read round was never consumed'
                elif route != 'chatgpt':
                    assert submitted() == 3, 'hidden or missing explicit agent request'
                    assert all(packet['model'] == model for packet in rows() if packet['kind'] == 'submitted'), 'requested agent identity changed'
                result['observations'] = app.observations
                result['status'] = 'composed' if route == 'chatgpt' else 'passed'
            except Exception:
                result['status'] = 'failed'
                result['error'] = traceback.format_exc()
            finally:
                save(output / 'wire.json', peer.snapshot() if native else rows())
                if native:
                    save(output / 'peer-errors.json', peer.errors)
                if result['status'] == 'failed' and session is not None:
                    session.capture('failure')
                    try:
                        save(output / 'failure-state.json', dict(
                            tasks=app.db('SELECT partner_drafts,criteria,workflows FROM tasks'),
                            claims=app.db('SELECT * FROM active_work'),
                            receipts=app.db('SELECT record FROM outgoing_receipts ORDER BY rowid')))
                    except Exception:
                        result['evidence_error'] = traceback.format_exc()
                if 'gate' in locals():
                    if 'stream_gate' in locals():
                        stream_gate.release.set()
                    gate.release.set()
                if session is not None:
                    try:
                        session.close()
                    except Exception:
                        result['status'] = 'failed'
                        result['close_error'] = traceback.format_exc()
                save(output / 'results.json', result)
    return result


def main():
    if '--peer' in sys.argv:
        protocol_peer(sys.argv[sys.argv.index('--peer') + 1]); return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--node', type=Path, default=shutil.which('node'))
    parser.add_argument('--sidecar', type=Path, default=Path(__file__).resolve().parents[1] / 'sidecar')
    parser.add_argument('--account-wire-proof', type=Path, required=True, help='retained output from cargo test gp10_account_wire_matrix -- --nocapture')
    parser.add_argument('--route', choices=[r for r, _ in NATIVE] + ['codex', 'claude', 'opencode', 'gemini', 'devin', 'chatgpt'])
    parser.add_argument('--model', choices=MODELS, help='Bounded diagnostic row; default runs every model')
    parser.add_argument('--size', choices=('120x36', '60x18'), help='Bounded diagnostic geometry; default runs both')
    parser.add_argument('--merge', type=Path, nargs='+', metavar='MATRIX', help='Validate and consolidate distinct frozen-binary route manifests into the complete 66-row matrix')
    args = parser.parse_args()
    for path in (args.binary, args.account_wire_proof):
        if not path.is_file():
            parser.error('required existing input missing: ' + str(path))
    proof = args.account_wire_proof.read_text()
    if any(proof.count('GP10_ACCOUNT_WIRE model=' + model + ' ') != 1 for model in MODELS) or 'test result: ok.' not in proof:
        parser.error('account wire proof must contain three distinct model markers and a passing test result; UI geometry is proved by the six account PTY rows')
    variants = [*NATIVE, *((route, None) for route in ('codex', 'claude', 'opencode', 'gemini', 'devin', 'chatgpt'))]
    if args.merge and any((args.route, args.model, args.size)):
        parser.error('--merge requires complete coverage, not diagnostic filters')
    if args.route:
        variants = [(route, protocol) for route, protocol in variants if route == args.route]
    if not args.merge and any(route == 'claude' for route, _ in variants) and (args.node is None or not (args.sidecar / 'node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs').is_file()):
        parser.error('full matrix requires Node >=24 and installed sidecar dependencies')
    binary_sha256 = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    args.output.mkdir(parents=True, exist_ok=False)
    if args.merge:
        results = []
        for path in args.merge:
            manifest = json.loads(path.read_text())
            if manifest['binary_sha256'] != binary_sha256:
                parser.error('superseded or different binary in ' + str(path))
            for original in manifest['rows']:
                result = dict(original)
                identity = f'{result["route"]}-{result["protocol"] or "controlled"}-{result["model"]}-{result["size"][0]}'
                result['evidence_root'] = str((path.parent / identity).resolve())
                results.append(result)
        observed = [(result['route'], result['protocol'], result['model'], tuple(result['size'])) for result in results]
        expected = {(route, protocol, model, size) for route, protocol in variants for model in MODELS for size in ((120, 36), (60, 18))}
        if len(observed) != len(expected) or set(observed) != expected:
            parser.error('merged matrix must contain each of the 66 distinct route/protocol/model/geometry rows exactly once')
    else:
        results = [row(args.binary.resolve(), Path(args.node).resolve() if args.node else None, args.sidecar.resolve(),
            args.output / f'{route}-{protocol or "controlled"}-{model}-{size[0]}', route, protocol, model, size, args.account_wire_proof.resolve())
            for route, protocol in variants for model in ([args.model] if args.model else MODELS)
            for size in ([tuple(map(int, args.size.split('x')))] if args.size else ((120, 36), (60, 18)))]
    for result in results:
        if result['status'] in ('passed', 'composed'):
            if any(result.get(key) for key in ('error', 'close_error', 'evidence_error')) or result['states'] != list(STATES) or [entry['state'] for entry in result['observations']] != list(STATES):
                result.update(status='failed', error='Incomplete observations or retained runtime error; cannot count as successful conformance')
    successful = [result for result in results if result['status'] == 'passed']
    if successful:
        baseline = successful[0]['receipt_fields']
        baseline_keys = [(observation['state'], observation['key_hints']) for observation in successful[0]['observations']]
        for result in successful:
            observed_keys = [(observation['state'], observation['key_hints']) for observation in result['observations']]
            if result['receipt_fields'] != baseline or observed_keys != baseline_keys:
                result.update(status='failed', error='Actual receipt field names or observed journey key hints differ from canonical matrix baseline')
                if not args.merge:
                    save(args.output / f'{result["route"]}-{result["protocol"] or "controlled"}-{result["model"]}-{result["size"][0]}' / 'results.json', result)
    save(args.output / 'matrix.json', dict(binary_sha256=binary_sha256, rows=results,
        limits=['Controlled protocol/UI/source-read evidence; no live model interpretation, usage or entitlement claimed.', 'Account UI/no-dispatch composed with private-boundary wire proof; no production endpoint override.']))
    return int(any(result['status'] not in ('passed', 'composed') for result in results))


if __name__ == '__main__':
    raise SystemExit(main())
