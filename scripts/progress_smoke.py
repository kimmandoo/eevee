#!/usr/bin/env python3
"""Opt-in GP09 progress journeys in the real disposable TUI; no live model.

Requires an already-built ev, pyte/wcwidth, Node >=24 and installed sidecar
packages. Controlled Codex/ACP peers and an SDK query producer supply provider
observations, not execution evidence. Native progress uses the actual host todo
tool over loopback SSE. SQLite is read-only; all mutations use the app UI/tools.
No build, compiler, test suite, external endpoint or account is invoked.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import sqlite3
import sys
import tempfile
import traceback
import time
import uuid
from unittest.mock import patch

import response_smoke
from file_creation_smoke import CreationJourney, Gate, response_plan, save, tool_results
from native_connection_smoke import MODEL, Peer, decoded_payload, until
from response_smoke import SurfaceJourney, SurfaceSession
from terminal_smoke import FORMS, confirm_exit, inspect_permission_scope, permission_run_available
from workbench_smoke import wait_for

ESC = '\x1b'
HOME, DELETE = ESC + '[H', ESC + '[3~'
SIZES = ((120, 36), (60, 18))
ROUTES = ('codex', 'acp', 'sdk', 'native')
EAGER_PRELUDE = ('Before working, initialize a phased task list with the todo tool. '
                 'Keep one active step; report status as claims, not verification evidence.\n')
LABELS = {
    'en': ('Investigate error', 'Fix code', 'Verify tests'),
    'ko': ('오류 조사', '코드 수정', '테스트 검증'),
}
REQUESTS = {
    'en': ('Continue.', 'Continue.', 'Continue.'),
    'ko': ('계속해 주세요.', '계속해 주세요.', '계속해 주세요.'),
}
MALFORMED = 'Malformed progress marker'


def record(path, **value):
    with Path(path).open('a', encoding='utf-8') as stream:
        stream.write(json.dumps(value, ensure_ascii=False) + '\n')


def publish(path, value):
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(value, ensure_ascii=False), encoding='utf-8')
    pending.replace(path)


def protocol_steps(route, steps):
    if not isinstance(steps, list):
        return steps
    field = 'step' if route == 'codex' else 'content'
    statuses = {'active': 'in_progress', 'done': 'completed'}
    return [{field: step['label'], 'phase': step.get('phase'),
             'status': statuses.get(step['status'], step['status'])} for step in steps]


def controlled_peer(route):
    """JSONL process endpoint. Never performs tools, files, commands or network."""
    if '--version' in sys.argv or 'version' in sys.argv:
        print('codex-cli 0.152.1' if route == 'codex' else '1.18.0')
        return 0
    control, log = Path(os.environ['EV_PROGRESS_CONTROL']), os.environ['EV_PROGRESS_LOG']
    thread = 'progress-session-' + uuid.uuid4().hex
    turn = 'progress-turn'
    count = 0
    client_writing = False
    pending = b''

    def emit(packet):
        record(log, direction='peer', packet=packet)
        print(json.dumps(packet, ensure_ascii=False), flush=True)

    def event(method, **params):
        emit({'method': method, 'params': {'threadId': thread, 'turnId': turn, **params}})

    def wait_release(spec):
        if not spec.get('hold'):
            return
        record(log, kind='held', spec=spec)
        deadline = time.monotonic() + 60
        while not (control / 'release').exists():
            if time.monotonic() >= deadline:
                raise RuntimeError('Controlled busy-progress fixture was not released')
            time.sleep(.02)

    while True:
        if b'\n' not in pending:
            ready, _, _ = select.select([sys.stdin], [], [], .1)
            if not ready:
                continue
            chunk = os.read(sys.stdin.fileno(), 65536)
            if not chunk:
                return 0
            pending += chunk
        if b'\n' not in pending:
            continue
        line, pending = pending.split(b'\n', 1)
        packet = json.loads(line)
        record(log, direction='client', packet=packet)
        identity, method, params = packet.get('id'), packet.get('method'), packet.get('params', {})
        if identity is None:
            continue
        if method == 'initialize':
            if route == 'acp':
                client_writing = params.get('clientCapabilities', {}).get('fs', {}).get('writeTextFile', False)
            result = ({'userAgent': 'offline-progress-peer'} if route == 'codex' else
                      {'protocolVersion': 1, 'agentCapabilities': {'loadSession': True}})
        elif method == 'account/read':
            result = {'account': None, 'requiresOpenaiAuth': False}
        elif method == 'config/read':
            result = {'config': {'model': MODEL}}
        elif method == 'model/list':
            result = {'data': [{'model': MODEL, 'displayName': MODEL, 'isDefault': True}], 'nextCursor': None}
        elif method in ('thread/start', 'thread/resume'):
            thread = (params['threadId'] if method == 'thread/resume'
                      else 'progress-thread-' + uuid.uuid4().hex)
            result = {'thread': {'id': thread}, 'cwd': params['cwd'],
                      'model': params.get('model', MODEL),
                      'reasoningEffort': params.get('config', {}).get('model_reasoning_effort'),
                      'approvalPolicy': 'on-request', 'approvalsReviewer': 'user',
                      'sandbox': {'type': 'readOnly', 'networkAccess': False}}
        elif method == 'session/new':
            thread = 'progress-session-' + uuid.uuid4().hex
            result = {'sessionId': thread}
        elif method == 'session/load':
            thread = params['sessionId']
            result = {}
        elif method in ('turn/start', 'session/prompt'):
            spec = json.loads((control / 'next.json').read_text(encoding='utf-8'))
            count += 1
            turn = 'progress-turn-' + uuid.uuid4().hex
            blocks = params['input'] if route == 'codex' else params['prompt']
            envelope = json.loads('\n'.join(block.get('text', '') for block in blocks))
            operation = (envelope['operation'] if route == 'codex' else
                         'implement' if client_writing else
                         'review' if envelope.get('purpose', '').startswith('Independent read-only review')
                         else 'plan')
            record(log, kind='started', spec=spec, params=params, operation=operation)
            steps = protocol_steps(route, spec['steps'])
            if route == 'codex':
                emit({'id': identity, 'result': {'turn': {'id': turn, 'status': 'inProgress', 'items': []}}})
                # Repeat the same snapshot to exercise revision deduplication.
                for _ in range(2):
                    event('turn/plan/updated', plan=steps, explanation=spec['answer'])
                wait_release(spec)
                event('item/completed', item={'id': 'answer-' + turn, 'type': 'agentMessage', 'text': spec['answer']})
                event('turn/completed', turn={'id': turn, 'status': 'completed', 'items': []})
            else:
                for _ in range(2):
                    emit({'jsonrpc': '2.0', 'method': 'session/update', 'params': {
                        'sessionId': thread, 'update': {'sessionUpdate': 'plan', 'entries': steps}}})
                wait_release(spec)
                emit({'jsonrpc': '2.0', 'method': 'session/update', 'params': {
                    'sessionId': thread, 'update': {'sessionUpdate': 'agent_message_chunk',
                    'content': {'type': 'text', 'text': spec['answer']}}}})
                emit({'jsonrpc': '2.0', 'id': identity, 'result': {'stopReason': 'end_turn'}})
            record(log, kind='finished', spec=spec)
            continue
        elif method in ('turn/interrupt', 'session/cancel'):
            result = {}
        else:
            emit({'id': identity, 'error': {'code': -32601, 'message': 'Unsupported offline progress method: ' + str(method)}})
            continue
        emit({'jsonrpc': '2.0', 'id': identity, 'result': result})


# Only query production is replaced in a disposable copy. main.ts, policy.ts,
# SDK version policy and progress extraction remain the production implementation.
SDK_QUERY = r'''import {createSdkMcpServer, tool} from './sdk-real.mjs';
export {createSdkMcpServer, tool};
import {readFileSync, appendFileSync, existsSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
const version=JSON.parse(readFileSync(new URL('./package.json',import.meta.url))).claudeCodeVersion;
function log(value){appendFileSync(process.env.EV_PROGRESS_LOG,JSON.stringify(value)+'\n');}
export function query({prompt,options}) {
  const session_id=options.resume??randomUUID();
  let interrupted=false;
  return {
    accountInfo:async()=>({apiProvider:'firstParty'}),
    initializationResult:async()=>({claude_code_version:version,hooks_applied:true}),
    supportedModels:async()=>[], close(){}, interrupt:async()=>{interrupted=true;},
    async *[Symbol.asyncIterator](){
      const {value:submission}=await prompt.next();
      if(!submission||interrupted)throw Error('No actual released SDK submission');
      const spec=JSON.parse(readFileSync(process.env.EV_PROGRESS_CONTROL+'/next.json','utf8'));
      log({kind:'started',spec,submission,operation:options.tools.includes('Write')?'implement':'plan'});
      yield {type:'system',subtype:'init',session_id,model:'controlled-progress',
        claude_code_version:version,permissionMode:'default',apiKeySource:'ANTHROPIC_API_KEY',
        tools:[...options.tools,'mcp__eevee_source__discover',
          ...(options.tools.includes('Write')?['mcp__eevee_command__run_command']:[])],
        mcp_servers:[{name:'eevee_source',status:'connected'},
          ...(options.tools.includes('Write')?[{name:'eevee_command',status:'connected'}]:[])],plugins:[],skills:[]};
      const todos=Array.isArray(spec.steps)?spec.steps.map(s=>({content:s.label,phase:s.phase,
        status:({active:'in_progress',done:'completed'})[s.status]??s.status})):spec.steps;
      // Neither replayed history nor a delegated list may replace task progress.
      for(const [id,extra] of [['replayed',{isReplay:true}],['delegated',{parent_tool_use_id:'parent'}]]){
        yield {type:'assistant',session_id,...extra,message:{id,content:[{type:'tool_use',id,
          name:'TodoWrite',input:{todos:[{content:'Ignored '+id,status:'in_progress'}]}}]}};
        yield {type:'user',session_id,message:{content:[{type:'tool_result',tool_use_id:id,
          content:'Ignored historical/delegated observation',is_error:false}]}};
      }
      for(let n=0;n<2;n++){
        const id='progress-observation-'+submission.uuid+'-'+n;
        yield {type:'assistant',session_id,user_message_uuid:submission.uuid,
          message:{id:'observed-'+submission.uuid+'-'+n,content:[{type:'tool_use',id,name:'TodoWrite',input:{todos}}]}};
        // This is an observed provider report, not a host/local TodoWrite run.
        yield {type:'user',session_id,message:{content:[{type:'tool_result',tool_use_id:id,
          content:'Controlled provider progress observation only',is_error:false}]}};
      }
      if(spec.hold) {
        log({kind:'held',spec});
        const deadline=Date.now()+60000;
        while(!existsSync(process.env.EV_PROGRESS_CONTROL+'/release')) {
          if(Date.now()>=deadline)throw Error('Controlled busy SDK fixture was not released');
          await new Promise(resolve=>setTimeout(resolve,20));
        }
      }
      yield {type:'assistant',session_id,user_message_uuid:submission.uuid,
        message:{id:'answer-'+submission.uuid,content:[{type:'text',text:spec.answer}]}};
      log({kind:'finished',spec});
      yield {type:'result',session_id,is_error:false,subtype:'success'};
    }
  };
}
'''


class ProgressJourney(SurfaceJourney):
    setup_route = CreationJourney.setup_route

    def __init__(self, binary, output, root, size, route, peer, node, sidecar):
        self.progress_route = route
        self.control, self.log = root / 'control', root / 'peer.jsonl'
        self.control.mkdir()
        self.log.touch()
        original = response_smoke.SurfaceSession

        def session(argv, cwd, environment, dimensions, directory, **kwargs):
            for key in list(environment):
                if key.startswith(('EEVEE_', 'EV_', 'AZURE_', 'BEDROCK_', 'VERTEX_')):
                    environment.pop(key, None)
            environment.update(EV_PROGRESS_CONTROL=str(self.control), EV_PROGRESS_LOG=str(self.log))
            runtime = root / 'empty-bin'
            if route in ('codex', 'acp'):
                name = 'codex' if route == 'codex' else 'opencode'
                wrapper = runtime / name
                wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' ' +
                                   shlex.quote(str(Path(__file__).resolve())) +
                                   ' --controlled-peer ' + route + ' "$@"\n', encoding='utf-8')
                wrapper.chmod(0o700)
                environment['CODEX_API_KEY'] = 'controlled-not-a-real-key'
            elif route == 'sdk':
                package = root / 'sidecar'
                shutil.copytree(sidecar, package, ignore=shutil.ignore_patterns('.git', '.DS_Store'))
                sdk = package / 'node_modules/@anthropic-ai/claude-agent-sdk'
                shutil.copyfile(sdk / 'sdk.mjs', sdk / 'sdk-real.mjs')
                (sdk / 'sdk.mjs').write_text(SDK_QUERY, encoding='utf-8')
                environment.update(ANTHROPIC_API_KEY='controlled-not-a-real-key',
                                   EEVEE_CLAUDE_NODE=str(node), EEVEE_CLAUDE_SIDECAR=str(package / 'main.ts'))
            self.argv, self.environment, self.size = list(argv), dict(environment), dimensions
            return original(argv, cwd, environment, dimensions, directory, **kwargs)

        with patch.object(response_smoke, 'SurfaceSession', session):
            super().__init__(binary, output, root, size, size[0] == 60,
                             'compatible', 'responses', True, False, peer)
        self.evidence = []
        try:
            wait_for(self.session, 'Storage: saved', timeout=40)
            (self.project / 'calc.py').write_text('def total(a, b):\n    return a - b\n', encoding='utf-8')
            if route == 'native':
                self.setup_route()
            else:
                self.session.command('provider-' + {'codex': 'codex', 'acp': 'opencode', 'sdk': 'claude'}[route])
                self.workspace()
        except BaseException:
            self.close()
            raise

    def state(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            db.execute('PRAGMA query_only=ON')
            db.row_factory = sqlite3.Row
            tasks = [dict(row) for row in db.execute('SELECT * FROM tasks ORDER BY id')]
            partners = [dict(row) for row in db.execute('SELECT * FROM partners ORDER BY id')]
            claims = [dict(row) for row in db.execute('SELECT * FROM active_work')]
            executions = db.execute('SELECT count(*) FROM executions').fetchone()[0]
            receipts = [json.loads(row[0]) for row in db.execute('SELECT record FROM outgoing_receipts ORDER BY rowid')]
        for task in tasks:
            task['workflows'] = json.loads(task['workflows'])
            task['partner_drafts'] = json.loads(task['partner_drafts'])
        return {'tasks': tasks, 'partners': partners, 'claims': claims, 'executions': executions, 'receipts': receipts}

    def progress(self):
        return self.state()['tasks'][0]['workflows'].get('progress', {})

    def starts(self):
        return [json.loads(line) for line in self.log.read_text(encoding='utf-8').splitlines()
                if line.strip() and json.loads(line).get('kind') == 'started']

    def dispatch_count(self):
        return len(self.peer.snapshot('POST')) if self.progress_route == 'native' else len(self.starts())

    def capture_progress(self, label):
        before = self.dispatch_count()
        self.session.command('todo')
        # The compact task card scrolls; inspect the actual subsection, not a
        # fabricated model of how it would render at either terminal size.
        screens = []
        for _ in range(30):
            screens.append(self.session.display())
            if 'task progress' in screens[-1]:
                break
            self.session.send(ESC + '[6~')
        assert any('task progress' in screen for screen in screens), '\n'.join(screens)
        self.session.send(ESC + '[F')
        screens.append(self.session.display())
        self.capture(label)
        save(self.output / (label + '-screens.json'), screens)
        save(self.output / (label + '-state.json'), self.state())
        assert self.dispatch_count() == before, '/todo inspection dispatched inference'
        self.workspace()

    def adopt(self, label):
        before = self.progress()
        assert before['revision'] > 0, before
        if before['adopted_revision'] == before['revision']:
            # Automatic form mode adopts provider proposals without /todo-adopt.
            assert before['steps'] == before['adopted_steps'], before
            self.capture_progress(label + '-auto-adopted')
            self.evidence.append({'action': label + '-auto-adopted', 'revision': before['revision'],
                                  'steps': before['steps']})
            return
        self.capture_progress(label + '-proposed')
        count = self.dispatch_count()
        self.session.command('todo-adopt')
        until(self.session, lambda: self.progress().get('adopted_revision') == before['revision'],
              'explicit adoption did not commit')
        after = self.progress()
        assert after['steps'] == before['steps'] == after['adopted_steps']
        assert after['revision'] == before['revision'] and self.dispatch_count() == count
        self.capture_progress(label + '-adopted')
        self.evidence.append({'action': label, 'revision': after['revision'], 'steps': after['steps']})

    def preview(self, request, label):
        self.write_request(request)
        count = self.dispatch_count()
        self.session.command('evolution-preview')
        wait_for(self.session, 'Next evolution')
        screen = self.session.display()
        form = re.search(r'Appearance:\s*(' + '|'.join(FORMS) + r')\b', screen)
        operation = re.search(r'Effective operation:\s*(Plan|Implement|Review)', screen, re.IGNORECASE)
        assert form and operation, 'Preview did not expose form/operation\n' + screen
        self.capture(label + '-preview')
        self.session.send('\r')
        self.session.send(ESC)
        assert self.dispatch_count() == count, 'preview inspection dispatched inference'
        return {'form': FORMS.index(form.group(1)), 'operation': operation.group(1).lower(), 'screen': screen}

    def dispatch(self, request, steps, label, *, busy_probe=False):
        before = self.progress()
        previous_receipts = len(self.state()['receipts'])
        answer = label + ': progress report only.'
        gate = Gate() if busy_probe and self.progress_route == 'native' else None
        if busy_probe:
            (self.control / 'release').unlink(missing_ok=True)
        publish(self.control / 'next.json', {'steps': steps, 'answer': answer,
                                            'hold': busy_probe and self.progress_route != 'native'})
        prediction = self.preview(request, label)
        posts = len(self.peer.snapshot('POST'))
        if self.progress_route == 'native':
            self.peer.plans.put(response_plan(self.peer, calls=[{
                'id': 'progress-' + label, 'name': 'todo',
                'arguments': {'operation': 'init', 'label': '', 'steps': steps}}]))
            self.peer.plans.put(response_plan(self.peer, text=answer, gate=gate))
        count = self.dispatch_count()
        self.session.send('\r', settle=False)
        approved = False

        def started():
            nonlocal approved
            if self.dispatch_count() > count:
                return True
            if not approved and permission_run_available(self.session):
                self.capture(label + '-explicit-dispatch-scope')
                self.session.send('y', settle=False)
                approved = True
            elif not approved and inspect_permission_scope(self.session):
                pass
            elif 'Alt+A' in self.session.display():
                self.session.send(ESC + 'a')
            return False

        until(self.session, started, 'explicit request never reached controlled transport', timeout=90)
        if busy_probe:
            try:
                until(self.session, lambda: bool(self.state()['claims']) and
                      self.progress().get('steps') == steps, 'actual in-flight progress proposal missing')
                if gate is not None:
                    until(self.session, gate.ready.is_set, 'native progress result did not reach held terminal round')
                in_flight = self.state()
                frozen_owner = in_flight['claims'][0]
                current_form = in_flight['partners'][0]['form']
                assert current_form == prediction['form'], 'provider progress evolved the active form mid-run'
                adopted_revision = self.progress()['adopted_revision']
                assert adopted_revision == self.progress()['revision'], 'automatic proposal was not adopted in flight'
                held_count = self.dispatch_count()
                self.session.command('todo-adopt')
                until(self.session, lambda: any(reason in self.session.display() for reason in (
                    'Finish or cancel the active provider request first.',
                    'Wait for an idle saved open task')),
                    'busy progress adoption was not visibly refused')
                assert self.progress()['adopted_revision'] == adopted_revision
                self.write_request('Continue.' if 'ko-' not in label else '계속해 주세요.')
                self.session.send('\r')
                self.session.pump(.2)
                assert self.dispatch_count() == held_count, 'busy Send dispatched another provider request'
                assert self.state()['claims'][0] == frozen_owner, 'busy edits replaced frozen request ownership'
                assert self.state()['partners'][0]['form'] == current_form
                self.capture(label + '-busy-proposal-adoption-and-send-refused')
                self.session.send(ESC)  # Keep the draft; do not steer or queue it.
                self.evidence.append({'action':'busy-progress-keeps-frozen-authority','frozen_owner':frozen_owner,
                                      'actual_form':current_form,'dispatches':held_count,
                                      'adopted_revision':adopted_revision})
            finally:
                (self.control / 'release').touch()
                if gate is not None:
                    gate.release.set()
        until(self.session, lambda: not self.state()['claims'] and 'Storage: saved' in self.session.display(),
              'controlled turn did not settle', timeout=90)
        state = self.state()
        partner = next(p for p in state['partners'] if p['id'] == state['tasks'][0]['lead_partner'])
        assert partner['form'] == prediction['form'], (prediction, partner)
        assert len(state['receipts']) == previous_receipts + 1, 'logical request did not retain exactly one outgoing receipt'
        receipt = state['receipts'][-1]
        assert receipt['owner']['form'] == prediction['form']
        assert receipt['operation'].lower() == prediction['operation']
        expected_progress = None
        if before:
            active = next((step for step in before['steps'] if step['status'] == 'active'), None)
            pending = next((step for step in before['steps'] if step['status'] == 'pending'), None)
            if before['adopted_revision'] == before['revision'] and active:
                expected_progress = f"adopted revision {before['revision']} / {active['label']}"
                assert f"adopted revision {before['revision']}" in prediction['screen'], prediction['screen']
            elif before['adopted_revision'] == before['revision'] and pending:
                # The focus step falls back to the next pending step (0.1.35).
                expected_progress = f"adopted revision {before['revision']} / next: {pending['label']}"
            elif before['revision'] == 0:
                expected_progress = 'no step list from this route'
            else:
                adoption = 'adopted' if before['adopted_revision'] == before['revision'] else 'proposed; adoption required'
                expected_progress = f"revision {before['revision']} / {adoption} / no adopted active step"
        assert receipt.get('progress') == expected_progress, (receipt, expected_progress)
        save(self.output / (label + '-frozen-receipt.json'), receipt)
        if self.progress_route == 'native':
            observed = self.peer.snapshot('POST')
            assert len(observed) == posts + 2, 'todo tool must complete one actual host tool round'
            results = tool_results('responses', observed[-1]['body'])
            assert 'progress-' + label in results, results
            returned = results['progress-' + label]
            value = json.loads(returned) if isinstance(returned, str) else returned
            assert isinstance(value, dict), value
            expected_error = any(step['status'] == 'invalid_progress_status' for step in steps)
            assert (value.get('status') in ('error', 'refused')) == expected_error, value
            save(self.output / (label + '-native-tool-result.json'), results)
            operation = 'implement' if any(t.get('name') == 'eevee_write_file'
                for t in observed[posts]['body'].get('tools', [])) else 'plan'
            assert operation == prediction['operation'], (operation, prediction)
        else:
            assert not self.peer.snapshot('POST'), 'controlled route silently fell back to native HTTP'
            wire = self.starts()[-1]
            assert wire['operation'].lower() == prediction['operation'], (wire, prediction)
            save(self.output / (label + '-wire.json'), wire)
        assert state['executions'] == 0, 'provider progress fabricated local test execution'
        self.session.command('response')
        self.session.send(ESC + '[1;3F')
        for _ in range(60):
            if answer in self.session.display():
                break
            self.session.send(ESC + '[5~')
        assert answer in self.session.display(), 'Provider answer not visible after transcript inspection\n' + self.session.display()
        self.capture(label + '-actual-response')
        self.workspace()
        self.evidence.append({'action': label + '-dispatch', 'prediction': prediction,
                              'actual_form': partner['form'], 'previous_revision': before.get('revision', 0), 'receipt': receipt})
        return before

    def presentation(self, language):
        """Native host-tool updates in one request, using the production PTY."""
        labels = LABELS[language]
        steps = [{'label': label, 'status': 'active' if i == 0 else 'pending'}
                 for i, label in enumerate(labels)]
        request = ('Investigate the error; explain only, do not change files.'
                   if language == 'en' else '오류를 조사하고 설명만 해 주세요. 파일은 변경하지 마세요.')
        arguments = [
            {'operation': 'init', 'label': '', 'steps': steps},
            *[{'operation': 'view', 'label': '', 'steps': []} for _ in range(2)],
            {'operation': 'done', 'label': labels[0], 'steps': []},
            {'operation': 'start', 'label': labels[1], 'steps': []},
            *[{'operation': 'view', 'label': '', 'steps': []} for _ in range(2)],
            {'operation': 'start', 'label': 'Missing exact todo label', 'steps': []},
            {'operation': 'view', 'label': '', 'steps': []},
        ]
        gate = Gate()
        answer = 'TODO_PRESENTATION_FINAL: provider claims are not verification.'
        posts = len(self.peer.snapshot('POST'))
        for index, arguments_value in enumerate(arguments):
            self.peer.plans.put(response_plan(self.peer, calls=[{
                'id': 'presentation-' + str(index), 'name': 'todo', 'arguments': arguments_value}],
                gate=gate if index == 7 else None))
        self.peer.plans.put(response_plan(self.peer, text=answer))
        self.write_request(request)
        self.session.send('\r', settle=False)
        approved = False

        def held():
            nonlocal approved
            if gate.ready.is_set():
                return True
            if not approved and permission_run_available(self.session):
                self.session.send('y', settle=False)
                approved = True
            elif not approved and inspect_permission_scope(self.session):
                pass
            elif 'Alt+A' in self.session.display():
                self.session.send(ESC + 'a')
            return False

        try:
            until(self.session, held, 'native repeated todo request did not reach held round', timeout=90)
            self.session.command('response')
            self.session.send(ESC + '[1;3H')
            self.session.pump(.2)
            visible = self.session.display()
            self.capture('presentation-repeated-updates')
            save(self.output / 'presentation-held-state.json', self.state())
            assert visible.count('Todo progress') == 1, 'Repeated todo snapshots did not become one readable card\n' + visible
            assert '1/3 done' in visible and '7 updates' in visible, visible
            assert labels[1] in visible, 'Compact todo metadata hid the actual active step\n' + visible
            assert '"arguments"' not in visible and '"operation"' not in visible, visible
            assert not any(0xE000 <= ord(c) <= 0xF8FF for c in visible), 'Nerd Font dependency in todo view'
        finally:
            gate.release.set()
        until(self.session, lambda: not self.state()['claims'] and 'Storage: saved' in self.session.display(),
              'native presentation request did not settle', timeout=90)
        self.session.send(ESC + '[1;3F')
        until(self.session,
              lambda: re.sub(r'\s+', '', answer) in re.sub(r'\s+', '', self.session.display()),
              'final answer was not visible after latest-output navigation')
        self.capture('presentation-final-answer-visible')
        observed = self.peer.snapshot('POST')
        assert len(observed) == posts + len(arguments) + 1, 'Native loop retried or skipped calls'
        results = tool_results('responses', observed[-1]['body'])
        values = {key: json.loads(value) if isinstance(value, str) else value for key, value in results.items()}
        assert len(values) == len(arguments), values
        assert values['presentation-7']['status'] == 'refused', values
        assert all(value['status'] == 'success' for key, value in values.items() if key != 'presentation-7'), values
        save(self.output / 'presentation-native-tool-results.json', values)
        self.session.command('verbose')
        self.session.command('response')
        self.session.send(ESC + '[1;3H')
        screens = []
        previous_start = None
        while True:
            screen = self.session.display()
            position = re.search(r'(?:Paused|Following|New output) (\d+)-(\d+)/(\d+)', screen)
            assert position, 'Raw transcript did not disclose its paging position'
            start, end, total = map(int, position.groups())
            assert previous_start is None or start > previous_start, 'Raw transcript paging stopped before its end'
            screens.append(screen)
            if end == total:
                break
            previous_start = start
            self.session.send(ESC + '[6~')
        save(self.output / 'presentation-raw-inspection-screens.json', screens)
        joined = '\n'.join(screens)
        compact = re.sub(r'\s+', '', joined)
        assert '"arguments"' in compact and 'refused' in compact and 'Missing' in compact, joined
        observed_ids = set(re.findall(r'todo-\d+-\d+', compact))
        assert len(observed_ids) == len(arguments), 'Raw inspection lost early or repeated calls'
        self.session.command('verbose')
        self.workspace()
        self.evidence.append({'action': 'native-todo-readable-coalescing',
                              'raw_calls': len(observed_ids), 'operations': [value['operation'] for value in arguments],
                              'final_answer': answer, 'executions': self.state()['executions']})
        assert self.state()['executions'] == 0
        return self.evidence

    def manual(self, command, label):
        revision = self.progress()['revision']
        count = self.dispatch_count()
        self.session.command(command)
        until(self.session, lambda: self.progress()['revision'] == revision + 1,
              'manual progress edit did not save exactly one revision: ' + command)
        assert self.dispatch_count() == count
        progress = self.progress()
        assert progress['adopted_revision'] == progress['revision'], progress
        assert progress['adopted_steps'] == progress['steps'], progress
        self.capture_progress(label + '-user-adopted')
        self.evidence.append({'action': label + '-user-adopted', 'revision': progress['revision'],
                              'steps': progress['steps']})

    def continuity(self, old_binary, label):
        expected, count = self.progress(), self.dispatch_count()
        assert not expected.get('eager_off', False)
        assert expected['adopted_revision'] == expected['revision']
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()
        database = self.data / 'workbench.sqlite3'
        original = database.read_bytes()
        if old_binary is not None:
            refused = SurfaceSession([str(old_binary), *self.argv[1:]], self.project,
                                     self.environment, self.size, self.output / (label + '-old-writer'))
            try:
                wait_for(refused, 'Storage: save failed', timeout=40)
                refused.capture('actual-predecessor-progress-refusal')
            finally:
                # Published older writers can remain in failed-storage mode;
                # terminate this owned probe, do not claim graceful old exit.
                refused.close()
            assert database.read_bytes() == original, 'predecessor changed progress database bytes'
            self.evidence.append({'action': 'actual-predecessor-refusal',
                                  'binary': str(old_binary),
                                  'binary_sha256': hashlib.sha256(old_binary.read_bytes()).hexdigest(),
                                  'database_sha256_unchanged': hashlib.sha256(original).hexdigest(),
                                  'shutdown': 'owned failed-storage probe terminated'})
        self.session = SurfaceSession(self.argv, self.project, self.environment, self.size,
                                      self.output / (label + '-resumed'))
        wait_for(self.session, 'Storage: saved', timeout=40)
        self.session.pump(.3)
        assert self.progress() == expected, 'restart changed adopted list or eager preference'
        assert self.dispatch_count() == count and not self.state()['claims'], 'restart replayed provider work'
        self.capture_progress(label + '-progress-persisted-no-replay')
        self.evidence.append({'action': 'restart-persisted-adopted-progress-eager-no-replay',
                              'progress': expected, 'dispatches': count})
        if self.progress_route == 'native':
            # The fixture deliberately selected SESSION credentials, which must
            # not be restored on restart. Only after proving zero replay, use
            # the same explicit connection UI to reauthorize this exact local
            # endpoint/model/protocol before a later explicit Send.
            self.setup_route()
            assert self.progress() == expected, 'explicit reconnect changed task progress'
            assert self.dispatch_count() == count and not self.state()['claims']
            self.capture_progress(label + '-explicit-same-endpoint-reconnect')
            self.evidence.append({'action':'explicit-session-only-reconnect-after-no-replay',
                                  'endpoint':self.peer.base,'model':MODEL,'protocol':'responses'})

    def exercise(self, language, old_binary=None):
        labels = LABELS[language]
        initial_request = ('Investigate the error in calc.py and explain its cause; do not change files.'
                           if language == 'en' else
                           'calc.py 오류를 조사하고 원인을 설명해 주세요. 파일은 변경하지 마세요.')
        self.write_request(initial_request)
        initial_steps = [{'label': text,
                          'phase': ('조사', '수정', '검증')[i] if language == 'ko'
                          else ('investigate', 'fix', 'verify')[i],
                          'status': 'active' if i == 0 else 'pending'}
                         for i, text in enumerate(labels)]
        self.dispatch(initial_request, initial_steps, language + '-initial-proposal')
        assert self.progress()['steps'] == initial_steps
        assert self.progress()['adopted_revision'] == self.progress()['revision']
        assert self.progress()['adopted_steps'] == initial_steps
        assert not self.progress().get('eager_off', False), 'native todo prelude must default on'
        if self.progress_route == 'native':
            policy, messages = decoded_payload('responses', self.peer.snapshot('POST')[-2]['body'])
            assert EAGER_PRELUDE in policy + '\n'.join(str(message) for message in messages)
        self.adopt(language + '-initial-proposal')
        criterion = 'Actual verification evidence is still required' if language == 'en' else '실제 검증 증거가 필요함'
        self.session.command('criteria-edit')
        self.session.paste(criterion)
        self.session.send('\r')
        until(self.session, lambda: json.loads(self.state()['tasks'][0]['criteria']) == [{'text':criterion,'met':False}],
              'explicit user acceptance criterion did not persist')
        self.workspace()
        for stage, request in enumerate(REQUESTS[language]):
            statuses = (('done', 'active', 'pending'),
                        ('done', 'done', 'active'), ('done', 'done', 'done'))[stage]
            steps = [{'label': text, 'phase': ('조사', '수정', '검증')[i] if language == 'ko'
                      else ('investigate', 'fix', 'verify')[i], 'status': statuses[i]}
                     for i, text in enumerate(labels)]
            old = self.dispatch(request, steps, language + '-stage-' + str(stage), busy_probe=stage == 0)
            assert self.evidence[-1]['actual_form'] == (5, 3, 2)[stage], self.evidence[-1]
            until(self.session, lambda: self.progress().get('revision') == old.get('revision', 0) + 1,
                  'actual provider snapshot was lost or duplicated')
            proposed = self.progress()
            assert proposed['steps'] == steps, proposed
            assert proposed['adopted_revision'] == proposed['revision'], proposed
            assert proposed['adopted_steps'] == steps, proposed
            self.adopt(language + '-stage-' + str(stage))

        # An adopted active fix must not override manual appearance or an
        # explicit read-only request. These are real explicit sends, not just
        # predictions, and their unchanged provider list remains adopted.
        self.manual('todo start ' + labels[1], language + '-guard-active-fix')
        guard_steps = self.progress()['steps']
        self.session.command('vaporeon')
        until(self.session, lambda: self.state()['partners'][0]['form'] == 1 and
              not self.state()['partners'][0]['auto_form'], 'manual specialist selection did not persist')
        pinned_steps = [dict(step, status=('done', 'pending', 'done')[i])
                        for i, step in enumerate(guard_steps)]
        before_pinned = self.dispatch(REQUESTS[language][0], pinned_steps, language + '-manual-form-guard')
        proposed = self.progress()
        assert proposed['revision'] == before_pinned['revision'] + 1, proposed
        assert proposed['steps'] == pinned_steps, proposed
        assert proposed['adopted_revision'] == before_pinned['adopted_revision'], proposed
        assert proposed['adopted_steps'] == before_pinned['adopted_steps'], proposed
        assert self.evidence[-1]['actual_form'] == 1
        assert not self.state()['partners'][0]['auto_form']
        self.adopt(language + '-manual-form-proposal')
        self.manual('todo start ' + labels[1], language + '-manual-user-active-fix')
        guard_steps = self.progress()['steps']
        self.session.command('eevee')
        until(self.session, lambda: bool(self.state()['partners'][0]['auto_form']),
              'explicit return to automatic form did not persist')
        readonly = ('Read calc.py and explain its current code; do not change files or run commands.'
                    if language == 'en' else
                    'calc.py 파일을 읽고 현재 코드를 설명해 주세요. 파일을 변경하거나 명령을 실행하지 마세요.')
        self.dispatch(readonly, guard_steps, language + '-explicit-readonly-guard')
        assert self.evidence[-1]['actual_form'] == 1
        assert self.evidence[-1]['prediction']['operation'] == 'plan'
        assert self.progress()['adopted_revision'] == self.progress()['revision']
        before_review = self.dispatch_count()
        review = self.preview('Review the code changes without implementing a fix.', language + '-explicit-review-guard')
        assert review['form'] == 7 and review['operation'] == 'plan', 'Ordinary code review must remain read-only; independent Review is a separate explicit policy'
        assert self.dispatch_count() == before_review
        self.manual('todo done ' + labels[1], language + '-guard-fix-complete-claim')

        # Exact-label user operations are adopted immediately, even in manual
        # form mode. They never infer success or dispatch provider work.
        self.manual('todo start ' + labels[2], language + '-restart-verification')
        self.manual('todo done ' + labels[2], language + '-done')
        self.manual('todo block ' + labels[1], language + '-blocked')
        assert self.progress()['steps'][1]['status'] == 'blocked'
        self.manual('todo unblock ' + labels[1], language + '-unblocked')
        self.manual('todo start ' + labels[1], language + '-active')
        self.manual('todo drop ' + labels[1], language + '-dropped')
        assert self.progress()['steps'][1]['status'] == 'dropped'
        edited = [{'label': text + ' revised', 'phase': '검토' if language == 'ko' else 'revised',
                   'status': 'pending'} for text in labels]
        self.manual('todo-edit ' + json.dumps(edited, ensure_ascii=False), language + '-edit')
        assert self.progress()['steps'] == edited
        count = self.dispatch_count()
        pending = self.preview(REQUESTS[language][0], language + '-pending-focus')
        assert pending['form'] == 5 and pending['operation'] == 'plan', pending
        assert self.dispatch_count() == count, 'pending-step prediction dispatched inference'

        before = self.progress()
        self.session.command('todo start ' + edited[0]['label'][:-1])
        wait_for(self.session, 'exact label')
        assert self.progress() == before, 'inexact label changed progress'
        self.capture(language + '-exact-label-refusal')
        self.workspace()
        count = self.dispatch_count()
        for setting in ('on', 'off'):
            self.session.command('todo-eager ' + setting)
            until(self.session, lambda: self.progress()['eager'] == (setting == 'on'), 'eager preference not saved')
            assert self.dispatch_count() == count, 'eager preference sent a request'
            self.capture_progress(language + '-eager-' + setting)
            if self.progress_route == 'native':
                unchanged = self.progress()
                self.dispatch('Explain this task progress; do not change files or run commands.',
                              edited, language + '-eager-' + setting)
                assert self.progress() == unchanged, 'identical todo snapshot created another revision'
                body = self.peer.snapshot('POST')[-2]['body']
                policy, messages = decoded_payload('responses', body)
                payload = policy + '\n'.join(str(message) for message in messages)
                assert (EAGER_PRELUDE in payload) == (setting == 'on'), payload
                self.evidence.append({'action': 'native-eager-' + setting,
                                      'exact_added_bytes': EAGER_PRELUDE if setting == 'on' else '',
                                      'actual_request': body})
                count = self.dispatch_count()
            if setting == 'on':
                self.continuity(old_binary, language + '-eager-on')

        # Malformed provider observations must not destroy an adopted list or
        # terminate the answer stream. Inspect the original response item too.
        before = self.progress()
        malformed = [{'label': MALFORMED, 'phase': None, 'status': 'invalid_progress_status'}]
        self.dispatch('Explain this controlled progress observation; do not run tools or change files.',
                      malformed, language + '-malformed')
        assert self.progress() == before, 'malformed progress replaced the adopted list'
        self.session.command('response')
        self.session.send(ESC + '[1;3H')
        screens = []
        found = False
        needle = 'TodoWrite' if self.progress_route == 'sdk' else MALFORMED
        for _ in range(50):
            screen = self.session.display()
            screens.append(screen)
            # Compare actual transcript glyphs across terminal wrapping. The
            # wide partner sidebar is a separate column, not provider text; a
            # framed transcript's own left border is dropped first.
            transcript = ''.join(''.join(line.strip().removeprefix('│').split('│', 1)[0].split())
                                 for line in screen.splitlines())
            if ''.join(needle.split()) in transcript:
                found = True
                break
            self.session.send(ESC + '[6~')
        save(self.output / (language + '-malformed-transcript-screens.json'), screens)
        assert found, 'malformed progress original transcript item was not retained'
        self.capture(language + '-malformed-transcript-retained')
        self.workspace()
        self.manual('todo start ' + edited[1]['label'], language + '-independent-review-active-fix')
        before_review = self.dispatch_count()
        self.session.command('role-review')
        independent = self.preview(REQUESTS[language][0], language + '-independent-review-policy-guard')
        assert independent['form'] == 7 and independent['operation'] == 'review'
        assert self.dispatch_count() == before_review, 'Independent Review inspection sent without explicit user intent'
        assert (self.project / 'calc.py').read_text() == 'def total(a, b):\n    return a - b\n'
        assert self.state()['executions'] == 0
        completed = self.state()
        assert not completed['tasks'][0]['complete']
        assert json.loads(completed['tasks'][0]['criteria']) == [{'text':criterion,'met':False}], 'provider progress completed acceptance criteria'
        deleted_id = completed['tasks'][0]['id']
        count = self.dispatch_count()
        self.session.command('tasks')
        wait_for(self.session, 'Task cards / committed snapshots')
        self.session.send(HOME)
        self.session.command('delete')
        wait_for(self.session, 'Review local task deletion')
        self.session.send('\r')
        assert any(task['id'] == deleted_id for task in self.state()['tasks']), 'neutral Enter deleted adopted progress'
        self.session.command('delete')
        wait_for(self.session, 'Review local task deletion')
        self.session.send((ESC + '[6~') * 24 + ESC + '[C' + '\r')
        until(self.session, lambda: all(task['id'] != deleted_id for task in self.state()['tasks']),
              'explicit task purge retained its progress owner')
        assert self.dispatch_count() == count, 'task purge replayed provider work'
        self.capture(language + '-task-purge-removes-progress-owner')
        self.session.command('todo')
        wait_for(self.session, 'No todo yet')
        self.evidence.append({'action':'neutral-then-explicit-task-purge-removes-progress','task_id':deleted_id})
        self.workspace()
        return self.evidence


def run(args, route, size, language, output):
    output.mkdir(parents=True)
    result = {'status': 'failed', 'route': route, 'size': list(size), 'language': language}
    journey = None
    with tempfile.TemporaryDirectory(prefix='ev-progress-') as temporary:
        with Peer('responses', 'unused-offline-progress', True, retain_body_bytes=True) as peer:
            try:
                journey = ProgressJourney(args.binary, output, Path(temporary).resolve(), size,
                                          route, peer, args.node, args.sidecar)
                result['evidence'] = (journey.presentation(language) if args.presentation_only
                                      else journey.exercise(language, args.old_binary))
                journey.session.command('quit')
                confirm_exit(journey.session)
                assert journey.session.wait_exit() == 0
                peer.assert_clean()
                result['status'] = 'passed'
            except Exception as error:
                result.update(error=str(error), traceback=traceback.format_exc())
                if journey is not None:
                    journey.session.capture('failed')
            finally:
                if journey is not None:
                    shutil.copyfile(journey.log, output / 'controlled-wire.jsonl')
                    try:
                        save(output / 'final-state.json', journey.state())
                    finally:
                        journey.close()
                save(output / 'results.json', result)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path, help='already-built ev; never built by this driver')
    parser.add_argument('--output', required=True, type=Path, help='new artifact directory')
    parser.add_argument('--old-binary', type=Path, help='optional genuine predecessor for byte-exact refusal proof')
    parser.add_argument('--allow-controlled-runtime', required=True, action='store_true',
                        help='explicitly opt into disposable PTYs and controlled provider observations')
    parser.add_argument('--node', type=Path, default=shutil.which('node'))
    parser.add_argument('--sidecar', type=Path, default=Path(__file__).resolve().parents[1] / 'sidecar')
    parser.add_argument('--routes', nargs='+', choices=ROUTES, default=list(ROUTES))
    parser.add_argument('--presentation-only', action='store_true',
                        help='focused native init/status/repeated view/refusal/raw-history TUI journey')
    args = parser.parse_args(argv)
    args.binary, args.output, args.sidecar = (p.resolve() for p in (args.binary, args.output, args.sidecar))
    if args.presentation_only:
        args.routes = ['native']
    if os.name != 'posix':
        parser.error('this opt-in driver requires POSIX PTYs; no physical Windows claim')
    if not args.binary.is_file() or not os.access(args.binary, os.X_OK):
        parser.error('--binary must name an existing executable')
    if args.old_binary is not None:
        args.old_binary = args.old_binary.resolve()
        if not args.old_binary.is_file() or not os.access(args.old_binary, os.X_OK):
            parser.error('--old-binary must name an existing genuine predecessor executable')
        if args.old_binary == args.binary:
            parser.error('--old-binary must differ from --binary')
    if 'sdk' in args.routes:
        if args.node is None:
            parser.error('SDK route needs Node >=24; pass --node')
        args.node = Path(args.node).resolve()
        for required in (args.node, args.sidecar / 'main.ts',
                         args.sidecar / 'node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs'):
            if not required.is_file():
                parser.error('required existing SDK input missing: ' + str(required))
    args.output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'binary': str(args.binary),
              'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(), 'journeys': [],
              'limits': ['Controlled protocol/query observations, not live provider/model/account conformance.',
                         'Actual TUI, shared persistence and native todo host tool; no SQL writes.',
                         'No project code fix or test execution is claimed from provider step statuses.',
                         'English UI with English/Korean request and step content; PTY only, not physical terminal certification.']}
    try:
        for route in args.routes:
            for size in SIZES:
                for language in LABELS:
                    output = args.output / ('%s-%dx%d-%s' % (route, *size, language))
                    report['journeys'].append(run(args, route, size, language, output))
        if all(row['status'] == 'passed' for row in report['journeys']):
            report['status'] = 'passed'
    finally:
        save(args.output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'results': str(args.output / 'results.json')}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    if '--controlled-peer' in sys.argv:
        index = sys.argv.index('--controlled-peer')
        try:
            raise SystemExit(controlled_peer(sys.argv[index + 1]))
        except Exception:
            record(os.environ['EV_PROGRESS_LOG'], kind='peer_error', traceback=traceback.format_exc())
            raise
    raise SystemExit(main())
