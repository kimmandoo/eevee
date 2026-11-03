#!/usr/bin/env python3
"""Real quiet-save/exit/result journeys on isolated PTY/ConPTY application sessions.

Requires an already-built binary and existing terminal smoke dependencies. Uses
read-only SQLite observations and loopback HTTP responses; no account, personal
settings, copied UI implementation or model-generated filesystem effects.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import time
import traceback

from file_creation_smoke import CreationJourney, Gate, response_plan, save
from native_connection_smoke import Peer, until
from session_smoke import Fixture
from workbench_smoke import wait_for

ESC = '\x1b'
PREFIX = '요청界e\u0301 '
BURST = 'abcdefghijklmnopqrst'
FUTURE = 'Keep this unsent follow-up 요청界e\u0301 exactly.'
CASES = ('quiet-draft', 'active-quit', 'interrupt-quit', 'terminal-escape', 'failure-reason')


def task(journey):
    database = journey.data / 'workbench.sqlite3'
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=10)) as db:
        db.execute('PRAGMA query_only=ON')
        db.row_factory = sqlite3.Row
        row = db.execute('SELECT id,revision,lead_partner,partner_drafts FROM tasks ORDER BY id LIMIT 1').fetchone()
    if row is None:
        return None
    result = dict(row)
    result['partner_drafts'] = json.loads(result['partner_drafts'])
    return result


def draft(journey):
    current = task(journey)
    if current is None:
        return None
    state = current['partner_drafts'].get(str(current['lead_partner']), {})
    return next((value for value in state.get('drafts', []) if value['field'] == 'Request'), None)


def saved_text(journey, text):
    current = draft(journey)
    return current is not None and current['text'] == text


def exact_exit(session):
    # Deliberately no confirmation helper: a healthy explicit exit must not
    # need another consent, storage review, navigation key or repeated action.
    assert session.wait_exit() == 0, 'explicit exit did not restore a successful terminal'


def quiet_case(binary, output, root, size):
    fixture = Fixture(binary, output, root, size)
    try:
        session = fixture.launch('typing')
        session.paste(PREFIX)
        session.send('\x13')
        until(session, lambda: saved_text(fixture, PREFIX) and 'Storage: saved' in session.display(), 'explicit initial draft save')
        baseline = task(fixture)['revision']
        revisions = []
        started = time.monotonic()
        for char in BURST:
            sent = time.monotonic()
            session.send(char, settle=False)
            session.pump(max(0, .15 - (time.monotonic() - sent)))
            revisions.append(task(fixture)['revision'])
        duration = time.monotonic() - started
        assert duration >= 2.9, 'typing observation was too short'
        assert set(revisions) == {baseline}, 'continuous typing rewrote task snapshots: ' + repr(revisions)
        text = PREFIX + BURST
        until(session, lambda: saved_text(fixture, text) and 'Storage: saved' in session.display(), 'coalesced exact draft save', timeout=15)
        session.capture('coalesced-draft-saved')
        committed = task(fixture)['revision']
        session.send((ESC + '[D') * 8, settle=False)
        session.pump(2)
        assert task(fixture)['revision'] == committed, 'caret-only navigation autosaved a whole task'
        session.capture('caret-only-no-write')
        session.send('\x03', settle=False)
        exact_exit(session)
        current = draft(fixture)
        expected_cursor = len(text.encode()) - 8
        assert current == {'field': 'Request', 'text': text, 'cursor': expected_cursor}, current
        save(output / 'saved-after-quit.json', task(fixture))
        session.close()
        resumed = fixture.launch('resumed', '--continue')
        resumed.paste('!')
        resumed.send('\x13')
        inserted = PREFIX + BURST[:-8] + '!' + BURST[-8:]
        until(resumed, lambda: saved_text(fixture, inserted), 'resumed exact interior Unicode cursor')
        resumed.send('\x04', settle=False)
        assert resumed.alive(), 'Ctrl+D exited a nonempty Request'
        resumed.send('\x13')
        deleted = PREFIX + BURST[:-8] + '!' + BURST[-7:]
        until(resumed, lambda: saved_text(fixture, deleted), 'Ctrl+D forward-deleted at restored cursor')
        resumed.capture('restored-caret-forward-delete')
        resumed.send('\x03', settle=False)
        exact_exit(resumed)
        empty = fixture.launch('empty-composer')
        empty.send('\x04', settle=False)
        exact_exit(empty)
        return {'typing_seconds': duration, 'writes_during_burst': 0,
                'caret_only_revision_unchanged': True, 'quit_saved_exact_unicode_cursor': True,
                'restart_inserted_at_saved_cursor': True, 'ctrl_d_nonempty_forward_deleted': True,
                'ctrl_d_empty_exited': True}
    finally:
        fixture.close()


def native_case(binary, output, root, size, mode):
    gate = Gate()
    journey = None
    with Peer('responses', 'sk-public-interaction-fixture-only', True, retain_body_bytes=True) as peer:
        try:
            journey = CreationJourney(binary, output, root, size, size[0] == 60,
                                      'compatible', 'responses', True, False, peer)
            session = journey.session
            wait_for(session, 'Storage: saved', timeout=40)
            journey.setup_route()
            if mode in ('active-quit', 'interrupt-quit'):
                before = journey.begin('Explain the project without modifying files.', [('cancel', None)])
                until(session, peer.cancel_started.is_set, 'actual streaming socket did not start')
                session.paste(FUTURE, settle=False)
                journey.capture('streaming-with-unsent-follow-up')
                if mode == 'interrupt-quit':
                    session.send('\x03', settle=False)
                    assert session.alive(), 'first interrupt exited instead of stopping work'
                    session.send('\x03', settle=False)
                else:
                    session.command('quit', settle=False)
                exact_exit(session)
                assert peer.cancel_closed.wait(2), 'quit did not close the owned streaming connection'
                assert len(peer.snapshot('POST')) == before + 1, 'exit dispatched or replayed a follow-up'
                assert saved_text(journey, FUTURE), draft(journey)
                assert not journey.snapshot()['claims'], 'exit retained a local work lease after bounded teardown'
                detail = {'owned_socket_closed': True, 'unsent_draft_exact': True, 'request_count': 1}
            elif mode == 'terminal-escape':
                answer = 'The received answer remains available.'
                before = journey.begin('Explain the project without modifying files.', [response_plan(peer, text=answer, gate=gate)])
                until(session, gate.ready.is_set, 'completed response did not reach its actual frame gate')
                gate.release.set()
                journey.settled(None, before + 1)
                journey.capture('received-terminal-result')
                session.send(ESC, settle=False)
                session.command('quit', settle=False)
                exact_exit(session)
                assert len(peer.snapshot('POST')) == before + 1
                assert not journey.snapshot()['claims'], 'Esc recreated phantom work after terminal result'
                detail = {'terminal_escape_no_phantom_owner': True, 'request_count': 1}
            else:
                # Hold the first real HTTP refusal so the dispatch/lease
                # observation cannot race a fast loopback retry sequence.
                plans = [('rate_limited', lambda: gate.hold(peer))]
                plans.extend([('rate_limited', None)] * 3)
                before = journey.begin('Explain the project without modifying files.', plans)
                until(session, gate.ready.is_set, 'HTTP refusal did not reach its actual dispatch gate')
                gate.release.set()
                until(session, lambda: 'HTTP 429' in session.display() and not journey.snapshot()['claims']
                      and 'Storage: saved' in session.display(), 'actual failure reason did not settle visibly', timeout=40)
                journey.capture('typed-failure-with-actual-http-reason')
                view = session.display()
                assert 'Failed' in view and any(word in view.lower() for word in ('retry', 'check')), view
                requests = peer.snapshot('POST')[before:]
                assert all(row['fixture_mode'] == 'rate_limited' for row in requests), 'unexpected fallback after failure'
                assert all(row['body'] == requests[0]['body'] for row in requests), 'retry changed the reviewed request'
                session.command('verbose')
                journey.capture('verbose-failure-details')
                for _ in range(8):
                    if 'HTTP 429' in session.display():
                        break
                    session.send(ESC + '[5~')
                assert 'HTTP 429' in session.display(), session.display()
                session.command('quit', settle=False)
                exact_exit(session)
                detail = {'real_http_reason_visible': 'HTTP 429', 'typed_failed_with_action': True,
                          'attempts': len(requests), 'same_reviewed_request': True}
            assert not list(journey.project.iterdir()), 'interaction journey modified the isolated project'
            peer.assert_clean()
            save(output / 'final-state.json', journey.snapshot())
            save(output / 'actual-requests.json', peer.snapshot('POST'))
            return detail
        finally:
            gate.release.set()
            if journey is not None:
                journey.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--case', choices=CASES, action='append')
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'cases': [],
              'limits': ['Actual PTY/ConPTY surfaces, not physical terminal pixel/font certification',
                         'Loopback HTTP and disposable data; no live account or paid model requests']}
    with tempfile.TemporaryDirectory(prefix='ev-interaction-') as directory:
        for size in ((120, 36), (60, 18)):
            for mode in args.case or CASES:
                name = f'{size[0]}x{size[1]}-{mode}'
                case = output / name
                case.mkdir()
                root = Path(directory) / name
                root.mkdir()
                row = {'case': mode, 'size': list(size), 'status': 'failed'}
                try:
                    detail = (quiet_case(binary, case, root, size) if mode == 'quiet-draft'
                              else native_case(binary, case, root, size, mode))
                    row.update(status='passed', **detail)
                except Exception as error:
                    row.update(error=str(error), traceback=traceback.format_exc())
                report['cases'].append(row)
                save(output / 'results.json', report)
    if all(row['status'] == 'passed' for row in report['cases']):
        report['status'] = 'passed'
    save(output / 'results.json', report)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
