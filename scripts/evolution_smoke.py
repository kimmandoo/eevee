#!/usr/bin/env python3
"""Request-stable role evolution, explicit readonly policy and manual persistence.

Runs an explicit native binary in disposable PTYs with deterministic loopback SSE.
Peers propose operations but never write the tested files. No live model/account.
SQLite is read-only observation; all plans, mode changes and requests use the UI.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import traceback
from unittest.mock import patch

import file_creation_smoke as files
import response_smoke
from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

SOURCE = 'src/base.py'
CONTENT = '# user source\r\nvalue = 7\r\n'
GENERATED = 'generated/automatic.py'
GENERATED_CONTENT = 'print("automatic file")\r\n'

ROLES = (
    ('discover', 1, 'Vaporeon'),
    ('verify', 2, 'Jolteon'),
    ('implement', 3, 'Flareon'),
    ('plan', 4, 'Espeon'),
    ('diagnose', 5, 'Umbreon'),
    ('refactor', 6, 'Leafeon'),
    ('review', 7, 'Glaceon'),
    ('handoff', 8, 'Sylveon'),
)


def role_steps(active):
    # Deliberately neutral labels: the role, never the prose, owns appearance.
    return [
        {'label': 'Fixture stage ' + str(index + 1), 'phase': role,
         'status': 'done' if active is None or index < active
         else 'active' if index == active else 'pending'}
        for index, (role, _, _) in enumerate(ROLES)
    ]


def todo_call(identifier, steps):
    return {'id': identifier, 'name': 'todo',
            'arguments': {'operation': 'init', 'label': '', 'steps': steps}}


def paused_plan(peer, plan, gate):
    """Pause before the next SSE round, after the previous tool was consumed."""
    kind, reply = plan
    assert kind == 'custom'

    def held(handler, event, row):
        if gate.hold(peer):
            reply(handler, event, row)

    return ('custom', held)


class Journey(files.CreationJourney):
    def __init__(self, *args, **kwargs):
        self.expected_posts = 0
        original = response_smoke.SurfaceSession
        git = shutil.which('git')
        assert git, 'Git is required for genuine independent-review evidence'

        def remember(argv, cwd, environment, size, output, **options):
            for key in list(environment):
                if key.startswith('GIT_'):
                    environment.pop(key)
            environment.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull)
            if not (self.project / '.git').exists():
                (self.root / 'empty-bin' / 'git').symlink_to(git)
                subprocess.run([git, 'init', '-q', '--template='], cwd=self.project,
                               env=environment, check=True)
            self.restart_argv = list(argv)
            self.restart_environment = dict(environment)
            self.restart_size = size
            return original(argv, cwd, environment, size, output, **options)

        with patch.object(response_smoke, 'SurfaceSession', remember):
            super().__init__(*args, **kwargs)

    def capture(self, label):
        super().capture(label)
        files.save(self.output / (label + '-state.json'), {
            'state': self.snapshot(),
            'http_gets': len(self.peer.snapshot('GET')),
            'http_posts': len(self.peer.snapshot('POST')),
        })

    def begin(self, request, plans, **kwargs):
        # Hold the first terminal frame long enough to observe the real durable
        # lease; fast loopback text/read rounds must not race that observation.
        gate = files.Gate()
        kind, reply = plans[0]
        assert kind == 'custom'

        def held(handler, event, row):
            def emit(value, name=None):
                if value.get('type') == 'response.completed' and not gate.hold(self.peer):
                    return
                event(value, name)
            return reply(handler, emit, row)

        try:
            before = super().begin(request, [('custom', held), *plans[1:]], **kwargs)
            self.expected_posts += len(plans)
            self.capture('dispatched-' + str(before))
            return before
        finally:
            gate.release.set()

    def settled(self, answer, post_count):
        files.until(self.session, lambda: len(self.peer.snapshot('POST')) == post_count
                    and not self.snapshot()['claims'] and 'Storage: saved' in self.session.display(),
                    'native transport and durable ownership did not settle', timeout=40)
        self.session.command('response')
        self.session.send(response_smoke.ALT_END)
        super().settled(answer, post_count)

    def partner(self):
        state = self.snapshot()
        lead = state['tasks'][0]['lead_partner'] if state['tasks'] else state['partners'][0]['id']
        return next(row for row in state['partners'] if row['id'] == lead)

    def refresh_repository(self, label):
        self.workspace()
        self.session.command('refresh')
        self.session.command('diff')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.capture(label)
        self.workspace()

    def assert_mode(self, form, automatic):
        row = self.partner()
        assert (row['form'], bool(row['auto_form'])) == (form, automatic), row
        assert self.snapshot()['executions'] == 0, 'evolution implicitly executed a command'

    def progress(self):
        tasks = self.snapshot()['tasks']
        task = next(row for row in tasks if row['lead_partner'] == self.partner()['id'])
        return json.loads(task['workflows']).get('progress')

    def wait_mode(self, form, automatic=True):
        files.until(self.session, lambda: (self.partner()['form'], bool(self.partner()['auto_form']))
                    == (form, automatic), 'structured role did not reach form ' + str(form),
                    timeout=40)
        self.assert_mode(form, automatic)

    def assert_round(self, index, *, writing=True, owner=None):
        row = self.peer.snapshot('POST')[index]
        assert row['body']['model'] == files.MODEL, 'evolution switched the model'
        tools = files.advertised_tools(self.protocol, row['body'])
        ordinary = {'eevee_list_files', 'eevee_read_file', 'eevee_structural_search',
                    'eevee_artifact_list', 'eevee_artifact_read', 'eevee_artifact_search',
                    'eevee_artifact_locations', 'todo', 'eevee_delegate'}
        effects = {'eevee_write_file', 'eevee_edit_file', 'eevee_delete_file',
                   'eevee_rename_file', 'eevee_run_command'}
        assert tools == ordinary | (effects if writing else set()), tools
        if owner is not None:
            assert self.snapshot()['claims'] == owner, 'evolution replaced the live request owner/lease'

    def local_todo(self, steps):
        posts = len(self.peer.snapshot('POST'))
        self.workspace()
        self.session.command('todo init ' + json.dumps(steps, ensure_ascii=False))
        files.until(self.session, lambda: self.progress() is not None
                    and self.progress()['steps'] == steps
                    and self.progress()['adopted_revision'] == self.progress()['revision'],
                    'UI-authored todo did not save/adopt')
        self.workspace()
        assert len(self.peer.snapshot('POST')) == posts, 'local todo dispatched a request'


    def preview(self, request, form, operation, label):
        self.write_request(request)
        self.session.send('\x1b[D')
        caret = (self.session.screen.cursor.x, self.session.screen.cursor.y)
        before = (len(self.peer.snapshot('POST')), len(self.peer.snapshot('GET')),
                  self.partner()['form'], self.partner()['auto_form'])
        self.session.command('evolution-preview')
        title = 'Next evolution'
        wait_for(self.session, title)
        observed = self.session.display()
        assert form in observed and operation in observed, observed
        self.capture('prediction-' + label)
        # Enter is inspection-only, never the hidden editor's Send.
        self.session.send('\r')
        self.session.send(files.ESC)
        after = (len(self.peer.snapshot('POST')), len(self.peer.snapshot('GET')),
                 self.partner()['form'], self.partner()['auto_form'])
        assert after == before, (before, after)
        assert (self.session.screen.cursor.x, self.session.screen.cursor.y) == caret
        self.session.send('\x13')
        files.until(self.session, lambda: response_smoke.saved_request(
            self, request, len(request[:-1].encode())),
            'evolution inspection changed the exact draft/caret')
    def restart(self, label):
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()
        self.session = SurfaceSession([*self.restart_argv, '--continue'], self.project, self.restart_environment,
                                      self.restart_size, self.output / label,)
        wait_for(self.session, 'Storage: saved', timeout=40)


def run(binary, output, root, size, ascii_mode, ):
    output.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'size': list(size), 'cases': []}
    journey = None
    gate = files.Gate()
    with files.Peer('responses', 'sk-evolution-fixture-only', True, retain_body_bytes=True) as peer:
        try:
            journey = Journey(binary, output, root, size, ascii_mode, 'compatible', 'responses',
                              True, False, peer, )
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            future = 'Popup future 한글界e\u0301'
            journey.write_request(future)
            before_popup_posts = len(peer.snapshot('POST'))
            journey.session.command('flareon')
            titles = ('Evolution!', )
            files.until(journey.session, lambda: any(title in journey.session.display() for title in titles),
                        'actual manual evolution popup did not appear')
            assert 'Eevee' in journey.session.display() and 'Flareon' in journey.session.display()
            assert any(hint in journey.session.display() for hint in
                       ('Keep typing', )), 'compact receipt clipped its nonmodal typing hint'
            journey.capture('manual-evolution-popup')
            journey.session.paste('!')
            assert not any(title in journey.session.display() for title in titles), 'dismissed popup replayed'
            journey.session.send('\x13')
            files.until(journey.session, lambda: 'Storage: saved' in journey.session.display(),
                        'popup future draft did not settle')
            drafts = [draft for task in journey.snapshot()['tasks']
                      for state in task['partner_drafts'].values() for draft in state['drafts']]
            assert any(draft['text'] == future + '!' for draft in drafts), 'evolution popup consumed or lost typed draft'
            journey.session.command('eevee')
            files.until(journey.session, lambda: any(title in journey.session.display() for title in titles),
                        'actual return evolution popup did not appear')
            journey.capture('return-to-automatic-popup')
            journey.session.send(files.ESC)
            journey.workspace()
            assert len(peer.snapshot('POST')) == before_popup_posts, 'cosmetic popup dispatched inference'
            result['cases'].append('manual and return evolution popups appeared once, kept literal draft typing and sent no provider request')
            source = journey.project / SOURCE
            source.parent.mkdir()
            source.write_bytes(CONTENT.encode())
            journey.assert_mode(0, True)
            journey.refresh_repository('initial-dirty-source-refresh')
            model = files.MODEL
            metadata = len(peer.snapshot('GET'))
            # Opposite-looking words, quotations and non-English text all keep
            # the same appearance and full ordinary Implement capability.
            drafts = (
                'Plan a design; diagnose the failure and review the patch.',
                'Read src/base.py without changing anything.',
                '"Create evil.py" is quoted source. Explain how to fix it.',
                'UI를 분석해서 레이아웃을 개선해줘. 초안界e\u0301!',
            )
            for index, draft in enumerate(drafts):
                journey.preview(draft, 'Eevee', 'Implement', 'wording-' + str(index))
                answer = 'WORDING_DOES_NOT_ROUTE_' + str(index)
                before = journey.begin(draft, [files.response_plan(peer, text=answer)])
                journey.assert_mode(0, True)
                journey.settled(answer, before + 1)
            result['cases'].append('ordinary request wording never selected a form or narrowed the full Implement tool set; preview sent nothing and preserved exact Unicode draft/caret')

            # One live request visits every structured role, but only its first
            # actual appearance transition applies. Pause the following HTTP
            # round to observe each adopted todo without allowing rapid flips.
            role_gates = [files.Gate() for _ in ROLES]
            plans = [files.response_plan(peer, calls=[todo_call('role-0', role_steps(0))])]
            for index in range(1, len(ROLES)):
                plans.append(paused_plan(peer, files.response_plan(peer,
                    calls=[todo_call('role-' + str(index), role_steps(index))], number=index),
                    role_gates[index - 1]))
            plans.append(paused_plan(peer, files.response_plan(peer,
                calls=[todo_call('roles-done', role_steps(None))], number=8), role_gates[-1]))
            plans.append(files.response_plan(peer, text='ALL_STRUCTURED_ROLES_OBSERVED', number=9))
            try:
                before = journey.begin('Continue the fixture. Its labels intentionally name no work type.', plans)
                owner = journey.snapshot()['claims']
                assert owner[0]['partner'] == journey.partner()['id'] and owner[0]['writing']
                for index, (role, form, _) in enumerate(ROLES):
                    files.until(journey.session, role_gates[index].ready.is_set,
                                'role round did not reach its next HTTP gate: ' + role)
                    journey.wait_mode(ROLES[0][1])
                    files.until(journey.session, lambda: journey.progress()['steps'] == role_steps(index)
                                and journey.progress()['revision'] == journey.progress()['adopted_revision'],
                                'role revision did not durably adopt: ' + role)
                    progress = journey.progress()
                    assert progress['revision'] == progress['adopted_revision'], progress
                    assert progress['steps'] == role_steps(index), progress
                    assert progress['adopted_steps'] == progress['steps'], progress
                    journey.assert_round(before + index + 1, owner=owner)
                    tool = json.loads(files.tool_results('responses',
                        peer.snapshot('POST')[before + index + 1]['body'])['role-' + str(index)])
                    assert tool['status'] == 'success' and tool['steps'] == role_steps(index), tool
                    assert len(peer.snapshot('POST')) == before + index + 2, 'role evolution dispatched extra inference'
                    journey.capture('running-role-' + role)
                    role_gates[index].release.set()
                journey.settled('ALL_STRUCTURED_ROLES_OBSERVED', before + 10)
            finally:
                for role_gate in role_gates:
                    role_gate.release.set()
            journey.assert_mode(1, True)
            for index in range(before, before + 10):
                journey.assert_round(index)
            result['cases'].append('one owned running request adopted eight native role revisions but adapted appearance only once; owner, writer lease, model and full tools stayed frozen without extra inference or execution')

            # Preserve genuine first-role proof for every form: a fresh explicit
            # Send may adapt once even after its Send-time focus selection.
            for role, form, _ in ROLES[1:]:
                steps = [{'label': 'Fresh neutral stage', 'phase': role, 'status': 'active'}]
                done = [{**step, 'status': 'done'} for step in steps]
                answer = 'FRESH_REQUEST_ROLE_' + role.upper()
                fresh_gate = files.Gate()
                try:
                    before = journey.begin('Continue the neutral fixture.', [
                        files.response_plan(peer, calls=[todo_call('fresh-' + role, steps)]),
                        paused_plan(peer, files.response_plan(peer,
                            calls=[todo_call('fresh-done-' + role, done)], number=1), fresh_gate),
                        files.response_plan(peer, text=answer, number=2)])
                    files.until(journey.session, fresh_gate.ready.is_set,
                                'fresh role did not reach its completion gate: ' + role)
                    journey.wait_mode(form)
                    files.until(journey.session, lambda: journey.progress()['steps'] == steps,
                                'fresh active role was not adopted: ' + role)
                    fresh_gate.release.set()
                    journey.settled(answer, before + 3)
                finally:
                    fresh_gate.release.set()
                journey.assert_mode(form, True)
                assert journey.progress()['steps'] == done
                for index in range(before, before + 3):
                    journey.assert_round(index)
                tool = json.loads(files.tool_results('responses',
                    peer.snapshot('POST')[before + 1]['body'])['fresh-' + role])
                assert tool['status'] == 'success' and tool['steps'] == steps, tool
                journey.capture('fresh-request-role-' + role)
            result['cases'].append('each fresh explicit request retained its first distinct structured-role adaptation; all eight specialist forms remain proven through actual native todo execution')

            # A user-authored untagged step must not acquire a role by its words.
            # With no tagged focus, actual file activity is the fallback.
            journey.local_todo([{'label': 'Review code and plan a diagnosis', 'phase': None, 'status': 'active'}])
            request = 'Explain the source, do not infer a specialist from this wording.'
            journey.preview(request, 'Sylveon', 'Implement', 'untagged-focus')
            answer = 'ACTIVITY_FALLBACK_FILE_CREATED'
            before = journey.begin(request, [
                files.response_plan(peer, calls=[{'id': 'automatic-create', 'path': GENERATED,
                    'content': GENERATED_CONTENT}], gate=gate),
                files.response_plan(peer, calls=[todo_call('clear-untagged-focus', [])], number=1),
                files.response_plan(peer, text=answer, number=2)])
            files.until(journey.session, gate.ready.is_set, 'file activity did not reach the terminal gate')
            # File previews change live appearance before their terminal frame;
            # that held frame has not yet committed the partner snapshot.
            files.until(journey.session, lambda: '(Flareon) [Auto]' in journey.session.display().splitlines()[0],
                        'file activity did not evolve the live request appearance')
            busy_claims = journey.snapshot()['claims']
            assert busy_claims[0]['partner'] == journey.partner()['id']
            assert not (journey.project / GENERATED).exists()
            future = 'Perform an independent review of the code. 초안界e\u0301!'
            journey.write_request(future)
            journey.session.send('\r')
            assert '(Flareon) [Auto]' in journey.session.display().splitlines()[0]
            assert journey.snapshot()['claims'] == busy_claims, 'busy draft changed request ownership'
            assert len(peer.snapshot('POST')) == before + 1, 'busy Enter dispatched another request'
            journey.capture('busy-future-review-keeps-owner-and-form')
            journey.session.send(files.ESC)  # Keep the draft, do not steer/queue.
            gate.release.set()
            journey.permission(GENERATED, allow=True)
            journey.settled(answer, before + 3)
            journey.assert_mode(3, True)
            assert (journey.project / GENERATED).read_bytes() == GENERATED_CONTENT.encode()
            assert response_smoke.saved_request(journey, future, len(future.encode())), 'file consent lost the later exact draft/caret'
            result['cases'].append('untagged todo wording steered nothing; real file activity evolved Flareon without changing owner; busy future review did not send, and exact one-shot consent alone wrote actual bytes')

            # A read-looking form is not a restriction: a discover role can
            # propose and, only after exact approval, create a real file.
            discover = [{'label': 'Neutral stage', 'phase': 'discover', 'status': 'active'}]
            tagged_discover = [dict(discover[0], label='[discover] Neutral stage')]
            discover_done = [dict(discover[0], status='done')]
            tagged_gate = files.Gate()
            try:
                before = journey.begin('Review and diagnose this fixture.', [
                    files.response_plan(peer, calls=[todo_call('discover-write-plan', tagged_discover)]),
                    paused_plan(peer, files.response_plan(peer, calls=[{
                        'id': 'discover-create', 'path': 'discovered.txt', 'content': 'approved discovery\n'}],
                        number=1), tagged_gate),
                    files.response_plan(peer, calls=[todo_call('discover-write-done', discover_done)], number=2),
                    files.response_plan(peer, text='DISCOVER_WRITE_APPROVED', number=3)])
                owner = journey.snapshot()['claims']
                files.until(journey.session, tagged_gate.ready.is_set, 'discover write round did not arrive')
                journey.wait_mode(1)
                journey.assert_round(before + 1, owner=owner)
                assert journey.progress()['steps'] == discover, 'role tag did not normalize into structured phase'
                tagged_gate.release.set()
                files.until(journey.session, lambda: files.file_review_pending(journey.session),
                            'discover role lost its exact file approval')
                journey.assert_mode(1, True)
                assert journey.snapshot()['claims'] == owner and not (journey.project / 'discovered.txt').exists()
                journey.permission('discovered.txt', allow=True)
                journey.settled('DISCOVER_WRITE_APPROVED', before + 4)
            finally:
                tagged_gate.release.set()
            assert (journey.project / 'discovered.txt').read_bytes() == b'approved discovery\n'
            journey.assert_mode(1, True)
            result['cases'].append('tagged discover focus beat write-activity fallback while retaining full tools, the same writer owner and a separate exact file approval')

            handoff = [{'label': 'Neutral document stage', 'phase': 'handoff', 'status': 'active'}]
            handoff_gate = files.Gate()
            try:
                before = journey.begin('Write project documentation to README.md.', [
                    files.response_plan(peer, calls=[todo_call('handoff-plan', handoff)]),
                    paused_plan(peer, files.response_plan(peer, calls=[{
                        'id': 'documentation-create', 'path': 'README.md', 'content': '# Actual documentation\n'}],
                        number=1), handoff_gate),
                    files.response_plan(peer, calls=[todo_call('handoff-done',
                        [dict(handoff[0], status='done')])], number=2),
                    files.response_plan(peer, text='DOCUMENTATION_WRITTEN', number=3)])
                owner = journey.snapshot()['claims']
                files.until(journey.session, handoff_gate.ready.is_set, 'handoff write round did not arrive')
                journey.wait_mode(8)
                journey.assert_round(before + 1, owner=owner)
                handoff_gate.release.set()
                files.until(journey.session, lambda: files.file_review_pending(journey.session),
                            'handoff role lost exact document approval')
                journey.assert_mode(8, True)
                assert journey.snapshot()['claims'] == owner and not (journey.project / 'README.md').exists()
                journey.permission('README.md', allow=True)
                journey.settled('DOCUMENTATION_WRITTEN', before + 4)
            finally:
                handoff_gate.release.set()
            assert (journey.project / 'README.md').read_bytes() == b'# Actual documentation\n'
            journey.assert_mode(8, True)
            result['cases'].append('structured handoff role kept Sylveon during actual documentation creation and retained a separate exact one-shot approval')

            journey.session.command('flareon')
            journey.workspace()
            journey.assert_mode(3, False)
            manual_gate = files.Gate()
            manual_steps = [{'label': 'Pinned proposal', 'phase': 'diagnose', 'status': 'active'}]
            manual_done = [dict(manual_steps[0], status='done')]
            journey.preview('Read and plan a diagnosis. 초안界e\u0301!', 'Flareon', 'Implement', 'manual-appearance')
            try:
                before = journey.begin('Read and plan a diagnosis. 초안界e\u0301!', [
                    files.response_plan(peer, calls=[todo_call('manual-proposal', manual_steps)]),
                    paused_plan(peer, files.response_plan(peer,
                        calls=[todo_call('manual-done', manual_done)], number=1), manual_gate),
                    files.response_plan(peer, text='MANUAL_FORM_KEPT', number=2)])
                owner = journey.snapshot()['claims']
                files.until(journey.session, manual_gate.ready.is_set, 'manual todo proposal did not arrive')
                files.until(journey.session, lambda: journey.progress()['steps'] == manual_steps,
                            'manual proposal did not persist')
                journey.assert_mode(3, False)
                progress = journey.progress()
                assert progress['revision'] != progress['adopted_revision'], progress
                journey.assert_round(before + 1, owner=owner)
                manual_gate.release.set()
                journey.settled('MANUAL_FORM_KEPT', before + 3)
            finally:
                manual_gate.release.set()
            journey.restart('manual-restart')
            journey.assert_mode(3, False)
            journey.capture('manual-form-survived-restart')
            result['cases'].append('pinned specialist kept full ordinary tools and its appearance despite provider role proposals; proposals required adoption, and manual appearance survived a real restart')

            journey.session.command('eevee')
            journey.workspace()
            journey.wait_mode(0)
            # Connections are session-only. Deliberately restore the same route.
            journey.setup_route()
            journey.local_todo(discover)
            read_request = 'Review the patch, diagnose the error and plan its repair. 초안界e\u0301!'
            journey.preview(read_request, 'Vaporeon', 'Implement', 'adopted-focus')
            before = journey.begin(read_request, [
                files.response_plan(peer, calls=[{'id': 'ordinary-list', 'name': 'eevee_list_files',
                    'arguments': {'directory': '', 'query': 'src/', 'offset': 0}}]),
                files.response_plan(peer, calls=[{'id': 'ordinary-read', 'name': 'eevee_read_file',
                    'arguments': {'path': SOURCE, 'start_line': 1, 'end_line': 0}}], number=1),
                files.response_plan(peer, calls=[todo_call('ordinary-read-done', discover_done)], number=2),
                files.response_plan(peer, text='ADOPTED_FOCUS_CONTINUED', number=3)])
            journey.assert_mode(1, True)
            journey.settled('ADOPTED_FOCUS_CONTINUED', before + 4)
            read_results = files.tool_results('responses', peer.snapshot('POST')[before + 2]['body'])
            listing = json.loads(read_results['ordinary-list'])
            assert listing['paths'] == [SOURCE], listing
            actual = json.loads(read_results['ordinary-read'])
            assert actual['content'] == CONTENT and actual['sha256'] == hashlib.sha256(CONTENT.encode()).hexdigest(), actual
            assert source.read_bytes() == CONTENT.encode()
            journey.restart('automatic-restart')
            journey.assert_mode(1, True)
            journey.capture('evolved-automatic-mode-survived-restart')
            journey.setup_route()
            result['cases'].append('return to Eevee re-enabled automatic mode; adopted discover focus overrode unrelated request wording and previewed the actual Send form; real CRLF reads retained full tools, and automatic form persisted across restart')

            # Only deliberately selected Plan/Review narrow capability. Even
            # their provider role updates cannot broaden the running lease.
            for operation, form, role in (('Plan', 5, 'diagnose'), ('Review', 7, 'implement')):
                if operation == 'Review':
                    journey.refresh_repository('fresh-independent-review-evidence')
                journey.session.command('role-' + operation.lower())
                journey.workspace()
                appearance = 'Vaporeon' if operation == 'Plan' else 'Glaceon'
                journey.preview('Create forbidden.py and fix the code. 초안界e\u0301!',
                                appearance, operation, 'explicit-' + operation.lower())
                restriction_gate = files.Gate()
                restricted = [{'label': 'Restricted stage ' + operation, 'phase': role, 'status': 'active'}]
                forbidden = 'forbidden-' + operation.lower() + '.py'
                try:
                    before = journey.begin('Create ' + forbidden + ' and fix the code.', [
                        files.response_plan(peer, calls=[todo_call('restricted-plan-' + operation, restricted)]),
                        paused_plan(peer, files.response_plan(peer, calls=[{
                            'id': 'forbidden-' + operation, 'path': forbidden, 'content': 'must not exist\n'}],
                            number=1), restriction_gate),
                        files.response_plan(peer, calls=[todo_call('restricted-done-' + operation,
                            [dict(restricted[0], status='done')])], number=2),
                        files.response_plan(peer, text='EXPLICIT_' + operation.upper() + '_REFUSED', number=3)],
                        writing=False)
                    owner = journey.snapshot()['claims']
                    files.until(journey.session, restriction_gate.ready.is_set,
                                'explicit restriction role update did not arrive')
                    journey.wait_mode(form)
                    journey.assert_round(before + 1, writing=False, owner=owner)
                    restriction_gate.release.set()
                    journey.settled('EXPLICIT_' + operation.upper() + '_REFUSED', before + 4)
                finally:
                    restriction_gate.release.set()
                denial = json.loads(files.tool_results('responses',
                    peer.snapshot('POST')[before + 2]['body'])['forbidden-' + operation])
                assert denial['status'] == 'refused', denial
                assert not (journey.project / forbidden).exists()
                journey.assert_mode(form, True)
                for index in range(before, before + 4):
                    journey.assert_round(index, writing=False)
                journey.capture('explicit-' + operation.lower() + '-hard-write-denial')
            result['cases'].append('only explicit Plan/Review advertised read-only tools and acquired nonwriting leases; structured roles retained those restrictions, malicious writes were refused without approval/effects, and explicit Review kept Glaceon')

            requests = peer.snapshot('POST')
            assert len(requests) == journey.expected_posts, 'unrequested inference/classifier HTTP call'
            assert all(row['body']['model'] == model for row in requests), 'evolution switched the model'
            assert len(peer.snapshot('GET')) == metadata + 2, 'restart/evolution silently browsed metadata'
            result['evidence'] = {
                'specialist_forms': list(range(1, 9)),
                'automatic_inflight_transition_bound': 1,
                'http_posts': len(requests),
                'http_gets': len(peer.snapshot('GET')),
                'local_executions': journey.snapshot()['executions'],
                'active_claims': journey.snapshot()['claims'],
                'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                'created_sha256': hashlib.sha256((journey.project / GENERATED).read_bytes()).hexdigest(),
                'documentation_sha256': hashlib.sha256((journey.project / 'README.md').read_bytes()).hexdigest(),
                'real_restarts': 2,
            }
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            result['status'] = 'passed'
        except Exception as error:
            result['error'] = str(error)
            result['traceback'] = traceback.format_exc()
            if journey:
                journey.capture('failed')
        finally:
            gate.release.set()
            files.save(output / 'http-requests.json', peer.snapshot())
            if journey:
                files.save(output / 'owned-state.json', journey.snapshot())
                journey.session.close()
            files.save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    result = {'status': 'failed', 'binary': str(binary), 'sha256': digest, 'journeys': [],
              'limits': 'Deterministic loopback peers; actual UI/files/store, not authenticated model or physical GUI qualification.'}
    for size, ascii_mode in [((120, 36), False), ((60, 18), True)]:
        with tempfile.TemporaryDirectory(prefix='ev-evolution-漢字-') as temporary:
            result['journeys'].append(run(binary, output / str(size[0]), Path(temporary).resolve(), size, ascii_mode, ))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    result['status'] = 'passed' if all(row['status'] == 'passed' for row in result['journeys']) else 'failed'
    files.save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
