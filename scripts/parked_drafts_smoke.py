#!/usr/bin/env python3
"""Opt-in RX04: real PTY, existing writer, loopback provider; no paid/live calls.

Run only against an explicitly supplied already-built executable. All record
creation uses UI. One BEGIN IMMEDIATE transaction injects writer contention; it
changes no SQL record. Fixture data is retained under --output for compatibility
inspection, including actual durable parked entries, never synthesized SQL.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import threading
import traceback

import evolution_smoke
import file_creation_smoke as files
from workbench_smoke import wait_for

FIRST = 'First parked 한글 e\u0301 follow-up'
SECOND = 'Second independent future request 界'
CURRENT = 'Current replacement review draft 한글'

class Gate:
    def __init__(self):
        self.ready = threading.Event()
        self.release = threading.Event()
    def hold(self, peer):
        self.ready.set()
        for _ in range(3000):
            if self.release.wait(.03):
                return True
            if peer.stopped.is_set():
                return False
        raise AssertionError('parked-draft runtime gate not released')


def entries(journey):
    tasks = journey.snapshot()['tasks']
    return json.loads(tasks[0]['workflows']).get('parked_drafts', []) if tasks else []


def current_request(journey):
    tasks = journey.snapshot()['tasks']
    if not tasks:
        return None
    task = tasks[0]
    state = task['partner_drafts'].get(str(task['lead_partner']), {})
    return next((draft for draft in state.get('drafts', []) if draft['field'] == 'Request'), None)


def park(journey, text, count):
    journey.write_request(text)
    journey.session.command('park-draft')
    files.until(journey.session, lambda: len(entries(journey)) == count
                and 'Storage: saved' in journey.session.display()
                and (current_request(journey) is None or current_request(journey)['text'] == ''),
                'parking did not durably acknowledge and clear the exact Request', timeout=30)
    journey.capture('parked-' + str(count))


def review_all(journey):
    # Scroll each individual row; never skip unobserved scope. Extra Down at
    # the exact bottom remains neutral and cannot arm a replacement by itself.
    for _ in range(160):
        journey.session.send('\x1b[B', settle=False)
        journey.session.pump(.015)
    journey.session.send('\x1b[C')


def run(binary, output, size, ascii_mode):
    output.mkdir(parents=True, exist_ok=False)
    root = output / 'fixture'
    root.mkdir()
    result = {'status': 'failed', 'size': list(size), 'cases': []}
    journey = None
    gate = Gate()
    with files.Peer('responses', 'sk-parked-fixture-only', True, retain_body_bytes=True) as peer:
        try:
            journey = evolution_smoke.Journey(binary, output, root, size, ascii_mode,
                'compatible', 'responses', True, False, peer)
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            journey.session.command('role-plan')
            before = journey.begin('Explain this project without changing any files.',
                [files.response_plan(peer, text='RX04_RUNNING_COMPLETE', gate=gate)], writing=False)
            files.until(journey.session, gate.ready.is_set, 'provider did not remain running')
            park(journey, FIRST, 1)
            park(journey, SECOND, 2)
            assert len(peer.snapshot('POST')) == before + 1
            assert journey.snapshot()['claims'], 'parking unexpectedly completed/cancelled owned work'
            gate.release.set()
            journey.settled('RX04_RUNNING_COMPLETE', before + 1)
            assert len(entries(journey)) == 2
            result['cases'].append('two drafts parked while the original request really ran; completion sent neither')
            journey.restart('restart-with-two-drafts')
            assert len(entries(journey)) == 2
            assert len(peer.snapshot('POST')) == before + 1
            journey.session.command('party')
            journey.session.send('a')
            files.until(journey.session, lambda: len(journey.snapshot()['partners']) == 2, 'partner add did not commit')
            journey.session.command('lead 2')
            files.until(journey.session, lambda: journey.snapshot()['tasks'][0]['lead_partner'] == 2, 'owner switch did not commit')
            journey.session.command('drafts')
            wait_for(journey.session, 'No parked drafts')
            journey.session.command('lead 1')
            journey.workspace()
            assert len(peer.snapshot('POST')) == before + 1
            result['cases'].append('restart and owner switching restored no sends, sessions or grants')
            journey.write_request(CURRENT)
            journey.session.send('\x1b[D')
            journey.session.send('\x13')
            files.until(journey.session, lambda: current_request(journey) and current_request(journey)['text'] == CURRENT, 'current review draft not saved')
            caret = current_request(journey)['cursor']
            journey.session.command('drafts')
            journey.session.send('r')
            wait_for(journey.session, 'Replace the exact current Request?')
            journey.capture('replacement-neutral')
            journey.session.send('\r')
            journey.session.send(files.ESC)
            journey.session.send('\x13')
            files.until(journey.session, lambda: current_request(journey) and current_request(journey)['cursor'] == caret, 'neutral replacement changed caret')
            assert current_request(journey)['text'] == CURRENT and len(entries(journey)) == 2
            journey.session.command('drafts')
            journey.session.send('r')
            review_all(journey)
            journey.session.send('\x0b')
            journey.session.send(files.ESC)
            journey.session.send('\r')
            journey.session.send(files.ESC)
            assert current_request(journey)['text'] == CURRENT
            assert current_request(journey)['cursor'] == caret
            assert len(entries(journey)) == 2 and len(peer.snapshot('POST')) == before + 1
            journey.capture('obscured-review-revokes-replacement')
            result['cases'].append('a fully inspected replacement loses consent when obscured by the action chooser')
            journey.session.command('drafts')
            journey.session.send('r')
            journey.session.send(files.ESC)
            journey.session.send(files.ESC)
            assert len(peer.snapshot('POST')) == before + 1
            result['cases'].append('neutral Enter and Escape kept exact current/parked text and caret')
            # Actual store contention creates a bounded failed save, not a fake
            # app event or a seeded task. Original text remains in the live UI.
            journey.write_request('Recoverable after writer contention 한글')
            with sqlite3.connect(journey.data / 'workbench.sqlite3', timeout=1) as lock:
                lock.execute('BEGIN IMMEDIATE')
                journey.session.command('park-draft')
                files.until(journey.session, lambda: 'save failed' in journey.session.display().lower(), 'writer contention did not surface a save failure', timeout=15)
                journey.capture('failed-save-original-recoverable')
                lock.rollback()
            journey.session.command('storage-retry')
            files.until(journey.session, lambda: len(entries(journey)) == 3 and
                        entries(journey)[-1]['text'] == 'Recoverable after writer contention 한글',
                        'storage-only retry did not durably retain the exact parked candidate', timeout=40)
            assert len(peer.snapshot('POST')) == before + 1
            result['cases'].append('real writer save failure retained recoverable text; storage retry never resent')
            journey.session.command('drafts')
            journey.session.send('r')
            if 'Replace the exact current Request?' in journey.session.display():
                review_all(journey)
                journey.session.send('\r')
            journey.session.send('\x13')
            files.until(journey.session, lambda: current_request(journey) and current_request(journey)['text'] == FIRST, 'explicit restore did not stage the exact first draft')
            assert len(peer.snapshot('POST')) == before + 1
            # Session-only connection setup is intentionally not restored.
            # Reconnect explicitly before proving a fresh ordinary Send.
            journey.setup_route()
            assert current_request(journey)['text'] == FIRST
            assert len(peer.snapshot('POST')) == before + 1
            peer.plans.put(files.response_plan(peer, text='RX04_EXPLICIT_SEND_COMPLETE'))
            journey.session.send('\r')
            files.until(journey.session, lambda: len(peer.snapshot('POST')) == before + 2, 'ordinary explicit Send did not dispatch fresh current context', timeout=30)
            journey.settled('RX04_EXPLICIT_SEND_COMPLETE', before + 2)
            body = json.dumps(files.decoded_payload('responses', peer.snapshot('POST')[-1]['body']), ensure_ascii=False)
            assert FIRST in body and SECOND not in body, 'tray leaked unrelated parked text into current context'
            assert len(entries(journey)) == 3
            result['cases'].append('restore staged only; ordinary explicit Send used current context without other parked entries')
            result['data_dir'] = str(journey.data)
            result['project'] = str(journey.project)
            journey.session.command('quit')
            files.confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            result['status'] = 'passed'
        except Exception:
            result['error'] = traceback.format_exc()
            if journey:
                journey.capture('failure')
        finally:
            gate.release.set()
            if journey:
                journey.session.close()
            result['posts'] = len(peer.snapshot('POST'))
            result['peer_errors'] = peer.errors
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
    results = [run(binary, output / str(size[0]), size, ascii_mode)
               for size, ascii_mode in [((120, 36), False), ((60, 18), True)]]
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    result = {'status': 'passed' if all(r['status'] == 'passed' for r in results) else 'failed',
              'binary': str(binary), 'sha256': digest, 'journeys': results}
    files.save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'output': str(output)}))
    return 0 if result['status'] == 'passed' else 1

if __name__ == '__main__':
    raise SystemExit(main())
