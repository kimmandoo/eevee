#!/usr/bin/env python3
"""GP01/GP02 real offline CLI + disposable PTY restore acceptance.

Requires an already-built ev and existing terminal-smoke Python dependencies.
Creates actual task/draft/run/receipt/artifact through the application, uses only
synthetic loopback Responses data and an explicitly approved local Python run.
Never opens a live provider, imports credentials, or writes SQL. All data is disposable.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback

from file_creation_smoke import CreationJourney, response_plan
from native_connection_smoke import Peer, until
from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available, terminal_environment
from workbench_smoke import wait_for

ESC = '\x1b'
ACK = '--acknowledge-private-data'
DRAFT = 'UNSENT_SNAPSHOT_DRAFT 東京 preserved verbatim'


def environment(home, root):
    result = terminal_environment('mono')
    for key in list(result):
        if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GEMINI_',
                           'GOOGLE_', 'AWS_', 'OPENCODE_')) or key.lower().endswith('_proxy'):
            result.pop(key, None)
    result.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                  XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                  APPDATA=str(home / 'config'), LOCALAPPDATA=str(home / 'cache'),
                  CODEX_HOME=str(home / 'codex'), GEMINI_CLI_HOME=str(home / 'gemini'),
                  PATH=str(root / 'empty-bin'), NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost')
    return result


def hashes(directory):
    return {str(path.relative_to(directory)).replace('\\', '/'): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(directory.rglob('*')) if path.is_file()}


def records(data):
    with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
        return {table: list(db.execute('SELECT * FROM ' + table)) for table in
                ('tasks', 'executions', 'outgoing_receipts', 'tool_artifacts', 'active_work', 'provider_preferences')}


def cli(binary, args, root, env, output, label, success=True):
    result = subprocess.run([str(binary), 'snapshot', *map(str, args)], cwd=root, env=env,
                            capture_output=True, encoding='utf-8', timeout=90)
    (output / (label + '.json')).write_text(json.dumps({
        'argv': [str(binary), 'snapshot', *map(str, args)], 'status': result.returncode,
        'stdout': result.stdout, 'stderr': result.stderr}, indent=2) + '\n', encoding='utf-8')
    assert (result.returncode == 0) == success, (label, result.returncode, result.stdout, result.stderr)
    if success:
        assert 'Complete:' in result.stdout and 'snapshot format 1' in result.stdout
    else:
        assert 'Complete:' not in result.stdout
    return result


def stop(journey):
    journey.workspace()
    journey.session.command('quit')
    confirm_exit(journey.session)
    assert journey.session.wait_exit() == 0
    journey.session.close()


def run(binary, output, root):
    result = {'status': 'failed', 'cases': [], 'limits': 'Actual native CLI/PTY/local runner and synthetic loopback transport; no live provider claims'}
    journey = None
    with Peer('chat_completions', 'unused-public-snapshot-fixture', True, retain_body_bytes=True) as peer:
        try:
            journey = CreationJourney(binary, output, root, (120, 36), False, 'compatible',
                                      'chat_completions', True, False, peer, )
            session = journey.session
            wait_for(session, 'Storage: saved')
            (journey.project / 'snapshot-source.txt').write_text('SNAPSHOT_RETAINED_TOOL_TEXT\n', encoding='utf-8')
            journey.setup_route()
            before_posts = len(peer.snapshot('POST'))
            peer.plans.put(response_plan(peer, calls=[{'id': 'snapshot-read', 'name': 'eevee_read_file',
                'arguments': {'path': 'snapshot-source.txt', 'start_line': 1, 'end_line': 0}}]))
            peer.plans.put(response_plan(peer, text='SNAPSHOT_PROVIDER_DONE', number=1))
            journey.write_request('Read snapshot-source.txt and report its public retained text without changing files.')
            session.send('\r', settle=False)

            def submitted():
                if len(peer.snapshot('POST')) > before_posts:
                    return True
                if permission_run_available(session):
                    session.send('y', settle=False)
                elif inspect_permission_scope(session):
                    pass
                elif 'Alt+A' in session.display():
                    session.send(ESC + 'a')
                return False
            until(session, submitted, 'explicit loopback task submission did not dispatch', timeout=40)
            until(session, lambda: len(peer.snapshot('POST')) == before_posts + 2 and not journey.snapshot()['claims'],
                  'loopback tool/receipt did not settle', timeout=60)
            journey.workspace()
            argv = json.dumps([str(Path(sys.executable).resolve()), '-X', 'utf8', '-u', '-c',
                               'print("SNAPSHOT_LOCAL_RUN_OUTPUT")'])
            session.command('jolteon')
            session.send(ESC)
            session.command('command-edit')
            wait_for(session, 'JSON')
            session.paste(argv)
            session.send('\r')
            until(session, lambda: journey.snapshot()['tasks'][0]['command'] == argv, 'owned command draft not committed')
            session.command('execute')
            wait_for(session, 'Review one-shot execution permission')
            session.send(ESC + '[C' + '\r', settle=False)
            until(session, lambda: journey.snapshot()['executions'] == 1 and not journey.snapshot()['claims'],
                  'actual approved local run did not settle', timeout=60)
            journey.write_request(DRAFT)
            session.send('\x13')
            until(session, lambda: DRAFT in json.dumps(journey.snapshot()['tasks'], ensure_ascii=False), 'unsent draft not durable')
            journey.capture('source-task-and-unsent-draft')
            backup, restored = root / 'backup', root / 'restored'
            env = environment(journey.home, root)
            # Non-TTY root command must refuse live owner without starting another app.
            cli(binary, ['create', '--data-dir', journey.data, '--destination', backup, ACK], root, env, output, 'live-owner', False)
            assert not backup.exists()
            stop(journey)
            source_before = hashes(journey.data)
            rows_before = records(journey.data)
            assert len(rows_before['tasks']) == 1 and len(rows_before['executions']) == 1
            assert rows_before['outgoing_receipts'] and rows_before['tool_artifacts'] and not rows_before['active_work']
            cli(binary, ['create', '--data-dir', journey.data, '--destination', backup], root, env, output, 'privacy-required', False)
            assert not backup.exists()
            cli(binary, ['create', '--data-dir', journey.data, '--destination', backup, ACK], root, env, output, 'create')
            cli(binary, ['verify', '--source', backup], root, env, output, 'verify')
            manifest = json.loads((backup / 'snapshot-manifest.json').read_text())
            assert manifest['format'] == 1 and Path(manifest['origin']) == journey.data.resolve()
            for index, member in enumerate(manifest['members']):
                if not member['directory']:
                    data = (backup / f'member-{index:08}.data').read_bytes()
                    assert len(data) == member['bytes'] and hashlib.sha256(data).hexdigest() == member['sha256']
            assert hashes(journey.data) == source_before
            backup_before = hashes(backup)
            cli(binary, ['create', '--data-dir', journey.data, '--destination', backup, ACK], root, env, output, 'backup-collision', False)
            cli(binary, ['restore', '--source', backup, '--destination', journey.data, ACK], root, env, output, 'restore-collision', False)
            cli(binary, ['restore', '--source', backup, '--destination', restored, ACK], root, env, output, 'restore')
            restored_rows = records(restored)
            for table in ('tasks', 'outgoing_receipts', 'provider_preferences'):
                assert restored_rows[table] == rows_before[table], table
            assert not restored_rows['active_work']
            assert hashes(journey.data) == source_before and hashes(backup) == backup_before
            result['cases'].append('actual-task-draft-run-receipt-artifact-create-verify-fresh-restore-and-collisions')

            # Same project identity, new data folder: inspection must never replay the request/run.
            baseline_posts = len(peer.snapshot('POST'))
            journey.data = restored
            journey.session = SurfaceSession([str(binary), '--continue', '--data-dir', str(restored), '--images', 'off', '--color', 'mono'],
                journey.project, env, (120, 36), output / 'restored-tui', )
            session = journey.session
            wait_for(session, 'Storage: saved')
            until(session, lambda: DRAFT in session.display(), 'original unsent draft not visible on restored launch')
            journey.capture('restored-original-draft-zero-dispatch')
            session.command('receipts')
            wait_for(session, 'Outgoing receipts')
            journey.capture('restored-receipts')
            journey.workspace()
            session.command('runs')
            wait_for(session, 'Succeeded')
            session.send('\r')
            wait_for(session, 'Retained bytes')
            until(session, lambda: 'SNAPSHOT_LOCAL_RUN_OUTPUT' in session.display(), 'restored original run output unavailable')
            journey.capture('restored-run-output')
            journey.workspace()
            session.command('artifacts')
            wait_for(session, 'Tool artifacts')
            session.send('\r')
            until(session, lambda: 'SNAPSHOT_' in session.display(), 'restored retained artifact bytes unavailable')
            journey.capture('restored-artifact')
            assert len(peer.snapshot('POST')) == baseline_posts
            assert len(records(restored)['executions']) == 1 and not records(restored)['active_work']
            stop(journey)
            result['cases'].append('actual-restored-launch-original-draft-receipt-run-artifact-inspection-zero-dispatch')

            corrupt = root / 'corrupt-backup'
            shutil.copytree(backup, corrupt)
            index = next(i for i, member in enumerate(manifest['members']) if member['path'] == 'workbench.sqlite3')
            (corrupt / f'member-{index:08}.data').write_bytes(b'truncated')
            cli(binary, ['verify', '--source', corrupt], root, env, output, 'corruption-verify', False)
            rejected = root / 'rejected'
            cli(binary, ['restore', '--source', corrupt, '--destination', rejected, ACK], root, env, output, 'corruption-restore', False)
            assert not rejected.exists()
            interrupted_source = root / 'interrupted-source'
            shutil.copytree(root / 'data', interrupted_source)
            with (interrupted_source / 'zz-large-private-output').open('wb') as file:
                file.truncate((64 * 1024 * 1024) if os.name == 'nt' else (4 * 1024 * 1024 * 1024))
            interrupted = root / 'interrupted-backup'
            process = subprocess.Popen([str(binary), 'snapshot', 'create', '--data-dir', str(interrupted_source),
                '--destination', str(interrupted), ACK], cwd=root, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 10
                while not (interrupted / '.ev-snapshot-incomplete').exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.001)
                assert (interrupted / '.ev-snapshot-incomplete').exists(), 'copy completed/failed before interruption boundary'
                process.kill()
                process.wait(timeout=10)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
            cli(binary, ['verify', '--source', interrupted], root, env, output, 'interrupted-refusal', False)
            assert hashes(root / 'data') == source_before and hashes(backup) == backup_before
            result['cases'].append('actual-corrupt-and-killed-cli-backup-never-advertise-complete')
            (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            if journey:
                journey.session.close()
            (output / 'http-requests.json').write_text(json.dumps(peer.snapshot(), indent=2) + '\n', encoding='utf-8')
            (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    arguments = parser.parse_args()
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='ev-snapshot-smoke-') as temporary:
        result = run(arguments.binary.resolve(), output, Path(temporary))
    print(json.dumps(result, indent=2))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
