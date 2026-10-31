#!/usr/bin/env python3
"""PK08 real application/PTY acceptance; isolated homes, read-only SQL observations.

Run each terminal size explicitly, e.g. --size 60x18. No SQL
history seeding, live accounts, paid models, rename search or automatic dispatch.
The loopback peer captures an actual ordinary request for frozen-input isolation.
Writer-CAS exclusion/revocation/owner/configuration races also have focused Rust
regressions; this driver exercises their user-visible cancellation boundaries.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import traceback

import held_smoke as held
import file_creation_smoke as files
from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
PGDN = ESC + '[6~'
RIGHT = ESC + '[C'


class Journey(held.Journey):
    def target(self):
        row = self.slot()
        return json.loads(row['target']) if row else None

    def reject_file(self, path):
        self.workspace()
        before, basket = self.held_rows(), self.contexts()
        self.session.command('held-file')
        files.until(self.session, lambda: any(value in self.session.display()
                    for value in ('eligible files /', )),
                    'actual source chooser did not load', timeout=35)
        self.session.command('search')
        self.session.send(held.UP * 64 + held.HOME + held.DELETE * 4096)
        self.session.paste(path)
        self.session.send('\r')
        wait_for(self.session, '0 eligible files /', timeout=35)
        self.session.command('held-set-file')
        self.session.pump(.2)
        assert self.held_rows() == before and self.contexts() == basket
        self.capture('excluded-replacement-keeps-independent-context')
        self.workspace()

    def inspect_file(self, path):
        self.workspace()
        self.session.command('held-file')
        files.until(self.session, lambda: any(value in self.session.display()
                    for value in ('eligible files /', )),
                    'actual source chooser did not load', timeout=35)
        self.session.command('search')
        self.session.send(held.UP * 64 + held.HOME + held.DELETE * 4096)
        self.session.paste(path)
        self.session.send('\r')
        wait_for(self.session, '1 eligible files /', timeout=35)
        self.session.send('\r')
        wait_for(self.session, 'Review local capture /', timeout=35)
        self.session.command('held-set-file')
        source = (self.project / path).read_text(encoding='utf-8')
        self.review(expect_controls='\x1b' in source or '\u202e' in source)

    def review(self, expect_controls=False):
        files.until(self.session, lambda: any(value in self.session.display()
                    for value in ('OLD exact reference', 'Read the complete scope', )),
                    'exact old/new renewal review was not visible')
        self.capture('renewal-old-new-' + str(len(list(self.output.glob('renewal-old-new-*')))))
        # Sequential paging cannot skip unseen scope at the minimum terminal size.
        observed = []
        for _ in range(128):
            view = self.session.display()
            observed.append(''.join(view.splitlines()))
            if any(value in view for value in ('y renew exactly', )):
                break
            self.session.send(PGDN)
            self.session.pump(.02)
        files.until(self.session, lambda: any(value in self.session.display()
                    for value in ('y renew exactly', )),
                    'complete renewal scope did not expose explicit consent')
        if expect_controls:
            visible = ''.join(observed)
            assert '\\r' in visible and '\\u{202e}' in visible and '\\u{1b}' in visible, \
                'source control bytes were not explicitly escaped in the actual renewal review'

    def confirm(self, target):
        self.session.send('y')
        files.until(self.session, lambda: self.target() == target,
                    'explicit exact held renewal did not commit')
        assert len(self.held_rows()) == 1
        self.workspace()

    def memory_review(self):
        self.session.command('held-memory')
        self.session.command('held-set-memory')
        self.review()

    def approve_note(self, text):
        self.session.command('held-memory')
        self.session.command('create-memory')
        self.session.paste(text)
        self.session.send('\r')
        self.session.command('memory-scope')
        files.until(self.session, lambda: 'Project / all tasks' in self.session.display(),
                    'memory project scope did not appear')
        self.session.send(PGDN * 64 + RIGHT + '\r')
        files.until(self.session, lambda: len(self.memories()) == 1,
                    'explicit memory approval did not commit')

    def edit_note(self, suffix):
        before = self.memories()[0]['revision']
        self.session.command('held-memory')
        self.session.command('edit-memory')
        self.session.send(ESC + '[F')
        self.session.paste(suffix)
        self.session.send('\r')
        self.session.send(PGDN * 64 + RIGHT + '\r')
        files.until(self.session, lambda: self.memories()[0]['revision'] > before,
                    'explicit memory edit did not commit a new revision')
        self.workspace()

    def discard_failed_write(self):
        self.session.command('held')
        files.until(self.session, lambda: any(value in self.session.display()
                    for value in ('Save failed', )),
                    'source race did not produce truthful retained write failure')
        self.capture('changed-file-writer-refusal')
        self.session.command('records-discard')
        self.session.send(PGDN * 128 + RIGHT + '\r')
        self.workspace()

    def previous_reader_probe(self, previous):
        self.finish()
        database = self.data / 'workbench.sqlite3'
        before = database.read_bytes()
        original = (self.held_rows(), self.memories(), self.contexts(), self.outgoing_memories())
        posts = len(self.peer.snapshot('POST'))
        argv = [str(previous), *self.restart_argv[1:]]
        old = SurfaceSession(argv, self.project, self.restart_environment, self.restart_size,
                             self.output / 'previous-reader', )
        outcome = {'binary': str(previous),
                   'sha256': hashlib.sha256(previous.read_bytes()).hexdigest(),
                   'before_database_sha256': hashlib.sha256(before).hexdigest()}
        try:
            files.until(old, lambda: any(value in old.display()
                        for value in ('Storage: save failed', 'Storage: saved')),
                        'previous reader produced neither admission nor refusal', timeout=35)
            refused = 'Storage: save failed' in old.display()
            outcome['admission'] = 'refused' if refused else 'admitted'
            outcome['observed_surface'] = old.display()
            old.capture('previous-reader-observed-admission')
            if not refused:
                # Do not manufacture a rejection if this real reader admits the data.
                old.command('held')
                old.capture('previous-reader-held-reference')
                old.command('quit')
                confirm_exit(old)
                assert old.wait_exit() == 0
                outcome['shutdown'] = 'ordinary explicit quit'
            else:
                outcome['shutdown'] = 'owned refusal probe terminated; graceful old exit not asserted'
        finally:
            old.close()
        after = database.read_bytes()
        outcome['after_database_sha256'] = hashlib.sha256(after).hexdigest()
        outcome['database_bytes_unchanged'] = after == before
        if outcome['admission'] == 'refused':
            assert after == before, 'previous-reader refusal changed original database bytes'
        assert (
            (self.held_rows(), self.memories(), self.contexts(), self.outgoing_memories()) == original
        ), 'previous reader changed stale reference, actual memory or independent context'
        assert len(self.peer.snapshot('POST')) == posts, 'previous-reader inspection replayed a request'
        self.session = SurfaceSession([*self.restart_argv, '--continue'], self.project, self.restart_environment,
                                      self.restart_size, self.output / 'current-after-previous',)
        wait_for(self.session, 'Storage: saved', timeout=35)
        self.session.command('held')
        self.session.command('held-accept')
        assert (self.held_rows(), self.memories(), self.contexts(), self.outgoing_memories()) == original
        assert len(self.peer.snapshot('POST')) == posts, 'current restart replayed or stale acceptance dispatched'
        self.capture('current-resumes-stale-memory-after-previous-reader')
        self.workspace()
        return outcome


def run(binary, output, root, size, previous=None):
    output.mkdir(parents=True, exist_ok=False)
    root.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'size': list(size), 'cases': [],
              'binary_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
              'previous_reader': {'admission': 'not-run', 'reason':
                  'not reached before failure' if previous else '--previous-binary not supplied'}}
    journey = None
    with files.Peer('responses', 'sk-held-renewal-local-fixture', True, retain_body_bytes=True) as peer:
        try:
            journey = Journey(binary, output, root, size, size[0] == 60, 'compatible',
                              'responses', True, False, peer, )
            wait_for(journey.session, 'Storage: saved', timeout=35)
            journey.session.command('goal')
            journey.session.paste('PK08 deliberate renewal actual application acceptance')
            journey.session.send('\r')
            wait_for(journey.session, 'Storage: saved')
            source = journey.project / 'old-held.py'
            moved = journey.project / 'selected-replacement.py'
            source.write_bytes('HELD_RENEWAL_ORIGINAL = "한글"\r\n'.encode('utf-8'))
            subprocess.run(['git', '-c', 'core.hooksPath=', 'add', '--', source.name],
                           cwd=journey.project, check=True, capture_output=True)
            journey.refresh_repository('renewal-initial-source')
            journey.setup_route()
            journey.session.command('vaporeon')
            journey.workspace()
            journey.choose_file(source.name)
            old = journey.target()
            journey.session.command('held-accept')
            wait_for(journey.session, 'Review local capture /', timeout=35)
            journey.session.command('attach')
            files.until(journey.session, lambda: len(journey.contexts()) == 1,
                        'ordinary explicit file pin did not commit')
            journey.workspace()
            frozen = journey.send_context('Explain only the explicitly selected fixture source.',
                                          'PK08_FROZEN_SOURCE_RECEIPT')
            frozen_bytes = json.dumps(frozen, sort_keys=True, ensure_ascii=False)
            basket = journey.contexts()
            journey.write_request('Exact unsent 한글 draft remains after reference renewal')
            journey.session.send((ESC + '[D') * 3 + '\x13')
            wait_for(journey.session, 'Storage: saved')
            drafts = [task['partner_drafts'] for task in journey.snapshot()['tasks']]
            posts = len(peer.snapshot('POST'))
            source.rename(moved)  # Explicit external move; Eevee must not track it.
            # Controls belong to local replacement review, not the safe frozen
            # provider packet: native admission correctly refuses raw controls.
            moved.write_bytes(moved.read_bytes() + '# literal ESC:\x1b[31m bidi:\u202e\r\n'.encode('utf-8'))
            journey.refresh_repository('explicitly-moved-source')
            assert journey.target() == old
            journey.inspect_file(moved.name)
            assert journey.target() == old
            journey.session.send('\r')  # Neutral default keeps old.
            assert journey.target() == old and journey.contexts() == basket
            assert [task['partner_drafts'] for task in journey.snapshot()['tasks']] == drafts
            report['cases'].append('selected moved file: exact old/new preview, neutral cancel preserves old slot, basket and literal draft/caret')

            # Changed bytes after source preview are refused by the actual writer.
            journey.inspect_file(moved.name)
            moved.write_text('CHANGED_AFTER_REVIEW = True\n')
            journey.session.send('y')
            journey.discard_failed_write()
            assert journey.target() == old and journey.contexts() == basket
            journey.refresh_repository('changed-replacement-source')
            journey.inspect_file(moved.name)
            new_file = {'file': {'path': moved.name}}
            journey.confirm(new_file)
            assert journey.contexts() == basket
            assert json.dumps(peer.snapshot('POST')[0]['body'], sort_keys=True,
                              ensure_ascii=False) == frozen_bytes
            assert len(peer.snapshot('POST')) == posts
            report['cases'].append('file race refuses; fresh explicitly selected replacement renews one slot without pinning, dispatch or frozen-request rewrite')

            # Startup policy is immutable: restart makes an excluded replacement
            # unavailable and cannot restore the prior process-local approval.
            saved = journey.held_rows()
            journey.restart_with('--exclude-context', moved.name)
            assert journey.held_rows() == saved and journey.contexts() == basket
            journey.reject_file(moved.name)
            report['cases'].append('restart retains exact configuration; current exclusion policy prevents replacement review and loses old transient consent')

            journey.approve_note('PK08_APPROVED_MEMORY_ORIGINAL: explicit user note')
            journey.session.command('held-set-memory')
            journey.review()
            memory = journey.memories()[0]
            old_memory = {'memory': {'id': memory['id'], 'revision': memory['revision']}}
            journey.confirm(old_memory)
            assert journey.outgoing_memories() == []
            journey.edit_note(' PK08_CHANGED_REVISION')
            assert journey.target() == old_memory, 'memory edit removed or auto-renewed old identity'
            journey.session.command('held')
            journey.session.command('held-accept')
            assert journey.target() == old_memory and journey.outgoing_memories() == []
            assert journey.contexts() == basket and len(peer.snapshot('POST')) == posts
            journey.capture('changed-memory-old-exact-reference-unavailable')
            journey.workspace()
            journey.memory_review()
            journey.session.send(ESC)
            assert journey.target() == old_memory and journey.outgoing_memories() == []
            if previous is not None:
                report['previous_reader'] = journey.previous_reader_probe(previous)
            else:
                journey.restart_with()
            assert journey.target() == old_memory
            journey.memory_review()
            memory = journey.memories()[0]
            new_memory = {'memory': {'id': memory['id'], 'revision': memory['revision']}}
            journey.confirm(new_memory)
            assert journey.outgoing_memories() == [] and journey.contexts() == basket
            report['cases'].append('changed approved memory stays unavailable across cancel/restart until explicit exact-revision renewal; no automatic selection')

            # Configuration and owner transitions revoke the visible renewal.
            journey.edit_note(' PK08_REVIEW_TO_REVOKE')
            journey.memory_review()
            journey.session.command('held-clear')
            files.until(journey.session, lambda: journey.target() is None,
                        'explicit clear did not replace configuration independently')
            journey.session.send('y')
            assert journey.target() is None, 'stale renewal overwrote changed configuration'
            journey.session.command('held-memory')
            journey.session.command('held-set-memory')
            files.until(journey.session, lambda: journey.target() is not None,
                        'new initial exact preference did not commit')
            journey.workspace()
            journey.session.command('party')
            journey.session.send('a')
            files.until(journey.session, lambda: len(journey.snapshot()['partners']) == 2,
                        'actual second partner was not created')
            journey.workspace()
            journey.edit_note(' PK08_OWNER_RACE')
            journey.memory_review()
            before_owner = journey.held_rows()
            journey.session.command('lead 2')
            journey.session.send('y')
            assert journey.held_rows() == before_owner, 'old-owner review changed a slot after lead transition'
            journey.workspace()
            journey.session.command('lead 1')
            journey.session.command('vaporeon')
            journey.workspace()
            report['cases'].append('configuration clear and actual lead transition revoke old renewal without overwriting either owner')

            # Revocation uses the real memory deletion review, not a seeded row.
            journey.memory_review()
            journey.session.command('held-memory')  # Leaving renewal revokes consent.
            journey.session.command('delete-memory')
            journey.session.send(PGDN * 128 + RIGHT + '\r')
            files.until(journey.session, lambda: not journey.memories(),
                        'explicit memory revocation did not commit')
            assert journey.target() is None
            journey.workspace()
            journey.session.command('held')
            journey.session.send('y')
            assert journey.target() is None and journey.outgoing_memories() == []
            assert journey.contexts() == basket and len(peer.snapshot('POST')) == posts
            journey.restart_with()
            assert journey.target() is None and journey.contexts() == basket
            report['cases'].append('actual reviewed revocation cannot be resurrected by stale renewal or restart; independent source basket and frozen packet remain')
            journey.finish()
            peer.assert_clean()
            report['status'] = 'passed'
        except Exception as error:
            report.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            if journey:
                journey.close()
            files.save(output / 'results.json', report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--previous-binary', help='Optional genuine prior executable; observe admission/refusal without forcing either')
    parser.add_argument('--size', choices=('60x18', '120x36'), default='120x36')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    previous = Path(args.previous_binary).resolve() if args.previous_binary else None
    size = tuple(map(int, args.size.split('x')))
    with tempfile.TemporaryDirectory(prefix='ev-held-renewal-한글-') as temporary:
        result = run(binary, output, Path(temporary) / 'fixture', size, previous)
    print(json.dumps({'status': result['status'], 'output': str(output)}))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
