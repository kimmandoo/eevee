#!/usr/bin/env python3
"""GP08 actual TUI / local HTTP / frozen receipt / restart acceptance.

Requires an already-built binary and the terminal_smoke dependencies. Uses only
bounded deterministic loopback replies, disposable homes and read-only SQLite
observation. --old-binary exercises a genuine previous reader without modifying
SQL/schema/records. No live model, credentials, provider interpretation or builds.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile
import traceback

import file_creation_smoke as files
from evolution_smoke import Journey as EvolutionJourney
from native_connection_smoke import MODEL, Peer, until
from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available
from workbench_smoke import wait_for

ESC = '\x1b'
PAGE = ESC + '[6~'
LADDER = ['low', 'medium', 'high', 'xhigh', 'max']
CATALOGUE = {'data': [
    {'id': MODEL, 'name': MODEL, 'supported_reasoning_levels': LADDER,
     'default_reasoning_level': 'medium'},
    {'id': 'fixture-sparse', 'name': 'fixture-sparse',
     'supported_reasoning_levels': ['low', 'xhigh'], 'default_reasoning_level': 'low'},
    {'id': 'fixture-single', 'name': 'fixture-single',
     'supported_reasoning_levels': ['high'], 'default_reasoning_level': 'high'},
    {'id': 'fixture-fixed', 'name': 'fixture-fixed'},
], 'has_more': False}

# Each expected observation is an openness rule, never a semantic model score.
CASES = [
    ('Rename count to total in every supplied file, please.', 'trivial', 'low', 'given_fix'),
    ('변수 이름을 count에서 total로 변경해줘.', 'trivial', 'low', 'given_fix'),
    ('Compare two designs for the retry API.', 'moderate', 'high', 'bounded_alternatives'),
    ('두 가지 설계안을 비교해줘.', 'moderate', 'high', 'bounded_alternatives'),
    ('Investigate why the concurrent cache returns stale results.', 'hard', 'xhigh', 'open_cause'),
    ('동시성 캐시가 간헐적으로 잘못되는 원인을 조사해줘.', 'hard', 'xhigh', 'open_cause'),
    ('Design and prove subtle exactly-once invariants.', 'hard', 'xhigh', 'subtle_invariants'),
    ('The fix is given: rename old to new; investigate why the race is flaky.', 'trivial', 'low', 'given_fix'),
    ('Compare two designs and investigate why the concurrent cache is stale.', 'moderate', 'high', 'bounded_alternatives'),
    ('Please ultrathink, then answer this 한글 request.', None, 'max', 'ultrathink'),
    ('continue', None, 'high', 'keep_last'),
    ('Please inspect `ultrathink` only.', None, 'high', 'keep_last'),
    ('Explain ultrathinking and ultrathink.rs only.', None, 'high', 'keep_last'),
    ('Quoted source says "ultrathink"; just acknowledge.', None, 'high', 'keep_last'),
    ('Please say ULTRATHINK only.', None, 'high', 'keep_last'),
]


class Journey(EvolutionJourney):
    def choose_saved_model(self, expected_metadata):
        session = self.session
        wait_for(session, MODEL)
        until(session, lambda: len(self.peer.snapshot('GET')) == expected_metadata
              and self.surface['loading'] not in session.display(), 'controlled effort catalogue ready')
        session.send(MODEL)
        wait_for(session, self.surface['search'] + MODEL)
        wait_for(session, self.surface['matches'])
        session.send('\r')
        if 'Reasoning · ' in session.display():
            session.send('\r')
        self.workspace()
        assert len(self.peer.snapshot('GET')) == expected_metadata

    def records(self):
        database = self.data / 'workbench.sqlite3'
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=1)) as db:
            db.execute('PRAGMA query_only=ON')
            db.row_factory = sqlite3.Row
            receipts = [dict(row) for row in db.execute('SELECT * FROM outgoing_receipts ORDER BY rowid')]
            preferences = db.execute('SELECT record FROM provider_preferences WHERE id=1').fetchone()
            pairings = [json.loads(row[0]) for row in db.execute('SELECT record FROM partner_pairings ORDER BY rowid')]
            schema = db.execute('PRAGMA user_version').fetchone()[0]
        for row in receipts:
            row['record'] = json.loads(row['record'])
        return {'receipts': receipts, 'preferences': json.loads(preferences[0]) if preferences else None,
                'pairings': pairings, 'schema': schema}

    def saved_effort(self):
        preferences = self.records()['preferences']
        return preferences and preferences.get('selections', {}).get('Compatible', {}).get('effort')

    def effort(self, value):
        before = len(self.peer.snapshot('POST'))
        self.session.command('effort ' + value)
        expected = {'mode': 'auto'} if value == 'auto' else (None if value == 'default' else value)
        until(self.session, lambda: self.saved_effort() == expected,
              'effort selection did not commit through the existing preference writer', timeout=35)
        self.workspace()
        assert len(self.peer.snapshot('POST')) == before, 'local effort selection sent a request'

    def model(self, value):
        before = len(self.peer.snapshot('POST'))
        self.session.command('chat-new')
        self.workspace()
        self.session.command('model ' + value)
        wait_for(self.session, 'Reasoning · ')
        advertised = next(model for model in CATALOGUE['data'] if model['id'] == value)
        # This picker owns its keys: choose the actual advertised auto row,
        # rather than attempting a global command through its Enter handler.
        self.session.send(ESC + '[H' + (ESC + '[B') * (len(advertised.get('supported_reasoning_levels', [])) + 1))
        assert any('> auto' in ' '.join(line.split()) for line in self.session.display().splitlines()), self.session.display()
        self.session.send('\r')
        until(self.session, lambda: self.records()['preferences']['selections']['Compatible']['model'] == value
              and self.saved_effort() == {'mode': 'auto'}, 'exact model/auto selection did not commit')
        self.workspace()
        assert len(self.peer.snapshot('POST')) == before

    def save_pairing(self, value):
        # A pairing is offered only after a fresh explicit local choice in a
        # fixed appearance. Restart does not manufacture or restore that offer.
        self.session.command('model ' + MODEL)
        self.effort(value)
        wait_for(self.session, '/save-pairing')
        self.session.command('save-pairing')
        expected = {'mode': 'auto'} if value == 'auto' else value
        until(self.session, lambda: any(row['selection']['model'] == MODEL
              and row['selection']['effort'] == expected for row in self.records()['pairings']),
              'explicit effort pairing did not commit through the existing writer', timeout=35)
        self.workspace()

    def inspect(self, command, label):
        before = len(self.peer.snapshot('POST'))
        self.session.command(command)
        title = 'Next evolution' if command == 'evolution-preview' else 'Outgoing receipts'
        wait_for(self.session, title)
        views = []
        screen = self.session.screen
        top = next(index for index, line in enumerate(screen.display) if title in line)
        if command == 'evolution-preview':
            start, end, bottom_mark = ('+', '+', '+') if self.restart_size[0] == 60 else ('╔', '╗', '╚')
        else:
            start, end, bottom_mark = '┌', '┐', '└'
        left = next(x for x in range(screen.columns) if screen.buffer[top][x].data == start)
        right = max(x for x in range(screen.columns) if screen.buffer[top][x].data == end)
        bottom = next(y for y in range(top + 1, screen.lines) if screen.buffer[y][left].data == bottom_mark)
        for index in range(8):
            # Read observed terminal cells at the inspector's rectangle. String
            # indices are not columns when the background contains wide Korean.
            for y in range(top + 1, bottom):
                views.append(''.join(screen.buffer[y][x].data for x in range(left + 1, right)))
            self.capture(label + '-page-' + str(index))
            self.session.send(PAGE)
        if command == 'evolution-preview':
            self.session.send('\r')  # EV19 must not own the hidden Send
        assert len(self.peer.snapshot('POST')) == before, 'inspection dispatched a request'
        self.session.send(ESC)
        return ''.join(''.join(views).split())

    def request(self, text, bucket, resolved, reason, *, auto=True, busy=False):
        before = len(self.peer.snapshot('POST'))
        self.write_request(text)
        caret = (self.session.screen.cursor.x, self.session.screen.cursor.y)
        preview = self.inspect('evolution-preview', 'preview-' + str(before))
        assert 'Effort:' in preview and ('auto' in preview if auto else 'high' in preview), preview
        if auto:
            assert (resolved or 'providerdefault') in preview, preview
            if bucket:
                assert bucket in preview, preview
            assert ('notapplied' in preview if resolved is None else 'notacknowledged' in preview), preview
            matched_reason = {
                'given_fix': 'fixortargetgiven',
                'bounded_alternatives': 'boundedcandidatecausesordesigns',
                'open_cause': 'opencauseofflaky,concurrentorstalebehavior',
                'subtle_invariants': 'subtlecorrectnessinvariants',
                'ultrathink': 'standalonelowercaseproseultrathink',
                'keep_last': 'ambiguous;keeplastresolution',
                'model_default': 'ambiguous;initialmodeldefault',
            }[reason]
            assert matched_reason in preview, (reason, preview)
        assert (self.session.screen.cursor.x, self.session.screen.cursor.y) == caret, 'preview moved the Request caret'
        assert len(self.records()['receipts']) == before, 'preview persisted a dispatched receipt'
        gate = files.Gate()
        answer = 'EFFORT_PUBLIC_REPLY_' + str(before)
        self.peer.plans.put(files.response_plan(self.peer, text=answer, number=before, gate=gate))
        approved = False
        self.session.send('\r', settle=False)

        def dispatched():
            nonlocal approved
            if len(self.peer.snapshot('POST')) == before + 1:
                return True
            if not approved and permission_run_available(self.session):
                self.capture('explicit-connection-scope-' + str(before))
                self.session.send('y', settle=False)
                approved = True
            elif not approved:
                inspect_permission_scope(self.session)
            return False

        try:
            until(self.session, dispatched, 'explicit effort request did not reach the real loopback wire', timeout=40)
            until(self.session, lambda: gate.ready.is_set(), 'loopback completion gate was not reached')
            until(self.session, lambda: len(self.records()['receipts']) == before + 1,
                  'matching transport event did not commit frozen outgoing receipt')
            assert self.snapshot()['claims'], 'request bypassed durable ownership'
            wire = self.peer.snapshot('POST')[before]
            received = wire['body'].get('reasoning', {}).get('effort')
            assert received == resolved, (text, received, resolved, wire['body'])
            row = self.records()['receipts'][-1]
            raw = bytes.fromhex(wire['body_bytes_hex'])
            assert row['source'].encode('utf-8') == raw, 'reviewed receipt differs from actual HTTP request bytes'
            record = row['record']
            assert record['bytes'] == len(raw)
            assert bytes(record['sha256']) == hashlib.sha256(raw).digest()
            if auto:
                expected = {'bucket': bucket, 'resolved': resolved, 'reason': reason, 'source': 'user_request',
                            'application': 'fixed_by_provider_not_applied' if resolved is None else 'requested_not_acknowledged'}
                assert record['effort'] == expected, (text, record['effort'], expected)
                assert record['selection']['effort'] == {'mode': 'auto'}
            else:
                assert 'effort' not in record, 'fixed selection invoked the automatic classifier'
                assert record['selection']['effort'] == 'high'
            assert 'effort_resolution' not in wire['body'], 'local classifier metadata leaked into the protocol'
            if busy:
                self.session.send(ESC + 'a', settle=False)
                self.session.paste('ultrathink future draft 한글')
                self.session.send('\r', settle=False)
                self.session.pump(.2)
                assert len(self.peer.snapshot('POST')) == before + 1, 'busy future draft dispatched another request'
                assert self.records()['receipts'][-1] == row, 'busy future draft changed the frozen receipt'
                self.capture('busy-followup-keeps-frozen-effort')
            self.capture('wire-receipt-' + str(before))
        finally:
            gate.release.set()
        self.settled(answer, before + 1)
        receipt_view = self.inspect('receipts', 'receipt-' + str(before))
        assert 'Effort:' in receipt_view
        assert 'notacknowledged' in receipt_view if resolved is not None else 'notapplied' in receipt_view
        return {'request': text, 'bucket': bucket, 'resolved': resolved, 'reason': reason,
                'request_id': record['request_id'], 'wire_sha256': hashlib.sha256(raw).hexdigest(),
                'preview_and_actual_wire_and_receipt_agree': True}

    def stop(self):
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()

    def previous_refusal(self, binary, label):
        if binary is None:
            return {'status': 'not exercised', 'reason': '--old-binary not supplied'}
        clone = self.root / ('previous-' + label)
        shutil.copytree(self.data, clone)
        database = clone / 'workbench.sqlite3'
        before = database.read_bytes()
        session = SurfaceSession([str(binary), '--data-dir', str(clone), '--images', 'off', '--color', 'mono'],
                                 self.project, self.restart_environment, self.restart_size,
                                 self.output / ('old-reader-' + label))
        try:
            wait_for(session, 'Storage: save failed', timeout=35)
            session.capture('strict-auto-format-refused')
            observed = session.display()
            assert any(term in observed.lower() for term in ('string', 'map', 'preference', 'pairing', 'record')), observed
        finally:
            session.close()
        assert database.read_bytes() == before, 'previous reader rewrote persisted auto data'
        return {'status': 'refused', 'database_sha256': hashlib.sha256(before).hexdigest(),
                'byte_identical': True, 'schema': self.records()['schema'], 'reason_screen': observed}

    def resume(self, label):
        self.session = SurfaceSession(self.restart_argv, self.project, self.restart_environment,
                                      self.restart_size, self.output / label)
        wait_for(self.session, 'Storage: saved', timeout=40)
        self.workspace()


def run(binary, old_binary, output, root, size, ascii_mode):
    output.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'size': list(size), 'rule_observations': []}
    journey = None
    with Peer('responses', 'unused-public-keyless-effort-fixture', True, retain_body_bytes=True) as peer:
        peer.catalogue = CATALOGUE
        try:
            journey = Journey(binary, output, root, size, ascii_mode, 'compatible', 'responses', True, True, peer)
            wait_for(journey.session, 'Storage: saved')
            journey.session.command('goal')
            journey.session.paste('Local effort acceptance 한글')
            journey.session.send('\r')
            wait_for(journey.session, 'Storage: saved')
            journey.workspace()
            journey.setup_route()
            journey.session.command('role-plan')
            journey.workspace()
            journey.effort('auto')
            assert not journey.records()['pairings'] and not journey.records()['receipts']
            journey.stop()
            result['preference_reader_refusal'] = journey.previous_refusal(old_binary, 'preference')
            journey.resume('restart-auto-preference')
            assert journey.saved_effort() == {'mode': 'auto'}, 'auto selection did not survive restart'
            before = len(peer.snapshot('POST'))
            journey.session.command('espeon')
            journey.workspace()
            journey.setup_route()  # explicit metadata setup, never a restored session
            journey.save_pairing('auto')
            journey.effort('high')
            journey.stop()
            result['pairing_reader_refusal'] = journey.previous_refusal(old_binary, 'pairing')
            journey.resume('restart-fixed-preference-and-auto-pairing')
            assert journey.saved_effort() == 'high', 'fixed effort did not survive restart'
            assert any(row['selection']['effort'] == {'mode': 'auto'} for row in journey.records()['pairings'])
            assert len(peer.snapshot('POST')) == before, 'restart or pairing automatically sent a request'
            journey.setup_route()
            journey.effort('auto')
            journey.session.command('role-plan')
            journey.workspace()
            for index, row in enumerate(CASES):
                result['rule_observations'].append(journey.request(*row, busy=index == 0))
            for model, row in [
                ('fixture-sparse', ('Investigate why the concurrent cache is stale.', 'hard', 'low', 'open_cause')),
                ('fixture-single', ('Rename count to total.', 'trivial', 'high', 'given_fix')),
                ('fixture-fixed', ('동시성 캐시의 원인을 조사해줘.', 'hard', None, 'open_cause')),
            ]:
                journey.model(model)
                result['rule_observations'].append(journey.request(*row))
            journey.model(MODEL)
            journey.effort('high')
            result['fixed_wire'] = journey.request('ultrathink; investigate why the cache is stale.', None, 'high', None, auto=False)
            journey.save_pairing('high')
            pairings_before = journey.records()['pairings']
            receipt_before = journey.records()['receipts']
            posts = len(peer.snapshot('POST'))
            journey.stop()
            journey.resume('restart-exact-frozen-receipts')
            assert journey.records()['receipts'] == receipt_before
            assert journey.saved_effort() == 'high'
            assert journey.records()['pairings'] == pairings_before, 'fixed pairing did not survive restart'
            assert len(peer.snapshot('POST')) == posts, 'restart replayed inference'
            journey.stop()
            peer.assert_clean()
            result.update(status='passed', actual_explicit_requests=posts,
                          auto_and_fixed_restart=True, auto_and_fixed_pairing_restart=True,
                          no_automatic_dispatch=True)
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failure')
        finally:
            if journey:
                journey.session.close()
            files.save(output / 'wire-observations.json', peer.snapshot())
            files.save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--old-binary', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    old = args.old_binary.resolve() if args.old_binary else None
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    result = {'status': 'failed', 'binary_sha256': digest, 'journeys': [],
              'old_binary_sha256': hashlib.sha256(old.read_bytes()).hexdigest() if old else None,
              'limits': 'Deterministic loopback fixtures and actual terminal/store/wire observations only; no live or paid model interpretation.'}
    for size, ascii_mode in [((120, 36), False), ((60, 18), True)]:
        with tempfile.TemporaryDirectory(prefix='ev-effort-한글-') as temporary:
            result['journeys'].append(run(binary, old, output / str(size[0]), Path(temporary).resolve(), size, ascii_mode))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    result['status'] = 'passed' if all(row['status'] == 'passed' for row in result['journeys']) else 'failed'
    files.save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
