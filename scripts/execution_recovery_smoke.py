#!/usr/bin/env python3
"""Native real-TUI acceptance for failed/stale/partial/interrupted evidence."""
import argparse
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for
from execution_smoke import HOME, RIGHT, DELETE, TITLE, REVIEW, IDLE, NativeProcess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/execution-recovery-smoke')
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == 'nt' and not binary.exists():
        binary = binary.with_suffix('.exe')
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': [], 'host': platform.platform()}
    sessions, releases, processes = [], [], []

    def passed(name):
        result['cases'].append(name)
        print('passed:', name, flush=True)

    def cleanup():
        for release in releases:
            release.touch()
        for session in sessions:
            session.close()
        deadline = time.monotonic() + 8
        while any(process.alive() for process in processes) and time.monotonic() < deadline:
            time.sleep(0.05)
        for process in processes:
            process.close()

    try:
        with ExitStack() as stack:
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='ev-recovery-空 格-'))).resolve()
            stack.callback(cleanup)
            project, data = root / 'project 漢字', root / 'data'
            project.mkdir()
            source = project / 'input.txt'
            source.write_bytes(b'original\r\n')
            for argv in (('init', '--quiet'), ('add', '--', 'input.txt'),
                         ('-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.invalid', 'commit', '-qm', 'fixture')):
                subprocess.run(['git', '-c', 'core.autocrlf=false', '-c', 'core.hooksPath=', *argv], cwd=project,
                               check=True, capture_output=True, timeout=10)
            database = data / 'workbench.sqlite3'

            def rows():
                with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                    return [(json.loads(record), name, reserved) for record, name, reserved in
                            db.execute('SELECT record,output_name,reserved FROM executions ORDER BY rowid')]

            def task():
                with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                    return db.execute('SELECT command,goal,lead_partner,partner_drafts FROM tasks ORDER BY id LIMIT 1').fetchone()

            def until(session, predicate, message, timeout=35):
                deadline = time.monotonic() + timeout
                while not predicate():
                    assert session.alive(), message + ': application exited'
                    assert time.monotonic() < deadline, message + '\n' + session.display()
                    session.pump(0.05)

            def launch(name, size=(120, 36)):
                session = Session([str(binary), '--data-dir', str(data), '--spool-kib', '1', '--images', 'off', '--color', 'mono'],
                                  project, terminal_environment('mono'), size, output / name)
                sessions.append(session)
                wait_for(session, 'Storage: saved')
                return session

            def prepare(session, value):
                session.send('\x1b')
                session.command('jolteon')  # Jolteon owns command preparation.
                session.command('run')
                wait_for(session, TITLE)
                old = task()[0]
                session.command('command-edit')
                wait_for(session, 'Command / explicit JSON argv or shell')
                session.send(HOME)
                for offset in range(0, len(old), 32):
                    count = min(32, len(old) - offset)
                    session.send(DELETE * count)
                    wait_for(session, str(len(old) - offset - count) + ' / 4096 bytes')
                session.paste(value)
                session.send('\r')
                until(session, lambda: task()[0] == value, 'command save acknowledgement')
                wait_for(session, 'Approval pending')
                session.command('run')
                wait_for(session, REVIEW)
                session.capture('review-' + str(len(rows()) + 1))
                if not rows():
                    session.resize((39, 10))
                    session.send(RIGHT + '\r')
                    wait_for(session, 'No command can be approved here.')
                    assert not rows(), 'undersized permission review admitted execution'
                    session.capture('undersized-approval-refused')
                    session.resize((60, 18))
                session.send(RIGHT + '\r')

            def direct(code):
                return json.dumps([str(Path(sys.executable).resolve()), '-X', 'utf8', '-u', '-c', code])

            def finished(session, count, state='Succeeded'):
                until(session, lambda: len(rows()) == count and rows()[-1][0]['state'] == state and not rows()[-1][2],
                      'terminal journal acknowledgement: ' + state)
                wait_for(session, IDLE)
                record, name, _ = rows()[-1]
                assert record['tree'] == 'OwnedTreeExited'
                return record, data / name

            def quit_idle(session):
                session.command('quit')
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()

            def waiting(session, label, prelude=''):
                marker, release = root / (label + '.pid'), root / (label + '.release')
                releases.append(release)
                code = ('import os,time\nfrom pathlib import Path\n' + prelude +
                        '\nPath(' + repr(str(marker)) + ').write_text(str(os.getpid()))\n' +
                        'print("WAITING",flush=True)\nend=time.monotonic()+90\n' +
                        'while not Path(' + repr(str(release)) + ').exists() and time.monotonic()<end: time.sleep(.02)\n' +
                        'print("DONE",flush=True)')
                count = len(rows()) + 1
                prepare(session, direct(code))
                until(session, lambda: marker.exists() and len(rows()) == count and rows()[-1][0]['state'] == 'Running',
                      'running ownership acknowledgement')
                process = NativeProcess(int(marker.read_text()))
                processes.append(process)
                return count, marker, release, process

            session = launch('freshness-and-capture', (60, 18))
            session.command('goal')
            session.paste('recovery task')
            session.send('\r')
            until(session, lambda: task() is not None and task()[1] == 'recovery task', 'task creation')
            session.command('run')
            wait_for(session, TITLE)
            shell = {'shell': os.environ['COMSPEC'], 'dialect': 'cmd', 'text': 'echo shell verified & exit /b 7'} if os.name == 'nt' else {
                'shell': '/bin/sh', 'dialect': 'posix', 'text': "printf 'shell verified\\n'; exit 7"}
            prepare(session, json.dumps(shell))
            failed, spool = finished(session, 1, 'Failed')
            assert failed['exit_code'] == 7 and b'shell verified' in spool.read_bytes()
            assert failed['prepared']['spec']['Shell']['text'] == shell['text']
            session.command('history')
            wait_for(session, 'Failed command')
            passed('explicit-shell-dialect-and-nonzero-exit-remain-failed-evidence')

            count, _, release, _ = waiting(session, 'during')
            wait_for(session, 'WAITING')
            source.write_bytes(b'changed during command\r\n')
            release.touch()
            ambiguous, _ = finished(session, count)
            assert ambiguous['prepared']['before']['fingerprint'] != ambiguous['after']['fingerprint']
            session.command('history')
            wait_for(session, 'Ambiguous')
            session.capture('ambiguous-input-change')
            passed('during-run-input-change-is-ambiguous')
            prepare(session, direct('print("stable verification")'))
            stable, _ = finished(session, count + 1)
            assert stable['prepared']['before']['fingerprint'] == stable['after']['fingerprint']
            source.write_bytes(b'changed after command\r\n')
            session.command('history')
            session.command('refresh')
            wait_for(session, 'Stale')
            session.capture('stale-after-refresh')
            passed('post-run-input-change-is-stale-after-explicit-refresh')

            count, _, release, _ = waiting(session, 'overflow', 'print("\\x1b]0;untrusted-title\\x07VISIBLE",flush=True)\nprint("x"*(10*1024*1024),flush=True)\n')
            wait_for(session, 'CAPTURE TRUNCATED')
            session.capture('bounded-live-capture-warning')
            release.touch()
            partial, spool = finished(session, count)
            assert partial['truncated'] and partial['discarded_bytes'] > 9 * 1024 * 1024
            assert partial['captured_bytes'] <= 1024 and spool.stat().st_size <= 1024
            assert b'VISIBLE' in spool.read_bytes() and b'\x1b' not in spool.read_bytes()
            passed('ten-megabyte-output-drains-with-bounded-spool-sanitization-and-visible-truncation')

            count, _, release, process = waiting(session, 'locked-journal')
            wait_for(session, 'WAITING')
            with closing(sqlite3.connect(database, timeout=1)) as lock:
                lock.execute('BEGIN IMMEDIATE')
                session.resize((39, 10))
                session.command('cancel')
                wait_for(session, 'Cancel and wait')
                session.capture('below-minimum-neutral-owned-cancel')
                session.send(RIGHT + '\r')
                session.resize((120, 36))
                until(session, lambda: not process.alive(), 'owned cancellation while SQLite is locked')
                session.command('artifacts')
                wait_for(session, 'ERROR LAYER: Storage')
                assert rows()[-1][0]['state'] == 'Running' and rows()[-1][2]
                session.capture('failed-journal-save-retains-owned-result')
                lock.rollback()
            session.command('journal-retry')
            cancelled, _ = finished(session, count, 'Cancelled')
            assert cancelled['cancelled']
            passed('locked-journal-retains-cancelled-result-and-explicit-storage-retry-does-not-reexecute')
            quit_idle(session)

            interrupted = launch('interrupted-start')
            interrupted.command('run')
            count, marker, release, process = waiting(interrupted, 'interrupted')
            wait_for(interrupted, 'WAITING')
            original_pid = marker.read_bytes()
            interrupted.capture('committed-running-before-abrupt-close')
            interrupted.close()  # Abrupt app death is intentionally NOT a terminal-restoration claim.
            release.touch()  # Fixture cleanup, never evidence of Eevee cancellation on restart.
            deadline = time.monotonic() + 8
            while process.alive() and time.monotonic() < deadline:
                time.sleep(.05)
            assert not process.alive()
            resumed = launch('interrupted-resume', (60, 18))
            resumed.command('run')
            resumed.command('history')
            wait_for(resumed, 'Interrupted')
            recovered = rows()[-1][0]
            assert recovered['state'] == 'Interrupted' and recovered['tree'] != 'OwnedTreeExited'
            assert len(rows()) == count and marker.read_bytes() == original_pid and not rows()[-1][2]
            resumed.capture('interrupted-not-owned-or-replayed')
            quit_idle(resumed)
            passed('abrupt-app-death-resumes-as-interrupted-without-ownership-or-replay')
            (output / 'committed-evidence.json').write_text(json.dumps(rows(), indent=2) + '\n', encoding='utf-8')
        result['status'] = 'passed'
    except Exception as error:
        result['error'] = str(error)
        for session in sessions:
            session.capture('failure')
        raise
    finally:
        (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
