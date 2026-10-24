#!/usr/bin/env python3
"""PT12 real PTY/store/process acceptance using an immutable prebuilt binary.

No builds, SQL writes, provider programs, accounts or live-model calls. One
explicit keyless loopback protocol fixture exercises real Implement admission
and cancellation; it never produces a tool call, file effect or success frame.
Requires terminal_smoke.py's existing pyte/wcwidth dependencies. Example:
  python3 scripts/formation_readiness_smoke.py --binary /absolute/ev \
      --output /tmp/pt12 --language en --size 120x36 --size 60x18
Repeat with --language ko. All app state is created by actual application actions;
only inert portable definitions and ordinary project source files are authored here.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import tomllib

from terminal_smoke import (Session, confirm_exit, inspect_permission_scope,
                            permission_run_available, terminal_environment)
from native_connection_smoke import Peer, MODEL as FIXTURE_MODEL, decoded_payload, until
from connection_smoke_helpers import fill_text, focus_field, open_setup
from workbench_smoke import wait_for
from tm_smoke import expose_review

ESC = '\x1b'
UP, DOWN, LEFT, RIGHT, HOME = (ESC + suffix for suffix in ('[A', '[B', '[D', '[C', '[H'))
DRAFT = 'Keep mapped 界e\u0301 exact suffix'
MODEL = 'formation-local-unsent-model'
MOVE = 'Local mapped inspection'
SOURCE = 'held-public.py'
TABLES = ('tasks', 'partners', 'moves', 'held_items', 'partner_pairings', 'excerpts',
          'executions', 'active_work', 'provider_journal', 'move_usages', 'move_dispatches', 'selections')
LABELS = {
    'en': {'map': 'y apply', 'missing': 'Submit: no remembered route', 'held': 'held: unavailable',
           'draft': 'existing draft kept', 'moves': 'Moves: 1', 'empty': 'empty slot',
           'readonly': 'Read-only mapped owner', 'route': 'Remembered route:',
           'busy': 'Staging: blocked by owned work', 'eligible': 'Staging: eligible',
           'import': 'Import TM: literal', 'target': 'Import TM / choose', 'name': 'Move name',
           'review': 'Confirm local change', 'run': 'Review one-shot execution permission',
           'busy_refusal': 'owned work before'},
    'ko': {'map': 'y 적용', 'missing': '전송: 기억된 연결 없음', 'held': '지닌 참조: 사용 불가',
           'draft': '기존 초안 유지', 'moves': '기술: 1', 'empty': '빈 슬롯',
           'readonly': '배정된 소유자 읽기 전용', 'route': '기억된 연결:',
           'busy': '준비: 소유한 작업으로 차단됨', 'eligible': '준비: 가능',
           'import': 'TM 가져오기: 파일', 'target': 'TM 가져오기 · 배울', 'name': '기술 이름',
           'review': '로컬 변경 확인', 'run': '일회성 실행 권한 검토',
           'busy_refusal': '소유한 작업을 끝내'},
}


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


class Journey:
    def __init__(self, binary, output, root, size, language):
        self.binary, self.output, self.root, self.size, self.language = binary, output, root, size, language
        self.labels = LABELS[language]
        self.project, self.home, self.data = root / 'project 空 格', root / 'home', root / 'data'
        output.mkdir(parents=True, exist_ok=False)
        self.project.mkdir(parents=True)
        self.home.mkdir()
        tools = root / 'tools'
        tools.mkdir()
        git = shutil.which('git')
        assert git, 'git is required for actual repository observations'
        env = terminal_environment('mono')
        for key in list(env):
            if key.startswith(('EV_', 'EEVEE_', 'CODEX_', 'OPENAI_', 'ANTHROPIC_', 'CLAUDE_',
                               'GEMINI_', 'GOOGLE_', 'AWS_', 'OPENCODE_', 'DEVIN_', 'WINDSURF_')) or key.lower().endswith('_proxy'):
                env.pop(key, None)
        if os.name == 'nt':
            toolpath = os.pathsep.join((str(Path(git).parent), str(Path(sys.executable).parent)))
        else:
            (tools / 'git').symlink_to(Path(git).resolve())
            (tools / 'python3').symlink_to(Path(sys.executable).resolve())
            toolpath = str(tools)
        env.update(HOME=str(self.home), USERPROFILE=str(self.home), PATH=toolpath,
                   XDG_CONFIG_HOME=str(self.home / 'config'), XDG_DATA_HOME=str(self.home / 'data'),
                   XDG_CACHE_HOME=str(self.home / 'cache'), APPDATA=str(self.home / 'config'),
                   LOCALAPPDATA=str(self.home / 'cache'))
        self.env, self.sessions, self.serial = env, [], 0
        (self.project / SOURCE).write_text('value = "public local held fixture"\n', encoding='utf-8')
        (self.project / 'busy.py').write_text(
            'from pathlib import Path\nimport time\n'
            'Path("busy-started").write_text("actual owned process\\n")\n'
            'print("FORMATION_BUSY_OWNER", flush=True)\ntime.sleep(120)\n', encoding='utf-8')
        for args in (('init', '--quiet'), ('add', '--', SOURCE, 'busy.py'),
                     ('-c', 'user.name=Local acceptance', '-c', 'user.email=local@example.invalid', 'commit', '-qm', 'public fixture')):
            subprocess.run([git, '-c', 'core.hooksPath=', *args], cwd=self.project, check=True,
                           capture_output=True, timeout=15)
        self.authorized_journal = []
        self.authorized_dispatches = []
        self.launch()

    def state(self):
        database = self.data / 'workbench.sqlite3'
        if not database.exists():
            return {}
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=2)) as db:
            db.row_factory = sqlite3.Row
            return {table: [dict(row) for row in db.execute('SELECT * FROM "' + table + '" ORDER BY rowid')]
                    for table in TABLES}

    def task(self):
        return self.state()['tasks'][0]

    def wait(self, predicate, why, timeout=35):
        until(self.session, predicate, why, timeout=timeout)

    def label(self, name):
        wait_for(self.session, self.labels[name], timeout=35)

    def capture(self, name):
        self.serial += 1
        self.session.capture('%02d-%s' % (self.serial, name))
        save(self.output / ('%02d-%s-state.json' % (self.serial, name)), self.state())

    def launch(self):
        self.serial += 1
        self.session = Session([str(self.binary), '--data-dir', str(self.data), '--images', 'off',
                                '--color', 'mono', '--language', self.language,
                                *(['--ascii'] if self.size[0] == 60 else [])],
                               self.project, self.env, self.size, self.output / ('terminal-%02d' % self.serial))
        self.sessions.append(self.session)
        wait_for(self.session, 'Storage: saved', timeout=35)

    def workspace(self):
        for _ in range(12):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if footer.startswith(('Type Request /', 'Type a request /', 'Enter Send /', 'Draft only /',
                                  '요청 입력 /', 'Enter 전송 /', '초안 편집 /', '초안만')):
                return
            self.session.send(ESC)
        raise AssertionError('workspace not restored\n' + self.session.display())

    def lead(self, partner):
        self.workspace()
        self.session.command('lead ' + str(partner))
        self.wait(lambda: self.task()['lead_partner'] == partner, 'actual lead saved')
        self.workspace()

    def form(self, name, index):
        self.session.command(name)
        self.wait(lambda: next(row for row in self.state()['partners'] if row['id'] == self.task()['lead_partner'])['form'] == index,
                  'actual local form saved')
        self.workspace()

    def quiet_snapshot(self):
        state = self.state()
        return {key: state[key] for key in ('partners', 'held_items', 'partner_pairings', 'moves', 'excerpts',
                                           'provider_journal', 'move_usages', 'move_dispatches', 'selections')}, \
            (self.task()['lead_partner'], self.task()['partner_drafts'])

    def no_dispatch(self):
        state = self.state()
        assert state['provider_journal'] == self.authorized_journal, 'formation/inspection dispatched or rewrote provider history'
        assert not state['move_usages']
        assert state['move_dispatches'] == self.authorized_dispatches, 'formation/inspection added a dispatch receipt'
        assert not state['excerpts'] and not json.loads(self.task()['approved_memories']), 'formation/inspection implicitly selected context'

    def finish(self, *, owned_work=False):
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session, owned_work=owned_work)
        assert self.session.wait_exit() == 0
        self.session.close()

    def writer_admission(self):
        """Acquire an actual Implement writer through the ordinary UI/transport."""
        ready, closed = threading.Event(), threading.Event()
        peer_reader_finished = threading.Event()
        transport = {'close_observation': None, 'recv_timeouts': 0, 'client_bytes_after_request': 0,
                     'error': None}
        request = 'Implement a new project file named writer-not-created.txt with the public fixture text.'
        with Peer('chat_completions', '', True, retain_body_bytes=True) as peer:
            self.lead(2)
            open_setup(self.session, 'compatible', self.language)
            fill_text(self.session, 'endpoint', peer.base, self.language)
            fill_text(self.session, 'model', FIXTURE_MODEL, self.language)
            focus_field(self.session, 'auth', self.language)
            self.session.send(' ')  # Explicit reviewed keyless loopback source.
            focus_field(self.session, 'save', self.language)
            self.session.send('\r')
            self.wait(lambda: len(peer.snapshot('GET')) == 1 and FIXTURE_MODEL in self.session.display(),
                      'actual explicit local model metadata returned')
            self.session.send(FIXTURE_MODEL + '\r')
            self.workspace()
            assert not peer.snapshot('POST'), 'setup submitted an unapproved request'
            self.session.command('role-implement')
            self.workspace()
            before_writer = self.state()

            def hold_stream(handler, event, row):
                try:
                    event({'model': FIXTURE_MODEL, 'choices': [{'index': 0, 'delta': {
                        'content': 'WRITER_PARTIAL_ONLY_NO_SUCCESS'}, 'finish_reason': None}]})
                    ready.set()
                    handler.connection.settimeout(.1)
                    deadline = time.monotonic() + 90
                    while not peer.stopped.is_set() and time.monotonic() < deadline:
                        try:
                            incoming = handler.connection.recv(1)
                            if incoming == b'':
                                transport['close_observation'] = 'recv EOF'
                                closed.set()
                                return
                            transport['client_bytes_after_request'] += len(incoming)
                        except socket.timeout:
                            transport['recv_timeouts'] += 1
                        except (ConnectionResetError, ConnectionAbortedError) as error:
                            transport['close_observation'] = type(error).__name__
                            closed.set()
                            return
                    raise AssertionError('explicit writer cancellation did not close its real stream')
                except Exception as error:
                    transport['error'] = repr(error)
                    raise
                finally:
                    peer_reader_finished.set()

            peer.plans.put(('custom', hold_stream))
            # This is deliberate ordinary Request submission, not a fabricated
            # database claim or a formation-triggered send.
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if footer.startswith(('Type Request /', 'Type a request /', '요청 입력 /')):
                self.session.send('\r')
            self.session.send(UP * 64 + HOME + (ESC + '[3~') * 4096)
            self.session.paste(request)
            self.session.send('\x13')
            self.wait(lambda: any(draft['field'] == 'Request' and draft['text'] == request
                      for draft in json.loads(self.task()['partner_drafts']).get('2', {}).get('drafts', [])),
                      'exact Implement request draft committed')
            self.session.send('\r', settle=False)
            approved = False

            def submitted():
                nonlocal approved
                if ready.is_set():
                    return True
                if not approved and permission_run_available(self.session):
                    self.session.send('y', settle=False)
                    approved = True
                elif not approved and inspect_permission_scope(self.session):
                    pass
                elif 'Alt+A' in self.session.display():
                    self.session.send(ESC + 'a')
                return False

            self.wait(submitted, 'actual explicitly reviewed Implement request reaches loopback peer', timeout=40)
            task_id = self.task()['id']
            claims = self.state()['active_work']
            assert len(claims) == 1, claims
            claim = claims[0]
            assert claim['task_id'] == task_id and claim['partner'] == 2 \
                and claim['writing'] == 1 and claim['kind'] == 'provider', claim
            assert claim['request_id'], 'real writer claim has no request identity'
            posts = peer.snapshot('POST')
            assert len(posts) == 1
            _, messages = decoded_payload('chat_completions', posts[0]['body'])
            assert any(role == 'user' and request in text for role, text in messages)
            assert not (self.project / 'writer-not-created.txt').exists()
            save(self.output / 'acquired-writer-claim.json', claim)
            self.lead(1)
            self.session.command('formation formation.toml')
            self.label('map')
            self.label('busy')
            wait_for(self.session, '쓰기 소유자: #2' if self.language == 'ko' else 'Writer owner: #2')
            # Baseline is deliberately after real admission and lead change:
            # automatic appearance and submitted-draft transitions are genuine.
            baseline = self.state()['partners'], self.task()['partner_drafts'], self.task()['lead_partner']
            self.session.send('y')
            self.label('busy_refusal')
            assert (self.state()['partners'], self.task()['partner_drafts'], self.task()['lead_partner']) == baseline
            retained = self.state()['active_work']
            assert len(retained) == 1 and retained[0] == claim
            assert len(peer.snapshot('POST')) == 1, 'formation Apply dispatched another request'
            assert not (self.project / 'writer-not-created.txt').exists()
            self.capture('actual-acquired-implement-writer-refuses-apply')
            self.session.send(ESC)
            self.lead(2)
            before_cancel = self.state()

            def settled():
                state, view = self.state(), self.session.display()
                # Tool-free native turns have no tool effects to journal. The
                # actual dispatch receipt, not a fabricated tool record, owns
                # request attribution; physical EOF is local evidence only.
                return (closed.is_set() and peer_reader_finished.is_set() and not state['active_work']
                        and state['provider_journal'] == before_writer['provider_journal']
                        and 'Storage: saved' in view
                        and any(value in view.lower() for value in ('outcome unknown', '결과 불명'))
                        and any(value in view for value in ('partial', '부분'))
                        and any(value in view for value in ('unverified', '미검증')))

            try:
                self.session.command('cancel')
                self.wait(settled, 'actual writer cancellation closes the local stream and settles durable ownership',
                          timeout=40)
            finally:
                after_cancel, view = self.state(), self.session.display()
                save(self.output / 'writer-cancel-diagnostic.json', {
                    'claim': claim, 'closed': closed.is_set(), 'peer_reader_finished': peer_reader_finished.is_set(),
                    'transport': dict(transport), 'active_work': after_cancel['active_work'],
                    'provider_journal': after_cancel['provider_journal'],
                    'provider_tool_journal_unchanged': after_cancel['provider_journal'] == before_writer['provider_journal'],
                    'move_dispatches': after_cancel['move_dispatches'], 'executions': after_cancel['executions'],
                    'storage_saved': 'Storage: saved' in view, 'display': view,
                    'remote_outcome': 'unknown' if any(value in view.lower() for value in ('outcome unknown', '결과 불명')) else 'not_observed',
                    'remote_cancellation_confirmed': False, 'peer_errors': list(peer.errors),
                    'peer_workers_alive': [worker.is_alive() for worker in peer.workers],
                })
                save(self.output / 'writer-peer-requests.json', peer.snapshot())
            assert transport['error'] is None and transport['client_bytes_after_request'] == 0, transport
            assert after_cancel['provider_journal'] == before_writer['provider_journal'], \
                'tool-free cancellation fabricated provider-tool observations'
            assert after_cancel['executions'] == before_writer['executions'], \
                'tool-free cancellation fabricated local execution evidence'
            assert (after_cancel['partners'], after_cancel['tasks'][0]['partner_drafts'],
                    after_cancel['tasks'][0]['lead_partner']) == \
                (before_cancel['partners'], before_cancel['tasks'][0]['partner_drafts'],
                 before_cancel['tasks'][0]['lead_partner'])
            self.authorized_journal = after_cancel['provider_journal']
            self.authorized_dispatches = after_cancel['move_dispatches']
            assert len(self.authorized_dispatches) == 1
            receipt = self.authorized_dispatches[0]
            assert receipt['id'] == claim['request_id'] and receipt['task_id'] == task_id and receipt['partner'] == 2
            assert json.loads(receipt['pins']) == [], 'explicit writer unexpectedly acquired context selectors'
            assert len(peer.snapshot('POST')) == 1 and not (self.project / 'writer-not-created.txt').exists()
            peer.assert_clean()
            self.workspace()
            self.capture('implement-writer-cancelled-and-durably-settled')
        self.lead(1)
        self.no_dispatch()

    def fixture(self):
        self.session.command('goal')
        self.session.paste('PT12 mapped readiness local acceptance')
        self.session.send('\r')
        self.wait(lambda: bool(self.state()['tasks']), 'actual task saved')
        self.workspace()
        for count in (2, 3):
            self.session.command('party')
            self.session.command('add')
            self.wait(lambda: len(self.state()['partners']) == count, 'actual party addition')
            self.workspace()
        self.lead(2)
        self.form('glaceon', 7)
        # A real explicit import creates local availability, without fabricated
        # command/provider history or an automatic teaching action.
        portable = self.project / 'local-move.toml'
        portable.write_text('schema_version = 1\nname = "' + MOVE + '"\nform = "Glaceon"\n'
                            '[steps.Request]\ninstruction = "Inspect the local source"\npins = []\n', encoding='utf-8')
        self.session.command('move-import')
        self.label('import')
        self.session.paste(str(portable))
        self.session.send('\r')
        self.label('target')
        self.session.send(DOWN + '\r')  # Explicit roster target #2, not the lead default.
        self.label('name')
        self.session.send('\r')
        self.label('review')
        expose_review(self.session, self.language, self.capture)
        self.session.send('y')
        self.wait(lambda: len(self.state()['moves']) == 1, 'reviewed actual move import')
        assert self.state()['moves'][0]['partner'] == 2 and self.state()['moves'][0]['form'] == 7
        self.workspace()
        self.session.command('held-file')
        self.wait(lambda: 'eligible files /' in self.session.display() and SOURCE in self.session.display(),
                  'actual source inventory loaded the intended public fixture')
        self.session.command('search')
        self.session.paste(SOURCE)
        self.session.send('\r')
        wait_for(self.session, '1 eligible files /', timeout=35)
        self.session.send('\r')
        wait_for(self.session, 'Review local capture /', timeout=35)
        self.session.command('held-set-file')
        self.wait(lambda: any(row['partner'] == 2 and row['form'] == 7 for row in self.state()['held_items']),
                  'actual scoped held preference committed')
        self.workspace()
        self.form('eevee', 0)
        self.lead(3)
        self.form('espeon', 4)
        self.session.command('model ' + MODEL)
        wait_for(self.session, '/save-pairing', timeout=35)
        self.session.command('save-pairing')
        self.wait(lambda: any(row['partner'] == 3 and row['form'] == 4 for row in self.state()['partner_pairings']),
                  'actual remembered route committed without metadata discovery')
        self.workspace()
        footer = self.session.display().rstrip().splitlines()[-1].strip()
        if footer.startswith(('Type Request /', 'Type a request /', '요청 입력 /')):
            self.session.send('\r')
        self.session.paste(DRAFT)
        self.session.send(HOME + RIGHT * 5 + '\x13')
        self.wait(lambda: any(row['field'] == 'Request' and row['text'] == DRAFT
                             for row in json.loads(self.task()['partner_drafts']).get('3', {}).get('drafts', [])),
                  'actual nonempty owned draft saved')
        self.workspace()
        self.form('eevee', 0)
        self.lead(1)
        # The file becomes unavailable through an actual filesystem transition.
        # Restart observes it on the existing repository worker, not during draw.
        (self.project / SOURCE).unlink()
        self.finish()
        self.launch()
        self.workspace()
        definition = ('schema_version = 2\nname = "Mapped readiness"\nscope = []\ncontext = []\n'
                      'validation_commands = []\ncompletion_criteria = ["Review independently"]\n'
                      '[[steps]]\nform = "Vaporeon"\naction = "Stage only"\n')
        for role, form in (('scout', 'Vaporeon'), ('reviewer', 'Glaceon'), ('planner', 'Espeon')):
            definition += '\n[[roles]]\nrole = "' + role + '"\nform = "' + form + '"\nrequest = "Stage ' + role + ' only"\n'
        (self.project / 'formation.toml').write_text(definition, encoding='utf-8')
        self.form('jolteon', 2)
        self.workspace()

    def inspect(self, key, target, content):
        before = self.quiet_snapshot()
        counts = len(self.state()['executions']), len(self.state()['active_work'])
        self.session.send(key)
        wait_for(self.session, target, timeout=35)
        for _ in range(128):
            if content in self.session.display():
                break
            self.session.send(DOWN if key == 'm' else ESC + '[6~')
        else:
            raise AssertionError('scoped retained content was unreachable by bounded navigation\n' + self.session.display())
        self.capture('readonly-content-' + key)
        # No affirmative/action input can mutate a scoped observational visit.
        self.session.send('y\r' + DOWN)
        self.session.paste('do not stage or send')
        assert self.quiet_snapshot() == before
        assert (len(self.state()['executions']), len(self.state()['active_work'])) == counts
        self.no_dispatch()
        self.capture('readonly-' + key)
        self.session.send(ESC)
        self.label('map')
        self.label('empty')  # A deliberately unmapped slot survives every visit.
        assert self.quiet_snapshot() == before

    def run(self):
        cases = []
        self.fixture()
        drafts = self.task()['partner_drafts']
        roster = self.state()['partners']
        # Both existing inspector overlays precede the mapping frame. Warning
        # visits must still return to mapping when normal stack navigation
        # unwinds to an already-open Agents/Records surface.
        self.session.command('provider')
        wait_for(self.session, ' 연결 ' if self.language == 'ko' else ' Connections ')
        self.session.command('partner-summary')
        wait_for(self.session, '파트너 요약' if self.language == 'ko' else 'Partner summary')
        self.session.command('formation formation.toml')
        self.label('map')
        self.label('missing')
        self.label('eligible')
        self.capture('missing-route-stage-eligible')
        self.session.send(LEFT)  # Preserve a non-default empty mapping through inspectors.
        self.session.send(DOWN)
        self.label('held')
        self.label('moves')
        self.inspect('m', 'Eevee #2', MOVE)
        self.inspect('h', 'Eevee #2', SOURCE)
        self.session.send(DOWN)
        self.label('draft')
        self.inspect('r', 'Eevee #3', MODEL)
        assert self.task()['lead_partner'] == 1
        assert self.task()['partner_drafts'] == drafts and self.state()['partners'] == roster
        cases.append('three-real-partners-missing-route-unavailable-held-real-move-nonempty-draft')
        cases.append('scoped-warning-inspection-preserves-lead-nondefault-mapping-draft-caret-and-no-apply-or-dispatch')
        cases.append('existing-agents-and-records-history-unwind-still-returns-to-exact-formation-mapping')
        # Re-map the first slot. Its eligible review precedes actual admission of
        # an owned process while the formation remains in the overlay history.
        self.session.send(UP * 2 + RIGHT)
        self.label('eligible')
        self.capture('review-before-actual-owner-race')
        self.session.command('command-edit')
        self.session.paste(json.dumps(['python3', 'busy.py']))
        self.session.send('\r')
        self.wait(lambda: self.task()['command'] == json.dumps(['python3', 'busy.py']), 'real late command authored')
        self.session.command('execute')
        self.label('run')
        self.session.send(RIGHT + '\r')
        self.wait(lambda: (self.project / 'busy-started').exists()
                  and self.state()['active_work'] and self.state()['executions'][-1]['reserved'],
                  'actual approved command process holds ownership')
        command_owner = [(self.task()['id'], 1, 0)]
        assert [(row['task_id'], row['partner'], row['writing']) for row in self.state()['active_work']] == command_owner
        self.session.send(ESC)
        self.label('map')
        before = self.quiet_snapshot()
        self.session.send('y')
        assert self.quiet_snapshot() == before, 'late ownership allowed partial mapped-party writes'
        assert [(row['task_id'], row['partner'], row['writing']) for row in self.state()['active_work']] == command_owner
        assert self.state()['executions'][-1]['reserved'], 'formation refusal released a running command'
        self.no_dispatch()
        self.capture('actual-process-owner-race-refuses-apply')
        cases.append('actual-process-admitted-after-review-blocks-apply-without-partial-party-or-draft-writes')
        self.session.send(ESC)
        self.workspace()
        self.session.command('run')
        self.session.command('cancel')
        self.wait(lambda: not self.state()['active_work'] and not self.state()['executions'][-1]['reserved'],
                  'actual explicit cancellation and durable command settlement', timeout=40)
        self.workspace()
        self.writer_admission()
        cases.append('real-keyless-implement-admission-holds-exact-task-partner-writing-lease-apply-refuses-until-real-cancellation-settles')
        self.session.command('formation formation.toml')
        self.label('map')
        self.label('eligible')
        self.session.send('y')
        self.wait(lambda: [row['form'] for row in self.state()['partners']] == [1, 7, 4],
                  'independently revalidated successful mapped staging')
        assert json.loads(self.task()['partner_drafts'])['3']['drafts'] == json.loads(drafts)['3']['drafts']
        assert self.task()['lead_partner'] == 1
        assert [json.loads(row['record'])['state'] for row in self.state()['executions']] == ['Cancelled']
        self.no_dispatch()
        self.workspace()
        self.capture('successful-local-staging-after-owner-settlement')
        cases.append('successful-atomic-staging-independently-revalidated-and-nonempty-draft-caret-retained')
        self.session.command('formation-save exported.toml')
        wait_for(self.session, 'y 새 파일 생성' if self.language == 'ko' else 'y create new file', timeout=35)
        self.session.send('y')
        path = self.project / 'exported.toml'
        self.wait(path.exists, 'actual create-only formation export')
        exported = tomllib.loads(path.read_text(encoding='utf-8'))
        assert exported['schema_version'] == 2
        assert set(exported) <= {'schema_version', 'name', 'scope', 'context', 'steps', 'validation_commands', 'completion_criteria', 'roles'}
        assert len(exported['roles']) == 3
        assert all(set(role) == {'role', 'form', 'request'} for role in exported['roles'])
        self.no_dispatch()
        cases.append('portable-export-has-only-existing-semantic-fields-no-local-readiness-identities-or-authority')
        self.finish(owned_work=True)
        return {'status': 'passed', 'size': list(self.size), 'language': self.language, 'cases': cases}

    def close(self):
        for session in self.sessions:
            if session.alive():
                session.capture('last-before-close')
            session.close()


def size_arg(value):
    try:
        width, height = map(int, value.lower().split('x'))
        if width < 60 or height < 18:
            raise ValueError('minimum 60x18')
        return width, height
    except ValueError as error:
        raise argparse.ArgumentTypeError('size must be WIDTHxHEIGHT, at least 60x18') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    parser.add_argument('--size', type=size_arg, action='append', help='repeatable; default 120x36 and 60x18')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'journeys': []}
    with tempfile.TemporaryDirectory(prefix='ev-formation-readiness-') as temp:
        for index, size in enumerate(args.size or [(120, 36), (60, 18)]):
            journey = None
            try:
                journey = Journey(binary, output / ('%02d-%dx%d' % (index, *size)),
                                  Path(temp) / str(index), size, args.language)
                report['journeys'].append(journey.run())
            except Exception:
                report['journeys'].append({'status': 'failed', 'size': list(size), 'language': args.language,
                                           'traceback': traceback.format_exc()})
                save(output / 'results.json', report)
                raise
            finally:
                if journey is not None:
                    journey.close()
    report['status'] = 'passed'
    save(output / 'results.json', report)
    print(json.dumps(report, ensure_ascii=False))


if __name__ == '__main__':
    main()
