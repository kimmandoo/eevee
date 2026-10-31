#!/usr/bin/env python3
"""Real terminal/native-loop completion proof with explicitly reviewed local writes.

Loopback SSE fixtures supply model responses, not effects. No live account or
credential is used. All homes, projects, settings and data are disposable.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import traceback

from file_creation_smoke import (CreationJourney, Gate, file_review_pending, response_plan,
                                 save, tool_results)
from native_connection_smoke import Peer, until
from workbench_smoke import wait_for


def task_state(journey):
    with closing(sqlite3.connect((journey.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('PRAGMA query_only=ON')
        task = db.execute('SELECT workflows FROM tasks ORDER BY id LIMIT 1').fetchone()
        return {'workflows': json.loads(task[0]) if task else None,
                'receipts': db.execute('SELECT count(*) FROM outgoing_receipts').fetchone()[0],
                'journal': [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_journal ORDER BY rowid')]}


def case(binary, output, root, size, blocked):
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'size': list(size), 'blocked_prerequisite': blocked}
    journey = None
    with Peer('responses', 'sk-public-completion-fixture-only', True, retain_body_bytes=True) as peer:
        try:
            journey = CreationJourney(binary, output, root, size, size[0] == 60,
                                      'compatible', 'responses', True, False, peer)
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            before = task_state(journey)
            step = 'Create both requested files'
            steps = [{'label': step, 'status': 'active'}]
            if blocked:
                steps.insert(0, {'label': 'Obtain unavailable prerequisite', 'status': 'blocked'})
            initial = [{'id': 'plan', 'name': 'todo',
                        'arguments': {'operation': 'init', 'label': '', 'steps': steps}},
                       {'id': 'first', 'path': 'first.txt', 'content': 'first exact bytes\n'}]
            final = [{'id': 'second', 'path': 'second.txt', 'content': 'second exact bytes\n'},
                     {'id': 'done', 'name': 'todo',
                      'arguments': {'operation': 'done', 'label': step, 'steps': []}}]
            posts = journey.begin('Create first.txt and second.txt in the project, and finish all reachable work. '
                                  'Do not claim an unavailable prerequisite was resolved.', [
                response_plan(peer, calls=initial),
                response_plan(peer, text='First file ready. The second requested file is not written yet.'),
                response_plan(peer, calls=final),
                response_plan(peer, text='Both files are written.' if not blocked else
                              'Both files are written. The prerequisite remains unavailable.'),
            ])
            journey.permission('first.txt', allow=True)
            until(journey.session, lambda: file_review_pending(journey.session)
                  or (len(peer.snapshot('POST')) >= posts + 2 and not journey.snapshot()['claims']),
                  'neither remaining-work approval nor premature settlement observed', timeout=40)
            if not file_review_pending(journey.session):
                raise AssertionError('Provider final stopped the owned request while second.txt was still missing')
            first = journey.project / 'first.txt'
            first_identity = (first.stat().st_ino, first.stat().st_mtime_ns, first.read_bytes())
            owner = journey.snapshot()['claims']
            assert len(owner) == 1, 'continuation lost the original owned request'
            assert not (journey.project / 'second.txt').exists(), 'continuation bypassed its separate file approval'
            journey.capture('remaining-work-has-separate-explicit-approval')
            journey.permission('second.txt', allow=True)
            journey.settled(None, posts + 4)
            assert first_identity == (first.stat().st_ino, first.stat().st_mtime_ns, first.read_bytes()), 'completed effect replayed'
            assert first.read_bytes() == b'first exact bytes\n'
            assert (journey.project / 'second.txt').read_bytes() == b'second exact bytes\n'
            assert journey._file_reviews == 2, 'continued request repeated or bypassed consent'
            state = task_state(journey)
            assert state['receipts'] == before['receipts'] + 1, 'continuation became an unrelated user dispatch'
            actual_steps = state['workflows']['progress']['steps']
            assert [row['status'] for row in actual_steps] == (['blocked', 'done'] if blocked else ['done']), actual_steps
            results = tool_results('responses', peer.snapshot('POST')[-1]['body'])
            for identity in ['first', 'second', 'done']:
                value = results[identity]
                value = json.loads(value) if isinstance(value, str) else value
                assert value['status'] == 'success', value
            save(output / 'state.json', state)
            save(output / 'actual-forward-transcript.json', peer.snapshot('POST'))
            peer.assert_clean()
            journey.capture('reachable-work-settled-with-original-blocker-preserved' if blocked else 'whole-request-finished')
            report.update(status='passed', requests=4, logical_dispatches=1, explicit_file_reviews=2,
                          first_effect_identity_unchanged=True, progress=actual_steps)
        except BaseException as error:
            report.update(error=str(error), traceback=traceback.format_exc())
            if journey is not None:
                journey.capture('failure')
                save(output / 'failure-state.json', task_state(journey))
            save(output / 'failure-transcript.json', peer.snapshot('POST'))
        finally:
            if journey is not None:
                journey.close()
            save(output / 'results.json', report)
    return report


def boundary_case(binary, output, root, size, mode):
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'size': list(size), 'boundary': mode}
    journey = None
    gate = Gate() if mode == 'no-progress' else None
    with Peer('responses', 'sk-public-completion-boundary-only', True) as peer:
        try:
            journey = CreationJourney(binary, output, root, size, size[0] == 60,
                                      'compatible', 'responses', True, False, peer)
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            calls = [{'id': 'plan', 'name': 'todo', 'arguments': {
                'operation': 'init', 'label': '', 'steps': [{'label': 'Finish requested files', 'status': 'active'}]}}]
            if mode != 'no-progress':
                calls.append({'id': 'first', 'path': 'first.txt', 'content': 'approved original bytes\n'})
            plans = [response_plan(peer, calls=calls, gate=gate), response_plan(peer, text='Some requested work remains.')]
            if mode == 'cancel':
                plans.append(('cancel', None))
            elif mode == 'no-progress':
                plans.append(response_plan(peer, text='Some requested work remains.'))
            posts = journey.begin('Create first.txt and second.txt in the project and finish the request.', plans)
            if gate is not None:
                until(journey.session, gate.ready.is_set, 'initial round did not reach owned no-progress gate')
                gate.release.set()
            if mode != 'no-progress':
                journey.permission('first.txt', allow=mode == 'cancel')
            if mode == 'cancel':
                until(journey.session, peer.cancel_started.is_set, 'continuation did not reach actual streaming boundary')
                journey.capture('cancel-during-owned-continuation')
                journey.session.command('cancel', settle=False)
                until(journey.session, peer.cancel_closed.is_set, 'cancel did not close actual continuation socket')
            count = 2 if mode == 'deny' else 3
            journey.settled(None, posts + count)
            assert not (journey.project / 'second.txt').exists(), 'unfinished continuation performed an unapproved effect'
            first = journey.project / 'first.txt'
            if mode == 'cancel':
                assert first.read_bytes() == b'approved original bytes\n', 'cancellation lost the earlier approved effect'
            else:
                assert not first.exists(), 'denial or no-progress stopping created a file'
            state = task_state(journey)
            assert [step['status'] for step in state['workflows']['progress']['steps']] == ['active'], state
            peer.assert_clean()
            save(output / 'state.json', state)
            save(output / 'transcript.json', peer.snapshot('POST'))
            journey.capture('stopped-with-unfinished-claims-preserved')
            report.update(status='passed', requests=count, unfinished_claim_preserved=True,
                          earlier_approved_file_retained=mode == 'cancel',
                          continuation_socket_closed=peer.cancel_closed.is_set())
        except BaseException as error:
            report.update(error=str(error), traceback=traceback.format_exc())
            if journey is not None:
                journey.capture('failure')
        finally:
            if gate is not None:
                gate.release.set()
            if journey is not None:
                journey.close()
            save(output / 'results.json', report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'limits': 'Actual terminal, native HTTP and approved filesystem effects; synthetic model, not external-adapter auto-resubmission or live-provider conformance.', 'journeys': []}
    for size in [(120, 36), (60, 18)]:
        for blocked in [False, True]:
            name = f'{size[0]}x{size[1]}-' + ('mixed-blocker' if blocked else 'forward')
            with tempfile.TemporaryDirectory(prefix='ev-completion-') as temporary:
                result = case(binary, output / name, Path(temporary), size, blocked)
                report['journeys'].append(result)
            save(output / 'results.json', report)
            if result['status'] != 'passed':
                print(json.dumps(report, indent=2))
                return 1
        for mode in ['deny', 'no-progress', 'cancel']:
            with tempfile.TemporaryDirectory(prefix='ev-completion-boundary-') as temporary:
                result = boundary_case(binary, output / f'{size[0]}x{size[1]}-{mode}', Path(temporary), size, mode)
                report['journeys'].append(result)
            save(output / 'results.json', report)
            if result['status'] != 'passed':
                print(json.dumps(report, indent=2))
                return 1
    report['status'] = 'passed'
    save(output / 'results.json', report)
    print(json.dumps(report, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
