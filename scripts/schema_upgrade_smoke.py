#!/usr/bin/env python3
"""Verify real previous/current binary upgrade and nondestructive schema downgrade.

Requires the same pyte/wcwidth dependencies as terminal_smoke.py. Both executables
must already be built for this native host; no simulated binary or SQLite schema
substitution is used. All task state and captures are disposable.
"""
import argparse
from contextlib import ExitStack, closing
import hashlib
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import tempfile

from package_binary import digest_file
from terminal_smoke import Session, terminal_environment
from workbench_smoke import wait_for


def snapshot(database):
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        version = connection.execute('PRAGMA user_version').fetchone()[0]
        tasks = [dict(row) for row in connection.execute(
            'SELECT id,project,goal,form,criteria,next_action,complete FROM tasks ORDER BY id')]
    return version, tasks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--previous-binary', required=True)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', default='target/schema-upgrade-smoke')
    args = parser.parse_args()
    previous, current = (Path(value).resolve() for value in (args.previous_binary, args.binary))
    if os.name == 'nt':
        previous = previous if previous.exists() else previous.with_suffix('.exe')
        current = current if current.exists() else current.with_suffix('.exe')
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'host': platform.platform(), 'cases': [], 'binaries': {}}
    for label, binary in (('previous', previous), ('current', current)):
        result['binaries'][label] = {
            'sha256': digest_file(binary),
            'version': subprocess.check_output([str(binary), '--version'], text=True, timeout=10).strip(),
        }
    assert result['binaries']['previous']['sha256'] != result['binaries']['current']['sha256'], \
        'Provide distinct actual previous and current binaries'
    sessions = []

    def close_sessions():
        for session in sessions:
            session.close()

    try:
        with ExitStack() as stack:
            temporary = stack.enter_context(tempfile.TemporaryDirectory(prefix='ev-schema-空 格-'))
            stack.callback(close_sessions)
            root = Path(temporary).resolve()
            project, data = root / 'project 漢字', root / 'data 空 格'
            project.mkdir()
            database = data / 'workbench.sqlite3'

            def launch(binary, name):
                session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                                  project, terminal_environment('mono'), (120, 36), output / name)
                sessions.append(session)
                return session

            def finish(session):
                session.send('\x1b')
                session.command('quit')
                if session.alive():
                    session.send('\x1b[C\r')
                assert session.wait_exit() == 0
                session.close()

            def previous_action(session, command):
                # Drive the explicitly provided historical executable's visible
                # palette: Ctrl+K opens or resets the chooser (query starts
                # fresh), typing filters by the command's unique slash name,
                # and the selected (>) row must name it before Enter.
                for _ in range(3):
                    session.send('\x0b')
                    session.pump(0.4)
                    session.send(command)
                    session.pump(0.4)
                    if any(('> /' + command) in line
                           for line in session.display().splitlines()):
                        session.send('\r')
                        session.pump(0.4)
                        return
                    session.send('\x1b')
                    session.pump(0.2)
                raise AssertionError('previous executable has no visible action: ' + command)
            old = launch(previous, 'previous-create')
            wait_for(old, 'Storage: saved')
            old.paste('schema continuity 漢字')
            old.send('\r')
            wait_for(old, 'Storage: saved')
            previous_action(old, 'criteria')
            # criteria-edit is a contextual choice inside the checklist overlay.
            previous_action(old, 'criteria-edit')
            old.paste('Preserve user goal\nRetain exact criteria')
            old.send('\r\x1b')
            previous_action(old, 'next')
            old.paste('Inspect unchanged evidence after upgrade')
            old.send('\r')
            wait_for(old, 'Storage: saved')
            old.capture('previous-task')
            old.send('\x03')  # Structural safe exit works on prior releases.
            wait_for(old, 'Confirm exit')
            old.send('\x1b[C\r')
            assert old.wait_exit() == 0
            old.close()
            old_schema, before = snapshot(database)
            assert len(before) == 1 and before[0]['goal'] == 'schema continuity 漢字'
            assert [item['text'] for item in json.loads(before[0]['criteria'])] == [
                'Preserve user goal', 'Retain exact criteria']
            assert before[0]['next_action'] == 'Inspect unchanged evidence after upgrade'

            upgraded = launch(current, 'current-upgrade')
            wait_for(upgraded, 'Storage: saved')
            wait_for(upgraded, 'schema continuity 漢字')
            upgraded.capture('migrated-task')
            finish(upgraded)
            new_schema, after = snapshot(database)
            assert new_schema > old_schema, f'No schema upgrade exercised: {old_schema} -> {new_schema}'
            assert after == before, 'Upgrade changed task facts, criteria, identity or next action'
            result['schemas'] = {'previous': old_schema, 'current': new_schema}
            result['cases'].append('actual-previous-binary-task-survives-current-migration')

            # Attempt the real older executable against the migrated database.
            # It must refuse, not rewrite the newer schema or its task records.
            before_downgrade = database.read_bytes()
            refused = launch(previous, 'previous-downgrade-refusal')
            wait_for(refused, 'Storage: save failed')
            refused.capture('newer-schema-refused')
            finish(refused)
            assert database.read_bytes() == before_downgrade, 'Downgrade modified newer database bytes'
            assert snapshot(database) == (new_schema, after)
            result['cases'].append('actual-older-binary-refuses-newer-schema-byte-for-byte')

            resumed = launch(current, 'current-resume')
            wait_for(resumed, 'Storage: saved')
            wait_for(resumed, 'schema continuity 漢字')
            resumed.capture('resume-after-refused-downgrade')
            finish(resumed)
            assert snapshot(database) == (new_schema, after)
            result['cases'].append('current-binary-resumes-after-refused-downgrade')
            result['database_sha256'] = hashlib.sha256(database.read_bytes()).hexdigest()
        result['status'] = 'passed'
    except BaseException as error:
        result['error'] = str(error)
        raise
    finally:
        for session in sessions:
            session.close()
        (output / 'results.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
