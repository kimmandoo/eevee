#!/usr/bin/env python3
"""PK06 actual keyboard/store/evidence acceptance, with no SQL writes or live models."""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import traceback

import records_smoke as records
from native_connection_smoke import Peer
from workbench_smoke import wait_for
from workflow_smoke import DOWN, ESC, HOME, PAGE, RIGHT, UP


class Journey(records.Journey):
    def state(self):
        state = super().state()
        state['ribbons'] = self.ribbon_rows()
        return state

    def workspace(self, allow_offer=False):
        view = self.session.display()
        if any(title in view for title in ('Evolution!', '진화했어요!')):
            self.session.send('\x1b[24~')  # Neutral acknowledgment, not editor Escape.
        view = self.session.display()
        if any(title in view for title in ('Ribbons / project history', '리본 / 프로젝트 이력')):
            self.session.send(ESC)
        view = self.session.display()
        if any(title in view for title in ('Moves / local reusable actions', '기술 / 로컬 재사용 행동')):
            assert not any(text in view for text in ('Confirm local change', '로컬 변경 확인',
                                                     'wants to learn a move!', '기술을 배우려고 합니다!'))
            self.session.send(ESC)
        return super().workspace(allow_offer=allow_offer)

    def ribbon_rows(self):
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM ribbons ORDER BY project,kind')]

    def milestones(self):
        return [json.loads(row['record']) for row in self.ribbon_rows()
                if row['record'] is not None and Path(row['project']).resolve() == self.project.resolve()]

    def milestone(self, kind):
        return next((row for row in self.milestones() if row['kind'] == kind), None)

    def ribbon_panel(self):
        self.session.command('ribbons')
        self.wait_caption('Ribbons / project history', '리본 / 프로젝트 이력')
        self.session.send(UP * 8)

    def export(self, name):
        self.workspace()
        self.session.command('handoff')
        self.wait_caption('Portable task handoff / local-only', '이동 가능한 작업 인계 / 로컬 전용')
        self.session.command('export-json')
        self.wait_caption('LITERAL FILE PATH', '리터럴 파일 경로')
        destination = self.root / name
        self.session.paste(str(destination))
        self.session.send('\r')
        self.wait_caption('SANITIZED EXPORT PREVIEW', '민감 정보 정리 내보내기 미리보기')
        self.session.send(PAGE * 24 + RIGHT + '\r')
        self.until(destination.exists, 'explicit local handoff export did not finish')
        document = json.loads(destination.read_text(encoding='utf-8'))
        digest = hashlib.sha256(json.dumps(document, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
        self.until(lambda: any(row['kind'] == 'Handoff' and row['reference'] == digest
                              and row['handoff_revision'] == document['task']['revision']
                              for row in self.records('local_artifacts')),
                   'exact exported handoff receipt did not commit')
        self.workspace()
        assert 'ribbons' not in json.dumps(document), 'milestone metadata entered portable context'
        return document

    def teach_run(self, name):
        self.session.command('teach-move')
        self.wait_caption('Move name', '기술 이름')
        self.replace_editor('x' * 64, name)
        self.capture('explicit-move-name-preview')
        self.session.send('\r')
        self.wait_caption('Confirm local change', '로컬 변경 확인')
        assert self.milestone('learned_move') is None, 'teaching preview created a milestone'
        self.capture('neutral-teaching-review-before-commit')
        self.session.send('y')
        self.until(lambda: self.milestone('learned_move') is not None,
                   'explicit learned move did not produce a source-linked receipt')
        self.workspace()

    def configure_criteria(self):
        self.workspace()
        self.session.command('criteria-edit')
        self.session.paste('total(7, 2) returns 9')
        self.session.send('\r')
        self.until(lambda: [row['text'] for row in self.task()['criteria']] == ['total(7, 2) returns 9'],
                   'real criterion did not commit')
        self.workspace()

    def accept_review(self):
        self.form('glaceon')
        self.session.command('workflows')
        self.wait_caption('Specialist desk', '전문가 데스크')
        self.workflow_attach(8, 0)  # One actual source, followed by the real run.
        self.workflow_attach(8, 1)
        self.session.command('verdict-accept')
        self.until(lambda: self.milestone('accepted_review') is not None,
                   'current source/run-backed accepted review did not produce its receipt')
        self.workspace()


def run(binary, output, root, size, language):
    results = {key: {'evidence': []} for key in records.CASES}
    result = {'status': 'failed', 'size': list(size), 'language': language, 'cases': []}
    journey = None
    with Peer('chat_completions', 'unused-keyless-public-fixture', True, retain_body_bytes=True) as peer:
        try:
            journey = Journey(binary, output, root, size, size[0] == 60, results, peer, language)
            journey.configure_criteria()
            journey.form('sylveon')
            journey.export('unverified-context.json')
            assert journey.milestones() == [], 'unverified portable export invented a verified milestone'
            journey.form('glaceon')
            journey.session.command('workflows')
            journey.session.command('verdict-accept')
            journey.until(lambda: 'Accept blocked:' in journey.session.display(),
                          'unsupported review was not visibly refused')
            assert journey.milestones() == [], 'requested acceptance without source/run verification qualified'
            journey.capture('unsupported-review-and-unverified-export-no-milestone')
            journey.workspace()
            result['cases'].append('unverified handoff and unsupported requested acceptance created no milestone or effects')

            journey.source.write_text(records.GOOD, encoding='utf-8')
            journey.git_command('add', '--', 'calc.py')
            journey.git_command('-c', 'user.name=Local fixture', '-c', 'user.email=fixture@example.invalid',
                                'commit', '-qm', 'passing source')
            journey.session.command('refresh')
            journey.session.command('diff')
            wait_for(journey.session, 'Repository: refreshed', timeout=35)
            journey.workspace()
            capture = journey.pin_range('calc.py', '1:2')
            run_record = journey.execute_check(1, 'Succeeded')
            assert journey.milestones() == [], 'passing execution or source capture alone created a milestone'
            journey.teach_run('Literal check 한글')
            learned = journey.milestone('learned_move')
            move = next(row for row in journey.state()['moves']
                        if row['id'] == learned['evidence']['move']['id'])
            assert learned['evidence']['move']['revision'] == move['revision']
            assert learned['owner']['partner'] == run_record['prepared']['partner']['partner']
            assert learned['task_id'] == journey.record_task
            result['cases'].append('only an explicitly committed learned move created its exact move-revision/producer receipt')

            journey.accept_review()
            accepted = journey.milestone('accepted_review')
            artifact_id = accepted['evidence']['artifact']['id']
            artifact = next(row for row in journey.records('local_artifacts') if row['id'] == artifact_id)
            assert artifact['kind'] == 'Review' and artifact['producer'] == accepted['owner']
            assert any(reference.get('Source', {}).get('excerpt_id') == capture['id']
                       and reference['Source']['task_id'] == journey.record_task
                       and reference['Source']['source_sha256'] == capture['source_sha256']
                       for reference in artifact['evidence']), 'review receipt did not retain exact real source identity'
            assert {'Run': {'task_id': journey.record_task, 'run_id': run_record['prepared']['run_id']}} in artifact['evidence']
            journey.session.command('workflows')
            journey.session.command('verdict-accept')
            journey.until(lambda: journey.task()['workflows']['review_generation'] > 1,
                          'second explicit review did not commit')
            journey.workspace()
            assert journey.milestone('accepted_review') == accepted, 'later review replaced the first historical source'
            result['cases'].append('current source plus real passing check qualified effective acceptance once, retaining exact evidence and producer')

            journey.session.command('party')
            journey.session.send('a')
            journey.until(lambda: len(journey.state()['partners']) == 2, 'explicit second partner did not commit')
            journey.workspace()
            second = next(row['id'] for row in journey.state()['partners'] if row['id'] != journey.owner)
            journey.lead(second)
            journey.form('sylveon')
            verified_document = journey.export('verified-support-context.json')
            journey.until(lambda: journey.milestone('verified_handoff') is not None,
                          'normal handoff with current accepted review did not produce verified-support receipt')
            handoff = journey.milestone('verified_handoff')
            assert handoff['owner']['partner'] == second, 'handoff was attributed to another partner'
            assert handoff['task_id'] == journey.record_task and len(journey.milestones()) == 3
            assert set(verified_document) == {'schema_version', 'task', 'facts', 'sources', 'decisions', 'unresolved'}
            original = journey.milestones()
            saved_project = journey.project
            journey.quit()
            other_project = root / 'isolated-project'
            other_project.mkdir()
            subprocess.run([journey.git, '-c', 'core.hooksPath=', 'init', '-q', '--template='],
                           cwd=other_project, check=True, capture_output=True)
            journey.project = other_project
            journey.launch()
            journey.ribbon_panel()
            assert journey.milestones() == [], 'project history leaked through a shared data directory'
            journey.capture('isolated-project-with-shared-personal-data')
            journey.quit()
            journey.project = saved_project
            journey.launch()
            assert journey.milestones() == original, 'visiting another project changed original receipts'
            future = 'Future context draft 한글界e\u0301 Z-end'
            journey.write_request(future)
            journey.session.send('\x1b[D' * 3 + '\x13')
            journey.ribbon_panel()
            original_review = next(row for row in original if row['kind'] == 'accepted_review')
            wait_for(journey.session, original_review['evidence']['artifact']['id'])
            journey.capture('three-evidence-linked-first-recorded-milestones')
            before_effects = (len(journey.state()['executions']), len(peer.snapshot('POST')))
            journey.session.command('ribbon-inspect')
            journey.capture('ordinary-read-only-evidence-navigation')
            journey.workspace()
            assert (len(journey.state()['executions']), len(peer.snapshot('POST'))) == before_effects
            drafts = journey.task()['partner_drafts'][str(second)]['drafts']
            assert any(draft['field'] == 'Request' and draft['text'] == future
                       and draft['cursor'] == len(future.encode()) - 3 for draft in drafts), \
                'history inspection/navigation lost literal draft bytes or caret'
            result['cases'].append('verified-support handoff used actual producer and local evidence, while navigation and portable exports granted no effects or flavor')

            journey.ribbon_panel()
            journey.session.command('ribbon-hide')
            journey.until(lambda: sum(row['hidden'] for row in journey.milestones()) == 1,
                          'individual hide did not commit')
            hidden = next(row for row in journey.milestones() if row['hidden'])
            journey.capture('individual-hidden-source-retained')
            journey.workspace()
            journey.restart()
            assert next(row for row in journey.milestones() if row['kind'] == hidden['kind'])['hidden']
            journey.ribbon_panel()
            journey.session.command('ribbon-show')
            journey.until(lambda: not any(row['hidden'] for row in journey.milestones()),
                          'explicit show did not restore only the selected history row')
            journey.workspace()
            journey.session.command('partner-summary')
            journey.capture('attributed-summary-history')
            journey.workspace()
            journey.session.command('flavor-off')
            journey.until(lambda: journey.rows('local_settings')[0]['flavor'] == 0,
                          'persistent flavor suppression did not commit')
            journey.session.command('partner-summary')
            journey.capture('summary-with-optional-flavor-disabled')
            journey.workspace()
            journey.session.command('flavor-on')
            journey.until(lambda: journey.rows('local_settings')[0]['flavor'] == 1,
                          'explicit flavor restore did not commit')
            assert len(journey.milestones()) == 3
            result['cases'].append('individual hiding survived a real restart, show restored history, and optional flavor settings did not delete evidence')

            journey.source.write_text(records.GOOD + '# external change\n', encoding='utf-8')
            journey.session.command('refresh')
            journey.session.command('diff')
            wait_for(journey.session, 'Repository: refreshed', timeout=35)
            journey.workspace()
            journey.ribbon_panel()
            journey.capture('historical-receipt-after-external-source-change')
            assert journey.milestones() == original, 'source change rewrote historical first receipt metadata'
            journey.workspace()
            journey.source.write_text(records.GOOD, encoding='utf-8')
            journey.session.command('refresh')
            journey.session.command('diff')
            wait_for(journey.session, 'Repository: refreshed', timeout=35)
            journey.workspace()
            journey.connect()
            journey.session.command('model ' + records.MODEL)
            journey.workspace()
            post = journey.send_request('Summarize only the selected public fixture facts.', 'LOCAL_CONTEXT_REPLY')
            encoded = json.dumps(post['body'], ensure_ascii=False)
            assert '"ribbons"' not in encoded and 'Literal check 한글' not in encoded
            assert all(json.dumps(row['evidence'], sort_keys=True) not in encoded for row in original)
            journey.remove_chip(capture['id'])
            journey.until(lambda: journey.milestone('accepted_review') is None
                          and journey.milestone('verified_handoff') is None,
                          'purging actual source evidence left its review/handoff ribbon metadata')
            assert len(journey.milestones()) == 1 and journey.milestone('learned_move') is not None
            tombstones = [row for row in journey.ribbon_rows() if row['record'] is None]
            assert {row['kind'] for row in tombstones} == {'accepted_review', 'verified_handoff'}
            journey.capture('purged-evidence-removes-receipts-retains-only-kind-tombstones')
            journey.workspace()
            journey.session.command('tasks')
            journey.session.command('delete')
            journey.session.send(PAGE * 12 + RIGHT + '\r')
            journey.until(lambda: not journey.state()['tasks'], 'last-task project purge did not finish')
            assert journey.ribbon_rows() == [], 'final project purge retained milestone data or dedup tombstones'
            result['cases'].append('staleness stayed historical, actual HTTP omitted ribbon metadata, source purge removed linked receipts and final project purge removed tombstones')
            journey.quit()
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            if journey:
                journey.close()
            records.save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='ev-ribbons-漢字-') as temporary:
        root = Path(temporary).resolve()
        journeys = [run(binary, output / str(size[0]), root / str(size[0]), size, args.language)
                    for size in ((120, 36), (60, 18))]
    report = {'status': 'passed' if all(row['status'] == 'passed' for row in journeys) else 'failed',
              'binary': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'language': args.language, 'journeys': journeys,
              'limits': 'Real isolated CLI/PTY, local commands and read-only SQLite/loopback observations; not live-provider, physical-terminal or all-native conformance.'}
    records.save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
