#!/usr/bin/env python3
"""T01C actual native send/restart/inspect/purge acceptance against a built binary.

Uses the existing PTY/ConPTY and deterministic loopback HTTP fixtures. No builds,
SQL writes, provider credentials, paid calls or live-provider proof. Store unit
regressions separately exercise forged wrong-owner read/purge and stale retries.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import platform
import sqlite3
import tempfile
import traceback

from moves_smoke import digest, save
from records_smoke import Journey, RecordsSession, ESC, LEFT, RIGHT, PAGE
from native_connection_smoke import Peer
from workbench_smoke import wait_for


class ReceiptJourney(Journey):
    def launch(self):
        self.serial += 1
        quota = getattr(self, 'quota_kib', None)
        self.session = RecordsSession([
            str(self.binary), '--data-dir', str(self.data), '--images', 'off',
            '--color', 'mono', *(['--ascii'] if self.ascii_mode else []),
            *(['--quota-kib', str(quota)] if quota is not None else []),
        ], self.project, self.environment, self.size,
            self.output / ('terminal-%02d' % self.serial), controlling_tty=True)
        self.sessions.append(self.session)
        wait_for(self.session, 'Storage: saved', timeout=35)

    def state(self):
        state = super().state()
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True, timeout=1)) as db:
            db.execute('PRAGMA query_only=ON')
            db.row_factory = sqlite3.Row
            state['outgoing_receipts'] = [dict(row) for row in db.execute(
                'SELECT * FROM outgoing_receipts ORDER BY rowid')]
            for row in state['outgoing_receipts']:
                row['record'] = json.loads(row['record'])
        return state

    def receipt(self):
        rows = self.state()['outgoing_receipts']
        assert len(rows) == 1, 'one actual send must produce one durable section receipt'
        return rows[0]

    def inspector(self):
        self.session.command('receipts')
        self.wait_caption('Outgoing receipts', )
        self.until(lambda: '1/1' in self.session.display(), 'restored receipt inspection')
        return self.session.display()

    def run_receipts(self):
        self.connect()
        self.form('espeon')
        capture = self.pin_range('calc.py')
        instruction = 'Explain this selected source without edits; receipt marker cedar-927 東京.'
        wire = self.send_request(instruction, 'Fixture receipt public response cedar-927.')
        self.until(lambda: len(self.state()['outgoing_receipts']) == 1,
                   'actual transport receipt commit before ownership release')
        before = self.receipt()
        record = before['record']
        raw = bytes.fromhex(wire['body_bytes_hex'])
        assert before['source'].encode('utf-8') == raw, 'receipt must retain exact reviewed outgoing HTTP body'
        assert record['bytes'] == len(raw)
        assert bytes(record['sha256']) == hashlib.sha256(raw).digest()
        assert sum(section['bytes'] for section in record['sections']) + record['envelope_bytes'] == len(raw)
        assert record['captures'] and any(ref['id'] == capture['id'] for ref in record['captures'])
        assert record['task_id'] == self.task()['id'] and record['owner']['partner'] == self.task()['lead_partner']
        assert record['request_id'] == before['request_id']
        self.capture('actual-send-exact-wire-and-section-receipt')
        self.restart()
        assert self.receipt() == before, 'restart must preserve original count/source/owner identities'
        self.inspector()
        self.capture('restart-restored-receipt-inspector')
        self.session.send('s')
        self.wait_caption('Exact application envelope.', )
        self.until(lambda: 'messages' in self.session.display() or '"model"' in self.session.display(),
                   'actual retained wire envelope visible')
        self.capture('restored-exact-application-envelope')
        self.session.send(ESC)
        self.session.send('p')
        self.wait_caption('Purge this request', )
        self.capture('neutral-source-purge-review')
        self.session.send('\r')
        assert self.receipt() == before, 'neutral Enter must leave source untouched'
        self.session.send('p')
        self.wait_caption('Purge this request', )
        self.session.send(RIGHT + ESC)
        assert self.receipt() == before, 'cancel after selecting Purge must leave source untouched'
        self.session.send('p')
        self.wait_caption('Purge this request', )
        self.session.send(RIGHT + '\r')
        self.until(lambda: self.receipt()['source'] is None, 'committed source bytes purged')
        assert self.receipt()['record'] == record, 'purge must not rewrite identity, sections, references or counts'
        self.wait_caption('UNAVAILABLE', )
        self.capture('purged-source-unavailable-counts-retained')
        self.session.send('s')
        self.wait_caption('UNAVAILABLE', )
        assert self.receipt()['source'] is None
        self.session.send(ESC)
        self.workspace()
        draft = 'Future unrelated draft 東京 exact caret marker'
        self.write_request(draft)
        self.session.send(LEFT * 5)
        expected_cursor = len(draft.encode('utf-8')) - 5
        self.session.send('\x13')
        self.until(lambda: any(d['field'] == 'Request' and d['text'] == draft
                   and d['cursor'] == expected_cursor
                   for entry in self.task()['partner_drafts'].values() for d in entry.get('drafts', []))
                   and 'Storage: saved' in self.session.display(),
                   'exact moved Request caret durably acknowledged before inspection baseline')
        drafts_before = self.task()['partner_drafts']
        self.capture('exact-draft-and-middle-caret-committed-before-inspection')
        self.inspector()
        self.session.send(ESC)
        self.workspace()
        self.session.send('\x13')
        self.until(lambda: self.task()['partner_drafts'] == drafts_before
                   and 'Storage: saved' in self.session.display(),
                   'full saved draft content, middle caret and editor state preserved after receipt inspection')
        save(self.output / 'receipt-draft-caret-boundary.json', {
            'expected_request': draft, 'expected_cursor_utf8': expected_cursor,
            'before': drafts_before, 'after': self.task()['partner_drafts'],
            'inspection_made_provider_call': len(self.peer.snapshot('POST')) != 1,
        })
        self.session.paste('Z')
        self.session.send('\x13')
        expected = draft[:-5] + 'Z' + draft[-5:]
        self.until(lambda: any(d['field'] == 'Request' and d['text'] == expected
                   for entry in self.task()['partner_drafts'].values() for d in entry.get('drafts', [])),
                   'exact invoking Request draft and caret preserved')
        self.restart()
        assert self.receipt()['source'] is None and self.receipt()['record'] == record
        self.inspector()
        self.wait_caption('UNAVAILABLE', )
        self.capture('second-restart-unavailable-original-counts')
        assert len(self.peer.snapshot('POST')) == 1, 'inspection/restart/purge must never replay a provider call'
        self.session.send(ESC)
        self.workspace()
        self.quit()
        return {'request_id': record['request_id'], 'task_id': record['task_id'],
                'partner': record['owner']['partner'], 'wire_bytes': len(raw),
                'source_available_after_purge': False, 'counts_unchanged': True,
                'actual_inference_posts': 1, 'evidence': self.evidence}

    def recovery_review(self):
        self.session.send('d')
        self.wait_caption('This submitted request has no saved receipt.',
                          )
        # Each real frame exposes the next contiguous page; no End shortcut or
        # hidden authorization. Controls remain neutral until all rows are seen.
        for _ in range(16):
            self.session.send(PAGE)

    def run_quota_recovery(self):
        self.connect()
        self.form('espeon')
        for filename in ('calc.py', 'quota_receipt_second.py'):
            text = '# quota evidence ' + filename + ' ' + ('public bounded source observation ' * 1750) + '\n'
            assert len(text.encode()) < 65536
            (self.project / filename).write_text(text, encoding='utf-8')
        self.session.command('diff')
        wait_for(self.session, 'Repository', timeout=35)
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.home_surface()
        self.pin_range('calc.py')
        self.pin_range('quota_receipt_second.py')
        instruction = 'Explain these two public observations only; quota recovery marker cedar-927.'
        self.write_request(instruction)
        self.quit()
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True)) as db:
            pages = db.execute('PRAGMA page_count').fetchone()[0]
            free = db.execute('PRAGMA freelist_count').fetchone()[0]
            page_size = db.execute('PRAGMA page_size').fetchone()[0]
            live = (pages - free) * page_size
        # Enough headroom for the small work/usage/public-response journals, but
        # not the >110 KiB exact outgoing source copy. Real SQLite admission and
        # rollback enforce pressure; the driver never changes database rows.
        self.quota_kib = (live + 64 * 1024 + 1023) // 1024
        self.launch()
        self.connect()
        self.workspace()
        self.write_request(instruction)
        self.peer.plans.put(('complete', 'Fixture definite terminal quota response.'))
        self.session.send('\r')
        self.until(lambda: len(self.peer.snapshot('POST')) == 1, 'one actual quota fixture submission')
        wait_for(self.session, 'Fixture definite terminal quota response.', timeout=35)
        wire = self.peer.snapshot('POST')[0]
        assert len(bytes.fromhex(wire['body_bytes_hex'])) > 110 * 1024
        self.session.command('receipts')
        self.wait_caption('Outgoing receipts', )
        self.wait_caption('Storage/read failed', )
        assert not self.state()['outgoing_receipts'], 'quota failure must not claim a saved receipt'
        assert self.state()['active_work'], 'failed receipt must initially retain ownership'
        self.capture('real-quota-failed-receipt-retains-ownership')
        self.recovery_review()
        self.capture('exact-owner-error-bound-neutral-recovery')
        self.session.send('\r')
        assert self.state()['active_work'], 'neutral Enter discarded the failed write'
        self.recovery_review()
        self.session.send(RIGHT + ESC)
        assert self.state()['active_work'], 'cancel discarded the failed write'
        self.recovery_review()
        self.session.send(RIGHT + '\r')
        self.until(lambda: not self.state()['active_work'],
                   'reviewed unsaved-gap acknowledgement permits exact ownership release')
        assert not self.state()['outgoing_receipts'], 'gap acknowledgement fabricated a durable receipt'
        self.wait_caption('UNSAVED RECEIPT GAP', )
        self.capture('explicit-memory-only-gap-no-provider-replay')
        assert len(self.peer.snapshot('POST')) == 1
        self.session.send(ESC)
        self.workspace()
        self.restart()
        assert not self.state()['outgoing_receipts'] and not self.state()['active_work']
        self.session.command('receipts')
        self.wait_caption('No retained outgoing receipts', )
        self.capture('restart-honest-missing-receipt-no-fabricated-counts')
        assert len(self.peer.snapshot('POST')) == 1
        self.session.send(ESC)
        self.workspace()
        self.quit()
        return {'quota_kib': self.quota_kib, 'actual_inference_posts': 1,
                'durable_receipt_count': 0, 'explicit_reviewed_gap': True,
                'ownership_released': True, 'evidence': self.evidence}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--quota-recovery', action='store_true',
                        help='exercise genuine post-submission SQLite quota rollback and reviewed gap recovery')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    assert binary.is_file(), 'use an existing already-built binary'
    output.mkdir(parents=True, exist_ok=False)
    original = digest(binary)
    result = {'status': 'failed', 'binary': str(binary), 'binary_sha256': original,
              'host': platform.platform(), 'runs': [],
              'scope': 'Actual terminal send/restart/inspect/purge with deterministic loopback HTTP only',
              'live_provider_proof': False, 'sql_writes': False}
    try:
        for size, ascii_mode in (((120, 36), False), ((60, 18), True)):
            name = '%dx%d' % size
            with tempfile.TemporaryDirectory(prefix='ev-receipts-') as temporary:
                with Peer('chat_completions', 'unused-keyless-public-fixture', True,
                          retain_body_bytes=True) as peer:
                    journey = ReceiptJourney.__new__(ReceiptJourney)
                    try:
                        journey.__init__(binary, output / name, Path(temporary), size,
                                         ascii_mode, {}, peer, )
                        proof = journey.run_quota_recovery() if args.quota_recovery else journey.run_receipts()
                        result['runs'].append({'size': list(size), 'status': 'passed', **proof})
                    finally:
                        if hasattr(journey, 'sessions'):
                            journey.close()
            assert digest(binary) == original, 'immutable executable changed during acceptance'
        result['status'] = 'passed'
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'results': str(output / 'results.json')}))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
