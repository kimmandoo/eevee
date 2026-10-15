#!/usr/bin/env python3
"""Opt-in real Codex controlled-file patch, provenance journal and local check acceptance."""
import argparse
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

RIGHT = '\x1b[C'
TITLE = 'Flareon / Codex implementation'
IDLE = 'Ready ·'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', action='store_true')
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/implementation-smoke')
    args = parser.parse_args()
    if not args.live:
        parser.error('--live required: real provider requests and disposable file edits')
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': []}
    sessions = []

    def passed(name):
        result['cases'].append(name)
        print('passed:', name, flush=True)

    def cleanup():
        for session in sessions:
            session.close()

    try:
        with ExitStack() as stack:
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='ev-patch-空 格-'))).resolve()
            stack.callback(cleanup)
            project, data = root / 'project', root / 'data'
            project.mkdir()
            source = project / 'calculator.py'
            baseline = 'def add(a, b):\n    return a - b\n'
            source.write_text(baseline)
            for argv in (['init', '--quiet'], ['add', '--', 'calculator.py'],
                         ['-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.invalid', 'commit', '-qm', 'fixture']):
                subprocess.run(['git', '-c', 'core.hooksPath=', *argv], cwd=project, check=True, capture_output=True)
            dirty = baseline + '\n# User pre-existing dirty work: preserve this exact line.\n'
            source.write_text(dirty)
            notes = project / 'user-notes.txt'
            notes.write_text('Untracked user work stays byte-exact.\n')
            protected = root / 'protected.py'
            protected.write_text(dirty)
            index_before = (project / '.git/index').read_bytes()

            def launch(name):
                session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                                  project, terminal_environment('mono'), (120, 36), output / name)
                sessions.append(session)
                wait_for(session, 'Storage: saved')
                return session

            def rows():
                with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                    return [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_journal ORDER BY id')]

            def until(session, predicate, message, seconds=20):
                deadline = time.monotonic() + seconds
                while not predicate():
                    assert session.alive(), message + ': app exited'
                    assert time.monotonic() < deadline, message + '\n' + session.display()
                    session.pump(0.1)

            def start_patch(session):
                session.command('new')
                wait_for(session, 'LOCAL PREVIEW', timeout=30)
                wait_for(session, 'y Run')
                session.capture('exact-edit-baseline-preview')
                session.send('y')
                wait_for(session, 'a Allow once', timeout=180)
                session.send('d')
                session.capture('exact-before-after-patch')
                session.send('d')

            def allow(session):
                session.send('a')
                wait_for(session, IDLE, timeout=180)

            def quit_app(session):
                session.send('\x1b')
                session.command('quit')
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()

            session = launch('controlled-patch')
            session.command('goal')
            session.paste('Fix add(a,b) in calculator.py so it returns a+b. Use ONLY eevee_edit_file with full replacement content. '
                          'Preserve the exact pre-existing user comment and all other files. Do not run commands or checks; '
                          'I will verify through the local runner afterward. Ask approval for the exact patch and stop if denied.')
            session.send('\r')
            wait_for(session, 'Storage: saved')
            session.command('role-implement')
            wait_for(session, TITLE)
            session.command('paths')
            wait_for(session, 'SELECT EDIT PATHS')
            session.send('\x1b[H' + '\x1b[3~' * 2)
            session.paste('["calculator.py"]')
            session.send('\r')
            wait_for(session, TITLE)
            start_patch(session)
            session.send('\r')
            wait_for(session, IDLE, timeout=180)
            assert source.read_text() == dirty and notes.read_text() == 'Untracked user work stays byte-exact.\n'
            assert (project / '.git/index').read_bytes() == index_before
            passed('real controlled patch denial preserves dirty tracked and untracked work')

            if os.name == 'posix':
                start_patch(session)
                source.unlink()
                source.symlink_to(protected)
                allow(session)
                assert protected.read_text() == dirty, 'substituted protected link was written'
                session.capture('protected-link-write-refused')
                source.unlink()
                source.write_text(dirty)
                passed('protected external write via substituted path denied at actual approval dispatch')

            start_patch(session)
            session.resize((60, 18))
            session.capture('compact-patch-review')
            allow(session)
            changed = source.read_text()
            assert 'return a + b' in changed or 'return a+b' in changed, changed
            assert '# User pre-existing dirty work: preserve this exact line.\n' in changed
            assert notes.read_text() == 'Untracked user work stays byte-exact.\n'
            assert protected.read_text() == dirty
            assert (project / '.git/index').read_bytes() == index_before
            until(session, lambda: any('Eevee' in row['coverage'] and row['complete'] for row in rows()), 'controlled tool journal committed')
            journal_before = rows()
            assert all(row['agent'] == 'Codex' and row['origin'] != 'local-runner' for row in journal_before)
            session.capture('applied-controlled-patch')
            passed('real selected-file patch preserves unrelated user edits and journals actual control coverage')

            session.resize((120, 36))
            session.send('\x1b')
            session.command('diff')
            wait_for(session, 'Repository', timeout=30)
            session.capture('working-diff-review')
            session.send('\x1b')
            session.command('run')
            session.command('command-edit')
            wait_for(session, 'Command / explicit JSON argv or shell')
            check = json.dumps([sys.executable, '-c', "import runpy; m=runpy.run_path('calculator.py'); assert m['add'](2,3)==5; print('EEVEE_LOCAL_CHECK_OK')"])
            session.paste(check)
            session.send('\r')
            wait_for(session, 'Approval pending', timeout=35)
            session.command('run')
            session.capture('deliberate-local-check-review')
            session.send(RIGHT + '\r')
            wait_for(session, 'EEVEE_LOCAL_CHECK_OK', timeout=35)
            session.capture('actual-local-verification-output')
            assert source.read_text() == changed
            session.send('\x1b')
            quit_app(session)
            passed('working patch reviewed and deliberately checked through separate local runner')

            resumed = launch('journal-restart')
            resumed.command('role-implement')
            wait_for(resumed, TITLE)
            resumed.command('provider-journal')
            wait_for(resumed, 'PROVIDER JOURNAL')
            resumed.capture('durable-provider-origin-journal')
            assert rows() == journal_before, 'restart changed or replayed provider journal'
            assert source.read_text() == changed
            resumed.command('provider-journal')
            quit_app(resumed)
            passed('provider-origin journal survives restart without model replay or local-evidence relabelling')
        result['status'] = 'passed'
    finally:
        cleanup()
        (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
