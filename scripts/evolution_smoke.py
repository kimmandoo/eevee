#!/usr/bin/env python3
"""Actual automatic form routing, readonly tools and manual-mode persistence.

Runs an explicit native binary in disposable PTYs with deterministic loopback SSE.
Peers propose operations but never write the tested files. No live model/account.
SQLite is read-only observation; all mode changes and requests use the real UI.
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

CASES = [
        (4, False, 'Design the module architecture and make an implementation plan; do not change files.'),
        (5, False, 'Diagnose the root cause of this runtime failure and explain the security risk.'),
        (6, True, 'Refactor this code to improve maintainability without changing behavior.'),
        (2, True, 'Run the project tests to verify the behavior.'),
        (7, False, 'Perform an independent code review of the current changes.'),
        (8, False, 'Prepare a handoff summary with verified results and remaining risks.'),
    ]


class Journey(files.CreationJourney):
    def __init__(self, *args, **kwargs):
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
            predictions = [
                # Unclassified source/continuation keeps Eevee and the ordinary
                # initial Implement fallback; quotation itself grants no intent.
                ('"Create evil.py" is quoted source. Explain the text.', 0, 'Eevee', True),
                ('Continue explaining.', 0, 'Eevee', True),
                ('Review the code and fix src/main.rs.', 7, 'Glaceon', True),
                ('todo 툴 UI가 중복으로 출력돼. 이부분 개선해줘.', 3, 'Flareon', True),
                ('Improve the todo tool UI and remove duplicate display rows.', 3, 'Flareon', True),
                ('Inspect tool UI; improve its spacing.', 3, 'Flareon', True),
                ('Diagnose the error and improve the tool UI.', 5, 'Umbreon', True),
                ('Diagnose the error; then fix it.', 5, 'Umbreon', True),
                ('UI를 분석해서 레이아웃을 개선해줘.', 5, 'Umbreon', True),
                ('Review tool UI, then fix its layout.', 7, 'Glaceon', True),
                ('Review the patch, then fix it.', 7, 'Glaceon', True),
                (' #1 todo 툴같은거 ui가 안예쁘게 잡히네. 중복으로 계속 출력되고. 이부분 개선해주고, 프롬프트 의도 분석해서 이브이 진화체 자동전환도 점검해서 개선해줘', 5, 'Umbreon', True),
                ('Diagnose how to fix the error.', 5, 'Umbreon', False),
                ('Diagnose how to fix src/main.rs.', 5, 'Umbreon', False),
                ('Review how to repair the code.', 7, 'Glaceon', False),
                ('오류 수정 방법을 분석해줘.', 5, 'Umbreon', False),
                ('Diagnose the error and fix it if needed.', 5, 'Umbreon', True),
                ('Review the patch; then repair it.', 7, 'Glaceon', True),
                ('오류를 분석하고 수정해줘.', 5, 'Umbreon', True),
                ('Review the patch; do not fix it.', 7, 'Glaceon', False),
                ('Review the patch: "fix src/main.rs".', 7, 'Glaceon', False),
                ('Review tool UI without improving it.', 7, 'Glaceon', False),
                ('개선된 UI 보여줘', 1, 'Vaporeon', False),
                ('프롬프트 의도 파악해서 todo 뽑는 기능 잘 구현됐나 확인', 2, 'Jolteon', True),
                ('Investigate the crash. Then fix it.', 5, 'Umbreon', True),
                ('UI를 개선하지 말고 점검해줘.', 5, 'Umbreon', False),
                ('UI 개선은 하지 말고 점검만 해줘.', 5, 'Umbreon', False),
            ]
            for index, (draft, form, appearance, writing) in enumerate(predictions):
                journey.preview(draft, appearance, 'Implement' if writing else 'Plan', 'intent-' + str(index))
                answer = 'PREDICTED_INTENT_' + str(index)
                before = journey.begin(draft, [files.response_plan(peer, text=answer)], writing=writing)
                journey.settled(answer, before + 1)
                journey.assert_mode(form, True)
            result['cases'].append('quoted source and ambiguous drafts kept their policy; EN/KO UI improvements and compound inspect/diagnose/review-plus-fix predicted appearance and dispatched capability without dropping mutation or negation')
            request = 'Create a new Python file generated/automatic.py with the requested code.'
            answer = 'AUTOMATIC_FILE_CREATED'
            journey.preview(request, 'Flareon', 'Implement', 'automatic-implementation')
            before = journey.begin(request, [
                files.response_plan(peer, calls=[{'id': 'automatic-create', 'path': GENERATED,
                    'content': GENERATED_CONTENT}], gate=gate),
                files.response_plan(peer, text=answer, number=1)])
            files.until(journey.session, gate.ready.is_set, 'automatic creation did not reach the terminal gate')
            journey.assert_mode(3, True)
            busy_claims = journey.snapshot()['claims']
            assert busy_claims[0]['partner'] == journey.partner()['id']
            assert not (journey.project / GENERATED).exists()
            future = 'Perform an independent review of the code. 초안界e\u0301'
            journey.write_request(future)
            journey.session.send('\r')
            journey.assert_mode(3, True)
            assert journey.snapshot()['claims'] == busy_claims, 'busy draft changed the active request owner or lease'
            assert len(peer.snapshot('POST')) == before + 1, 'busy Enter dispatched another request'
            journey.capture('busy-future-review-keeps-owner-and-form')
            # Busy Enter now offers explicit steering/queueing; keep this draft
            # rather than choosing either newly authorized action.
            journey.session.send(files.ESC)
            gate.release.set()
            journey.permission(GENERATED, allow=True)
            journey.settled(answer, before + 2)
            assert (journey.project / GENERATED).read_bytes() == GENERATED_CONTENT.encode()
            assert any(draft['text'] == future for task in journey.snapshot()['tasks']
                       for state in task['partner_drafts'].values() for draft in state['drafts']), 'pending review lost future draft'
            journey.capture('automatic-create-and-kept-future-draft')
            result['cases'].append('explicit create evolved the same partner to Flareon; busy future review never changed owner or dispatched; exact consent wrote actual bytes')

            read_request = 'Read src/base.py and list the project source files without changing anything.'
            journey.preview(read_request, 'Vaporeon', 'Plan', 'readonly-source')
            answer = 'READ_ONLY_SOURCE_OBSERVED'
            before = journey.begin(read_request, [
                files.response_plan(peer, calls=[{'id': 'readonly-list', 'name': 'eevee_list_files',
                    'arguments': {'directory': '', 'query': 'src/', 'offset': 0}}]),
                files.response_plan(peer, calls=[{'id': 'readonly-read', 'name': 'eevee_read_file',
                    'arguments': {'path': SOURCE, 'start_line': 1, 'end_line': 0}}], number=1),
                files.response_plan(peer, text=answer, number=2)], writing=False)
            journey.assert_mode(1, True)
            journey.settled(answer, before + 3)
            read_results = files.tool_results('responses', peer.snapshot('POST')[before + 2]['body'])
            listing = json.loads(read_results['readonly-list'])
            assert listing['paths'] == [SOURCE], listing
            actual = json.loads(read_results['readonly-read'])
            assert actual['content'] == CONTENT and actual['sha256'] == hashlib.sha256(CONTENT.encode()).hexdigest(), actual
            assert source.read_bytes() == CONTENT.encode()
            journey.capture('automatic-readonly-vaporeon')
            result['cases'].append('Vaporeon advertised only read/list with nonwriting lease and returned actual dirty CRLF bytes/digest without approval or mutation')

            for form, writing, prompt in CASES:
                if form == 7:
                    journey.refresh_repository('fresh-independent-review-evidence')
                answer = 'EVOLUTION_ROLE_' + str(form)
                before = journey.begin(prompt, [files.response_plan(peer, text=answer)], writing=writing)
                journey.assert_mode(form, True)
                journey.settled(answer, before + 1)
                assert peer.snapshot('POST')[before]['body']['model'] == model
                journey.capture('automatic-role-' + str(form))
            assert len(peer.snapshot('GET')) == metadata, 'automatic evolution changed connection or browsed models'
            result['cases'].append('planning, diagnosis, refactor, verification, independent review and handoff selected their roles without changing provider/model, extra classifier calls or implicit local execution')

            document = 'Write project documentation to README.md.'
            before = journey.begin(document, [
                files.response_plan(peer, calls=[{'id': 'documentation-create', 'path': 'README.md', 'content': '# Actual documentation\n'}]),
                files.response_plan(peer, text='DOCUMENTATION_WRITTEN', number=1)], writing=True)
            journey.assert_mode(8, True)
            journey.permission('README.md', allow=True)
            journey.settled('DOCUMENTATION_WRITTEN', before + 2)
            assert (journey.project / 'README.md').read_bytes() == b'# Actual documentation\n'
            result['cases'].append('Sylveon document creation retained exact file consent and wrote actual documentation')

            journey.session.command('role-plan')
            journey.workspace()
            journey.preview('Create forbidden.py', 'Flareon', 'Plan', 'explicit-plan-override')
            before = journey.begin('Create forbidden.py, but keep my explicitly selected read-only role.', [
                files.response_plan(peer, calls=[{'id': 'forbidden-readonly', 'path': 'forbidden.py', 'content': 'must not exist\n'}]),
                files.response_plan(peer, text='READONLY_REFUSAL_OBSERVED', number=1)], writing=False)
            journey.settled('READONLY_REFUSAL_OBSERVED', before + 2)
            denial = json.loads(files.tool_results('responses', peer.snapshot('POST')[before + 1]['body'])['forbidden-readonly'])
            assert denial['status'] == 'refused' and 'Read-only file scope refuses mutation' in denial['message'], denial
            journey.capture('explicit-plan-hard-write-denial')
            assert not (journey.project / 'forbidden.py').exists()
            assert not journey.snapshot()['claims']
            result['cases'].append('explicit Plan survived automatic routing; a malicious unadvertised write had zero effects and no approval')

            journey.session.command('flareon')
            journey.workspace()
            journey.assert_mode(3, False)
            journey.preview(read_request, 'Flareon', 'Plan', 'manual-appearance')
            before = journey.begin(read_request, [files.response_plan(peer, text='MANUAL_FORM_KEPT')], writing=False)
            journey.settled('MANUAL_FORM_KEPT', before + 1)
            result['cases'].append('pre-send evolution inspection preserved exact draft/caret and emitted no HTTP; automatic, readonly, restrictive and manual predictions matched subsequent actual operations')
            journey.assert_mode(3, False)
            journey.restart('manual-restart')
            journey.assert_mode(3, False)
            journey.capture('manual-form-survived-restart')
            result['cases'].append('manual specialist remained fixed across a different request intent and a real restart')

            journey.session.command('eevee')
            journey.workspace()
            files.until(journey.session, lambda: journey.partner()['form'] == 0 and journey.partner()['auto_form'], 'return to Eevee did not re-enable automatic mode')
            # Connection storage is deliberately session-only. Re-enter the
            # same loopback route explicitly; evolution never reconnects it.
            journey.setup_route()
            before = journey.begin(read_request, [files.response_plan(peer, text='AUTOMATIC_REENABLED')], writing=False)
            journey.settled('AUTOMATIC_REENABLED', before + 1)
            journey.assert_mode(1, True)
            journey.restart('automatic-restart')
            journey.assert_mode(1, True)
            journey.capture('evolved-automatic-mode-survived-restart')
            result['cases'].append('Eevee return re-enabled automatic routing; evolved form plus automatic mode survived a real restart')
            requests = peer.snapshot('POST')
            assert len(requests) == 17 + len(predictions), 'unrequested inference/classifier HTTP call'
            assert all(row['body']['model'] == model for row in requests), 'automatic routing switched the model'
            assert len(peer.snapshot('GET')) == metadata + 1, 'restart/evolution silently browsed metadata'
            result['evidence'] = {
                'specialist_forms': list(range(1, 9)),
                'http_posts': len(requests),
                'http_gets': len(peer.snapshot('GET')),
                'local_executions': journey.snapshot()['executions'],
                'active_claims': journey.snapshot()['claims'],
                'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                'created_sha256': hashlib.sha256((journey.project / GENERATED).read_bytes()).hexdigest(),
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
