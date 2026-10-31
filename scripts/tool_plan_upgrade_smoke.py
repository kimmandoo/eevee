#!/usr/bin/env python3
"""Actual prior/current binary continuity and reader refusal with a staged plan.

Admitted predecessor data may migrate to the current SQL schema. Report whether
refusal was schema-based or strict-state-based; never infer one from the other.
Both binaries and all app-owned state are genuine, not seeded.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import traceback

from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit, terminal_environment
from tool_plans_smoke import stage_plan, load_plan, until
from workbench_smoke import wait_for


def state(database):
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        return {
            'schema': connection.execute('PRAGMA user_version').fetchone()[0],
            'tasks': [dict(row) for row in connection.execute('SELECT * FROM tasks ORDER BY id')],
            'partners': [dict(row) for row in connection.execute('SELECT * FROM partners ORDER BY id')],
            'runs': [dict(row) for row in connection.execute('SELECT * FROM executions ORDER BY rowid')],
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--previous-binary', required=True)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    previous, current, output = Path(args.previous_binary).resolve(), Path(args.binary).resolve(), Path(args.output).resolve()
    assert not output.exists() or not any(output.iterdir()), 'Use a new evidence directory'
    output.mkdir(parents=True, exist_ok=True)
    report = {'status': 'failed', 'binaries': {name: hashlib.sha256(binary.read_bytes()).hexdigest() for name, binary in [('previous', previous), ('current', current)]}, 'cases': []}
    sessions = []
    try:
        with tempfile.TemporaryDirectory(prefix='ev-plan-upgrade-') as temporary:
            root = Path(temporary).resolve()
            project, home, data = root / 'project', root / 'home', root / 'data'
            project.mkdir(); home.mkdir()
            (project / 'public.txt').write_text('prior-version public source\n', encoding='utf-8')
            environment = terminal_environment('mono')
            for key in list(environment):
                if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_')) or key.lower().endswith('_proxy'):
                    environment.pop(key, None)
            environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'), XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'), EV_PROVIDER='compatible', EV_API_BASE_URL='http://127.0.0.1:9/v1', EV_API_MODEL='unused', EV_API_KEYLESS='1')
            database = data / 'workbench.sqlite3'
            def launch(binary, label):
                session = SurfaceSession([str(binary), *(['--continue'] if binary == current else []), '--data-dir', str(data), '--images', 'off', '--color', 'mono', '--no-animation'], project, environment, (120, 36), output / label, )
                sessions.append(session)
                return session
            def finish(session):
                session.send('\x03'); confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()
            old = launch(previous, 'previous-create')
            wait_for(old, 'Storage: saved', timeout=35)
            old.command('goal'); old.paste('Retain prior work across tool-plan upgrade 漢字'); old.send('\r')
            wait_for(old, 'Storage: saved', timeout=35)
            old.command('next'); old.paste('Unsent next action survives'); old.send('\r')
            wait_for(old, 'Storage: saved', timeout=35)
            old.capture('actual-prior-task')
            finish(old)
            before = state(database)
            assert before['schema'] == 19
            upgraded = launch(current, 'current-admit')
            wait_for(upgraded, 'Storage: saved', timeout=35)
            wait_for(upgraded, 'Retain prior work', timeout=35)
            admitted = state(database)
            for key in ('goal', 'criteria', 'next_action', 'command', 'drafts', 'partner_drafts', 'lead_partner'):
                assert admitted['tasks'][0][key] == before['tasks'][0][key], key
            assert admitted['partners'] == before['partners'] and admitted['runs'] == before['runs']
            report['cases'].append('genuine-prior-task-and-owner-admitted-with-authored-facts-unchanged')
            nodes = [{'id': 'read-public', 'dependencies': [], 'operation': {'type': 'Read', 'path': 'public.txt', 'start_line': 1, 'end_line': 0}, 'reason': 'Read public fixture only after explicit plan start'}]
            stage_plan(upgraded, data, 'Strict plan reader continuity', nodes)
            until(upgraded, lambda: load_plan(data) is not None, 'New strict plan field did not persist')
            assert load_plan(data)['attempts'] == 0
            assert state(database)['runs'] == before['runs'], 'Upgrade/staging executed work'
            upgraded.capture('new-plan-staged-without-dispatch')
            finish(upgraded)
            after = state(database)
            assert after['schema'] == admitted['schema']
            frozen = database.read_bytes()
            refused = launch(previous, 'previous-refusal')
            wait_for(refused, 'Storage: save failed', timeout=35)
            refused.capture('actual-older-reader-refused-strict-field')
            refused.close()  # Owned refusal probe, not a claimed graceful old exit.
            assert database.read_bytes() == frozen, 'Older reader modified new-plan data'
            assert state(database) == after
            report['cases'].append('actual-prior-reader-refused-current-plan-data-with-every-database-byte-unchanged')
            report['reader_refusal_basis'] = (
                'newer SQL schema' if after['schema'] > before['schema']
                else 'strict serialized state')
            resumed = launch(current, 'current-resume')
            wait_for(resumed, 'Storage: saved', timeout=35)
            resumed.command('tool-plan')
            wait_for(resumed, 'Strict plan reader continuity')
            assert load_plan(data)['attempts'] == 0 and state(database)['runs'] == before['runs']
            resumed.capture('restart-restored-inspection-without-replay')
            finish(resumed)
            report['cases'].append('current-reader-resumed-plan-inspection-with-zero-replay')
            report['schema'] = {'previous': before['schema'], 'current': after['schema']}
            report['older_writer_shutdown'] = 'owned refusal probe terminated; graceful older-runtime exit not asserted'
        report['status'] = 'passed'
    except Exception:
        report['error'] = traceback.format_exc()
        raise
    finally:
        for session in sessions:
            session.close()
        (output / 'results.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False))

if __name__ == '__main__':
    main()
