#!/usr/bin/env python3
"""Actual PTY/ConPTY current-folder source capture; no providers or live calls.

Requires an already-built native binary and the existing terminal_smoke Python
runtime. Covers nested Git, ordinary folders, and Unix canonical/control paths.
"""
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
import traceback

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for


def rows(database):
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=1)) as db:
        tasks = db.execute('SELECT id,project,goal FROM tasks ORDER BY id').fetchall()
        captures = [json.loads(row[0]) for row in db.execute('SELECT record FROM excerpts ORDER BY id')]
    return tasks, captures


def await_rows(session, database, predicate):
    deadline = time.monotonic() + 10
    while True:
        current = rows(database)
        if predicate(current):
            return current
        assert session.alive(), 'application exited before durable source capture'
        assert time.monotonic() < deadline, session.display()
        session.pump(0.05)


def exercise(binary, output, fixture, kind):
    enclosing = fixture / 'enclosing'
    enclosing.mkdir()
    name = 'project [literal] 漢字'
    if kind == 'canonical-controls':
        name += '\tactual\nidentity'
    project = enclosing / name
    project.mkdir()
    (project / 'src').mkdir()
    (project / 'src/current.rs').write_bytes(b'PROJECT_LOCAL_SOURCE\n')
    (project / 'excluded').mkdir()
    (project / 'excluded/hidden.rs').write_bytes(b'CONFIGURED_EXCLUSION\n')
    (project / '.env').write_bytes(b'password=credential-fixture\n')
    sibling = enclosing / 'project l 漢字'
    sibling.mkdir()
    (sibling / 'outside.rs').write_bytes(b'SIBLING_MUST_NOT_ENTER_CONTEXT\n')
    if kind == 'nested-git':
        for args in [('init', '--quiet'), ('add', '--all'),
                     ('-c', 'user.name=Scope fixture', '-c', 'user.email=scope@example.invalid',
                      '-c', 'core.hooksPath=', 'commit', '--quiet', '-m', 'fixture')]:
            subprocess.run(['git', *args], cwd=enclosing, check=True, capture_output=True, timeout=10)
        (sibling / 'outside.rs').write_bytes(b'SIBLING_DIRTY_MUST_NOT_ENTER_DIFF\n')
    launch = project
    if kind == 'canonical-controls':
        launch = fixture / 'canonical-alias'
        launch.symlink_to(project, target_is_directory=True)
    home = fixture / 'home'
    home.mkdir()
    data = project / 'local-data'
    data.mkdir()
    private_name = 'ev-run-scope-retained.output'
    (data / private_name).write_bytes(b'PRIVATE_STORAGE_OUTPUT_MUST_NOT_ENTER_CONTEXT\n')
    database = data / 'workbench.sqlite3'
    environment = terminal_environment('mono')
    for key in list(environment):
        if key.startswith(('EV_', 'OPENAI_', 'ANTHROPIC_', 'GOOGLE_', 'GEMINI_', 'CLAUDE_', 'CODEX_')):
            environment.pop(key)
    environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                       XDG_DATA_HOME=str(home / 'data'), )
    session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono',
                       '--exclude-context', 'EXCLUDED'], launch, environment, (120, 36), output / kind)
    try:
        wait_for(session, 'Storage: saved')
        session.command('goal')
        session.paste('Current folder source ownership ' + kind)
        session.send('\r')
        await_rows(session, database, lambda value: any(row[2].endswith(kind) for row in value[0]))
        session.command('files')
        wait_for(session, 'Sources / task-owned context')
        wait_for(session, '1 eligible files /', timeout=35)
        assert 'current.rs' in session.display(), session.display()
        assert 'outside.rs' not in session.display() and 'hidden.rs' not in session.display()
        assert private_name not in session.display(), session.display()
        session.send('s')
        wait_for(session, 'Filename filter')
        session.paste(private_name)
        session.send('\r')
        wait_for(session, '0 eligible files /', timeout=35)
        session.send('\r')
        wait_for(session, 'No eligible source selected.')
        assert rows(database)[1] == [], 'private output became a durable context capture'
        assert 'PRIVATE_STORAGE_OUTPUT_MUST_NOT_ENTER_CONTEXT' not in session.display()
        session.send('s')
        wait_for(session, 'Filename filter')
        session.send('\x1b[H' + '\x1b[3~' * len(private_name))
        session.send('\r')
        wait_for(session, '1 eligible files /', timeout=35)
        session.send('\r')
        wait_for(session, 'Review local capture /')
        wait_for(session, 'PROJECT_LOCAL_SOURCE')
        session.send('\r')
        tasks, captures = await_rows(session, database, lambda value: len(value[1]) == 1)
        expected = str(project.resolve())
        assert all(row[1] == expected for row in tasks), tasks
        capture = captures[0]
        assert capture['root'] == expected, capture
        assert Path(capture['path']) == Path('src/current.rs'), capture
        assert capture['text'] == 'PROJECT_LOCAL_SOURCE\n', capture
        session.command('close')
        session.command('repo')
        wait_for(session, 'PROJECT ROOT', timeout=35)
        if kind != 'nested-git':
            wait_for(session, 'unavailable')
        assert 'SIBLING_' not in session.display(), session.display()
        session.command('close')
        session.command('init')
        wait_for(session, '/init prepared an editable request')
        session.capture('init-readonly-proposal')
        assert not (project / 'AGENTS.md').exists()
        session.command('quit')
        confirm_exit(session)
        assert session.wait_exit() == 0
        assert (project / 'src/current.rs').read_bytes() == b'PROJECT_LOCAL_SOURCE\n'
        assert not (project / 'AGENTS.md').exists()
        assert (data / private_name).read_bytes() == b'PRIVATE_STORAGE_OUTPUT_MUST_NOT_ENTER_CONTEXT\n'
        return {'case': kind, 'project_identity': expected, 'captured_path': capture['path'],
                'status': 'passed', 'scope': 'actual PTY source browse/read/attach and private retained-output context refusal, project ownership, init proposal; no provider dispatch'}
    finally:
        session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': []}
    try:
        with tempfile.TemporaryDirectory(prefix='ev-project-scope-') as temporary:
            root = Path(temporary).resolve()
            cases = ['nested-git', 'filesystem']
            if os.name != 'nt':
                cases.append('canonical-controls')
            for kind in cases:
                fixture = root / kind
                fixture.mkdir()
                result['cases'].append(exercise(binary, output, fixture, kind))
        result['status'] = 'passed'
    except Exception:
        result['error'] = traceback.format_exc()
        raise
    finally:
        (output / 'results.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
