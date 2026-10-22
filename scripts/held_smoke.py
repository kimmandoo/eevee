#!/usr/bin/env python3
"""Actual PK04 PTY, filesystem, scoped store and loopback-packet acceptance.

No SQL writes, paid/live models, original user data or installed provider are used.
Both visible interactions and literal outgoing context membership are observed.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import traceback

import evolution_smoke
import file_creation_smoke as files
from native_connection_smoke import decoded_payload
from response_smoke import ALT_END, SurfaceSession
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
UP, RIGHT, HOME, DELETE = (ESC + code for code in ('[A', '[C', '[H', '[3~'))
PATH = '資料_한글.py'
FILE_MARKER = 'HELD_FILE_LITERAL_7f31'
MEMORY_MARKER = 'HELD_APPROVED_MEMORY_8e42'
CONTENT = '# ' + FILE_MARKER + '\r\nvalue = "한글界e\u0301"\r\n'
NOTE = MEMORY_MARKER + ' Preserve literal { $not_a_template } notes.'


class Journey(evolution_smoke.Journey):
    def workspace(self):
        if any(title in self.session.display() for title in ('Evolution!', '진화했어요!')):
            # Escape is forwarded to the retained editor and would open discard.
            self.session.send('\x1b[24~')  # Neutral F12 acknowledges without an editor action.
        return super().workspace()

    def held_rows(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM held_items ORDER BY project,partner,form')]

    def contexts(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            return [json.loads(row[0]) for row in db.execute('SELECT record FROM excerpts ORDER BY id')]

    def memories(self):
        with closing(sqlite3.connect((self.data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM memories ORDER BY id')]

    def outgoing_memories(self):
        return [row for task in self.snapshot()['tasks'] for row in json.loads(task['approved_memories'])]

    def slot(self, partner=1, form=1):
        return next((row for row in self.held_rows()
                     if row['partner'] == partner and row['form'] == form
                     and Path(row['project']).resolve() == self.project.resolve()), None)

    def reject_file(self, path):
        before = self.held_rows()
        self.session.command('held-file')
        wait_for(self.session, 'eligible files /', timeout=35)
        self.session.command('search')
        self.session.send(UP * 64 + HOME + DELETE * 4096)
        self.session.paste(path)
        self.session.send('\r')
        wait_for(self.session, '0 eligible files /', timeout=35)
        self.session.command('held-set-file')
        self.session.pump(.2)
        assert self.held_rows() == before, 'excluded/missing chooser target became a held preference'
        assert self.contexts() == [], 'refused held target created a source capture'
        self.capture('excluded-held-target-' + path.replace('/', '-'))
        self.workspace()

    def choose_file(self, path):
        self.session.command('held-file')
        files.until(self.session, lambda: any(value in self.session.display() for value in ('eligible files /', '후보 파일')),
                    'held file selector did not use actual source inventory', timeout=35)
        self.session.command('search')
        self.session.send(UP * 64 + HOME + DELETE * 4096)
        self.session.paste(path)
        self.session.send('\r')
        wait_for(self.session, '1 eligible files /', timeout=35)
        self.session.send('\r')
        wait_for(self.session, 'Review local capture /', timeout=35)
        self.session.command('held-set-file')
        files.until(self.session, lambda: self.slot() is not None,
                    'reviewed held file preference did not commit')
        self.workspace()

    def send_context(self, request, answer):
        gate = files.Gate()
        before = len(self.peer.snapshot('POST'))
        try:
            self.begin(request, [files.response_plan(self.peer, text=answer, gate=gate)], writing=False)
            files.until(self.session, gate.ready.is_set, 'actual held-context request did not reach its terminal gate')
            body = self.peer.snapshot('POST')[before]['body']
            self.capture('frozen-context-before-completion-' + str(before))
            gate.release.set()
            self.settled(answer, before + 1)
            return body
        finally:
            gate.release.set()

    def settled(self, answer, post_count):
        self.session.command('response')
        self.session.send(ALT_END)

        def complete():
            view = self.session.display()
            if any(title in view for title in ('wants to learn a move!', '기술을 배우려고 합니다!')):
                self.capture('optional-move-offer-before-reading-' + str(post_count))
                self.session.send(ESC)  # Deliberate Not now; never learn or grant anything.
                self.session.command('response')
                self.session.send(ALT_END)
                return False
            return (len(self.peer.snapshot('POST')) == post_count
                    and not self.snapshot()['claims'] and 'Storage: saved' in view and answer in view)

        files.until(self.session, complete, 'held request and owned terminal output did not settle', timeout=40)
        assert self.snapshot()['executions'] == 0, 'held context created local command evidence'
        self.capture('settled-' + str(post_count))
        self.workspace()

    def finish(self):
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()

    def restart_with(self, *extra):
        self.finish()
        self.restart_argv.extend(extra)
        self.session = SurfaceSession(self.restart_argv, self.project, self.restart_environment,
                                      self.restart_size, self.output / ('restart-' + str(getattr(self, '_restarts', 0))),
                                      language=self.language)
        self._restarts = getattr(self, '_restarts', 0) + 1
        wait_for(self.session, 'Storage: saved', timeout=35)


def run(binary, output, root, size, ascii_mode, language):
    output.mkdir(parents=True, exist_ok=False)
    root.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'language': language, 'size': list(size), 'cases': []}
    journey = None
    with files.Peer('responses', 'sk-held-fixture-only', True, retain_body_bytes=True) as peer:
        try:
            journey = Journey(binary, output, root, size, ascii_mode, 'compatible', 'responses',
                              True, False, peer, language=language)
            wait_for(journey.session, 'Storage: saved')
            # Current exclusions must govern choosing, accepting and later sends.
            journey.restart_with('--exclude-context', 'blocked')
            journey.session.command('goal')
            journey.session.paste('PK04 actual held context fixture')
            journey.session.send('\r')
            wait_for(journey.session, 'Storage: saved')
            source = journey.project / PATH
            source.write_bytes(CONTENT.encode())
            (journey.project / 'other.py').write_text('ordinary unrelated source\n')
            (journey.project / '.env.local').write_text('password=must-not-be-held\n')
            (journey.project / 'blocked').mkdir()
            (journey.project / 'blocked' / 'hidden.py').write_text('custom excluded source\n')
            subprocess.run(['git', '-c', 'core.hooksPath=', 'add', '--', PATH, 'other.py', 'blocked/hidden.py'],
                           cwd=journey.project, check=True, capture_output=True)
            subprocess.run(['git', '-c', 'core.hooksPath=', '-c', 'user.name=Local fixture',
                            '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'held fixture'],
                           cwd=journey.project, check=True, capture_output=True)
            journey.refresh_repository('initial-source-boundary')
            journey.setup_route()
            journey.session.command('vaporeon')
            journey.workspace()
            future = 'Unsent held draft 한글界e\u0301Z-end'
            journey.write_request(future)
            journey.session.send('\x1b[D' * 3 + '\x13')
            posts = len(peer.snapshot('POST'))
            for forbidden in ('.env.local', 'blocked/hidden.py'):
                journey.reject_file(forbidden)
            assert len(peer.snapshot('POST')) == posts, 'held selection or refusal dispatched inference'
            result['cases'].append('credential and current CLI custom-excluded files could not be held and created no context/network effect')
            journey.choose_file(PATH)
            assert journey.contexts() == [] and journey.outgoing_memories() == [], 'saving preference implicitly selected context'
            assert len(journey.held_rows()) == 1 and len(peer.snapshot('POST')) == posts
            drafts = [draft for task in journey.snapshot()['tasks']
                      for owner in task['partner_drafts'].values() for draft in owner['drafts']]
            assert any(draft['text'] == future and draft['cursor'] == len(future.encode()) - 3
                       for draft in drafts), 'holding/inspecting lost the literal Request draft or caret'
            files.until(journey.session, lambda: PATH in journey.session.display()
                        and '/held' in journey.session.display(),
                        'saved held identity was not visible in the ordinary minimum-size workspace')
            journey.capture('main-workspace-saved-held-reference')
            journey.session.command('held')
            wait_for(journey.session, PATH)
            journey.capture('saved-file-proposal-not-included')
            journey.workspace()
            journey.session.command('espeon')
            journey.session.command('held')
            assert PATH not in journey.session.display(), 'held slot leaked into a different form'
            journey.workspace()
            journey.session.command('vaporeon')
            journey.workspace()
            files.until(journey.session, lambda: PATH in journey.session.display()
                        and '/held' in journey.session.display(),
                        'form entry did not visibly propose the scoped held identity')
            journey.capture('main-workspace-form-entry-held-proposal')
            journey.session.command('held-dismiss')
            before = journey.held_rows()
            journey.session.send('\x1b[24~')
            journey.workspace()
            assert PATH not in journey.session.display() and '/held' not in journey.session.display(), \
                'dismissed proposal remained in the ordinary workspace or reappeared on redraw'
            assert journey.held_rows() == before and journey.contexts() == [], 'dismissal mutated saved preference or context'
            journey.capture('dismiss-keeps-slot-without-selection')
            body = journey.send_context('Explain the public fixture without selecting any proposed context.', 'HELD_PROPOSAL_OMITTED')
            assert FILE_MARKER not in json.dumps(body, ensure_ascii=False), 'unaccepted held file entered actual packet'
            result['cases'].append('saved scoped file and form-entry proposal never silently selected, sent or included; dismissal retained configuration')

            journey.session.command('espeon')
            journey.session.command('vaporeon')
            journey.session.command('held-accept')
            wait_for(journey.session, 'Review local capture /', timeout=35)
            assert journey.contexts() == [], 'accept inspection pinned before deliberate action'
            journey.capture('explicit-current-policy-file-review')
            journey.session.command('attach')
            files.until(journey.session, lambda: len(journey.contexts()) == 1, 'explicit held file capture did not commit')
            assert journey.contexts()[0]['text'].encode() == CONTENT.encode(), 'held capture changed literal bytes'
            journey.workspace()
            body = journey.send_context('Explain the explicitly selected source context.', 'HELD_FILE_SELECTED')
            policy, _ = decoded_payload('responses', body)
            assert FILE_MARKER in policy and 'held_items' not in json.dumps(body), 'held source missing or flavor metadata leaked'
            assert len(journey.contexts()) == 1 and len(journey.held_rows()) == 1
            journey.session.command('held')
            journey.capture('frozen-explicit-file-receipt')
            journey.workspace()
            result['cases'].append('explicit current-policy capture retained CRLF/Unicode, ordinary admission sent its bytes once and local receipt retained held attribution')

            source.write_bytes((CONTENT + '# changed after capture\n').encode())
            before_posts = len(peer.snapshot('POST'))
            journey.write_request('Explain the selected source after the external change.')
            journey.session.send('\r')
            files.until(journey.session, lambda: 'Request blocked:' in journey.session.display(),
                        'stale held source did not visibly refuse before send', timeout=35)
            assert len(peer.snapshot('POST')) == before_posts, 'stale held source transmitted before revalidation'
            journey.capture('changed-source-refused-before-network')
            journey.session.command('cancel')
            files.until(journey.session, lambda: not journey.snapshot()['claims'],
                        'explicit cancellation did not release the blocked immutable preview', timeout=35)
            assert len(peer.snapshot('POST')) == before_posts, 'blocked preview dispatched during cancellation'
            source.write_bytes(CONTENT.encode())
            journey.refresh_repository('restore-literal-source')
            result['cases'].append('external source change refused actual send without substituting new bytes')

            journey.session.command('held-memory')
            journey.session.command('create-memory')
            journey.session.paste(NOTE)
            journey.session.send('\r')
            journey.session.command('memory-scope')
            files.until(journey.session, lambda: 'Project / all tasks' in journey.session.display(),
                        'approved-memory project scope was not inspectable')
            journey.session.send('\x1b[6~' * 12 + RIGHT + '\r')
            files.until(journey.session, lambda: len(journey.memories()) == 1, 'explicit memory approval did not commit')
            assert journey.outgoing_memories() == [], 'memory approval or held preference selected outgoing memory'
            journey.session.command('held-set-memory')
            files.until(journey.session, lambda: journey.slot() is not None
                        and json.loads(journey.slot()['target']) == {
                            'memory': {'id': journey.memories()[0]['id'], 'revision': journey.memories()[0]['revision']}},
                        'held approved-memory preference did not replace the exact single file slot')
            assert len(journey.held_rows()) == 1 and journey.outgoing_memories() == []
            memory = journey.memories()[0]
            memory_label = f"#{memory['id']} @{memory['revision']}"
            journey.workspace()
            body = journey.send_context('Explain this fixture before memory acceptance.', 'HELD_MEMORY_OMITTED')
            assert MEMORY_MARKER not in json.dumps(body), 'unaccepted held memory entered actual packet'
            journey.session.command('held-accept')
            files.until(journey.session, lambda: len(journey.outgoing_memories()) == 1,
                        'explicit approved held-memory selection did not commit')
            journey.workspace()
            body = journey.send_context('Explain this fixture with explicitly selected memory.', 'HELD_MEMORY_SELECTED')
            assert MEMORY_MARKER in decoded_payload('responses', body)[0]
            assert 'held_items' not in json.dumps(body)
            journey.session.command('held')
            journey.capture('explicit-memory-and-frozen-local-receipt')
            journey.workspace()
            result['cases'].append('exact approved project-memory revision replaced one slot without selection, and entered actual packet only after explicit acceptance')

            saved = journey.held_rows()
            journey.restart_with()
            assert journey.held_rows() == saved, 'restart rewrote or replayed held preference'
            journey.session.command('held')
            wait_for(journey.session, memory_label)
            journey.capture('memory-held-after-real-restart')
            journey.workspace()
            journey.session.command('party')
            journey.session.send('a')
            files.until(journey.session, lambda: len(journey.snapshot()['partners']) == 2, 'second partner did not commit')
            journey.workspace()
            journey.session.command('lead 2')
            journey.session.command('vaporeon')
            journey.session.command('held')
            assert memory_label not in journey.session.display(), 'held memory identity leaked to another partner'
            assert journey.slot(partner=2) is None, 'partner change copied a scoped preference'
            journey.workspace()
            journey.session.command('lead 1')
            journey.session.command('vaporeon')
            journey.finish()
            other_project = root / 'isolated-project'
            other_project.mkdir()
            subprocess.run(['git', 'init', '-q', '--template='], cwd=other_project,
                           check=True, capture_output=True)
            journey.session = SurfaceSession(journey.restart_argv, other_project, journey.restart_environment,
                                              journey.restart_size, output / 'other-project', language=language)
            wait_for(journey.session, 'Storage: saved', timeout=35)
            journey.session.command('held')
            assert memory_label not in journey.session.display() and PATH not in journey.session.display(), \
                'held project preference leaked through shared personal data directory'
            journey.capture('project-isolated-with-shared-data')
            journey.finish()
            assert journey.held_rows() == saved, 'visiting another project rewrote original scoped preference'
            journey.session = SurfaceSession(journey.restart_argv, journey.project, journey.restart_environment,
                                              journey.restart_size, output / 'original-project-return', language=language)
            wait_for(journey.session, 'Storage: saved', timeout=35)
            journey.session.command('lead 1')
            journey.session.command('vaporeon')
            journey.session.command('held-clear')
            files.until(journey.session, lambda: journey.slot() is None, 'explicit held clear did not commit')
            assert len(journey.contexts()) == 1 and len(journey.outgoing_memories()) == 1, 'clearing configuration removed independently accepted context'
            journey.capture('clear-preserves-ordinary-selected-context')
            journey.finish()
            result['cases'].append('real restart retained exact preference; partner/form isolation held, and clearing configuration preserved independent context selections')
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            if journey:
                journey.close()
            files.save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='ev-held-空 格-') as temporary:
        root = Path(temporary)
        journeys = [run(binary, output / str(size[0]), root / str(size[0]), size, size[0] == 60, args.language)
                    for size in ((120, 36), (60, 18))]
    report = {'status': 'passed' if all(row['status'] == 'passed' for row in journeys) else 'failed',
              'binary': str(binary), 'language': args.language, 'journeys': journeys,
              'limits': 'Isolated PTY/SQLite read-only/loopback acceptance, not live-provider or physical-terminal conformance.'}
    files.save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
