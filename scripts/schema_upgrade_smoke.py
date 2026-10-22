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
import sys
import time
import tempfile

from package_binary import digest_file
from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for


def snapshot(database):
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        version = connection.execute('PRAGMA user_version').fetchone()[0]
        tasks = [dict(row) for row in connection.execute('SELECT * FROM tasks ORDER BY id')]
        partners = [dict(row) for row in connection.execute('SELECT * FROM partners ORDER BY id')]
        evidence = {table: [dict(row) for row in connection.execute(f'SELECT * FROM {table} ORDER BY rowid')]
                    for table in ('excerpts', 'executions', 'provider_journal')}
    return {'schema': version, 'tasks': tasks, 'partners': partners, 'evidence': evidence}


def task_facts(state):
    # Navigation/save bookkeeping and the selected front may change through UI;
    # authored facts, scoped context and evidence must not.
    mutable = {'navigation', 'revision', 'updated_at', 'lead_partner', 'form',
               'drafts', 'partner_drafts'}
    return [{key: value for key, value in task.items() if key not in mutable}
            for task in state['tasks']]


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
    assert not output.exists() or not any(output.iterdir()), \
        'Choose a new or empty evidence directory; earlier failures must be retained'
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
            home = root / 'home'
            home.mkdir()
            environment = terminal_environment('mono')
            for key in list(environment):
                if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_')) \
                        or key.lower().endswith('_proxy'):
                    environment.pop(key, None)
            environment.update(HOME=str(home), USERPROFILE=str(home),
                               XDG_CONFIG_HOME=str(home / 'config'),
                               XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                               EV_PROVIDER='compatible', EV_API_BASE_URL='http://127.0.0.1:9/v1',
                               EV_API_MODEL='unused', EV_API_KEYLESS='1')

            def launch(binary, name):
                session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                                  project, environment, (120, 36), output / name)
                sessions.append(session)
                return session

            def finish(session):
                session.send('\x1b')
                session.command('quit')
                confirm_exit(session)
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
                    if any(('> /' + command + ' ') in (' '.join(line.split()) + ' ')
                           for line in session.display().splitlines()):
                        session.send('\r')
                        session.pump(0.4)
                        return
                    session.send('\x1b')
                    session.pump(0.2)
                raise AssertionError('previous executable has no visible action: ' + command)

            def until(session, predicate, reason, timeout=30):
                deadline = time.monotonic() + timeout
                while not predicate():
                    assert session.alive() and time.monotonic() < deadline, reason + '\n' + session.display()
                    session.pump(0.05)

            def select_lead(session, partner, historical=False):
                session.send('\x1b')
                if historical:
                    session.send('\x0b')
                    session.pump(0.4)
                    session.send(f'lead {partner}')
                    session.pump(0.4)
                    session.send('\r')
                else:
                    session.command(f'lead {partner}')
                until(session, lambda: snapshot(database)['tasks'][0]['lead_partner'] == partner
                      and 'Storage: saved' in session.display(), 'Lead selection did not commit')
                session.send('\x1b')

            def save_state(name, state):
                (output / (name + '.json')).write_text(
                    json.dumps(state, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
            old = launch(previous, 'previous-create')
            wait_for(old, 'Storage: saved')
            previous_action(old, 'goal')
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
            previous_action(old, 'party')
            for count in (2, 3):
                old.send('a')
                until(old, lambda: len(snapshot(database)['partners']) == count,
                      'Historical UI did not add partner')
                wait_for(old, f'Eevee #{count}')
            old.send('\x1b')
            select_lead(old, 2, historical=True)
            previous_action(old, 'jolteon')
            previous_action(old, 'nickname')
            old.paste('Bolt 漢字')
            old.send('\r')
            until(old, lambda: snapshot(database)['partners'][1]['nickname'] == 'Bolt 漢字',
                  'Historical specialist nickname did not commit')
            previous_action(old, 'command-edit')
            wait_for(old, 'Command / explicit JSON argv or shell')
            old.send('\x1b[A' * 100 + '\x1b[H' + '\x1b[3~' * 4096)
            command = json.dumps([str(Path(sys.executable).resolve()), '-B', '-c',
                                  'print("SCHEMA_RETAINED_LOCAL_EVIDENCE")'])
            old.paste(command)
            old.send('\r')
            until(old, lambda: snapshot(database)['tasks'][0]['command'] == command
                  and 'Storage: saved' in old.display(), 'Historical command did not commit')
            previous_action(old, 'execute')
            wait_for(old, 'Review one-shot execution permission')
            old.send('\x1b[C\r')
            until(old, lambda: len(snapshot(database)['evidence']['executions']) == 1
                  and not snapshot(database)['evidence']['executions'][0]['reserved'],
                  'Actual historical local execution did not settle')
            wait_for(old, 'SCHEMA_RETAINED_LOCAL_EVIDENCE')
            old.capture('previous-real-command-evidence')
            select_lead(old, 3, historical=True)
            previous_action(old, 'nickname')
            old.paste('Intake 漢字')
            old.send('\r')
            until(old, lambda: snapshot(database)['partners'][2]['nickname'] == 'Intake 漢字'
                  and 'Storage: saved' in old.display(), 'Historical Eevee nickname did not commit')
            select_lead(old, 1, historical=True)
            old.capture('previous-task')
            old.send('\x03')  # Structural safe exit works on prior releases.
            confirm_exit(old)
            assert old.wait_exit() == 0
            old.close()
            before = snapshot(database)
            old_schema = before['schema']
            save_state('previous-committed-state', before)
            assert len(before['tasks']) == 1 and before['tasks'][0]['goal'] == 'schema continuity 漢字'
            assert [item['text'] for item in json.loads(before['tasks'][0]['criteria'])] == [
                'Preserve user goal', 'Retain exact criteria']
            assert before['tasks'][0]['next_action'] == 'Inspect unchanged evidence after upgrade'
            assert [(row['id'], row['form']) for row in before['partners']] == [(1, 0), (2, 2), (3, 0)]
            assert before['partners'][1]['nickname'] == 'Bolt 漢字'
            assert before['partners'][2]['nickname'] == 'Intake 漢字'
            assert len(before['evidence']['executions']) == 1

            upgraded = launch(current, 'current-upgrade')
            wait_for(upgraded, 'Storage: saved')
            wait_for(upgraded, 'schema continuity 漢字')
            upgraded.capture('migrated-task')
            migrated = snapshot(database)
            save_state('current-migrated-state', migrated)
            new_schema = migrated['schema']
            assert new_schema > old_schema, f'No schema upgrade exercised: {old_schema} -> {new_schema}'
            assert task_facts(migrated) == task_facts(before), 'Migration changed authored task facts'
            assert migrated['tasks'][0]['lead_partner'] == before['tasks'][0]['lead_partner'] == 1
            assert migrated['tasks'][0]['form'] == before['tasks'][0]['form'] == 0
            for column in ('drafts', 'partner_drafts'):
                assert migrated['tasks'][0][column] == before['tasks'][0][column], column
            assert migrated['evidence'] == before['evidence'], 'Migration rewrote retained evidence'
            for prior, retained in zip(before['partners'], migrated['partners'], strict=True):
                assert {key: retained[key] for key in prior} == prior, \
                    'Migration changed a preexisting partner column'
            assert [(row['id'], row['form'], row['auto_form']) for row in migrated['partners']] == [
                (1, 0, 1), (2, 2, 0), (3, 0, 1)]
            result['schemas'] = {'previous': old_schema, 'current': new_schema}
            result['partner_migration'] = [
                {'id': row['id'], 'form': row['form'], 'auto_form': bool(row['auto_form']),
                 'retained_columns_unchanged': True} for row in migrated['partners']]
            result['cases'].extend([
                'actual-previous-binary-task-survives-current-migration',
                'old-eevee-partners-migrate-automatic-and-old-specialist-stays-manual',
                'all-preexisting-partner-columns-and-real-local-evidence-preserved'])
            for partner in (2, 3, 1):
                select_lead(upgraded, partner)
                state = snapshot(database)
                expected = next(row for row in migrated['partners'] if row['id'] == partner)
                assert state['tasks'][0]['form'] == expected['form']
                assert task_facts(state) == task_facts(before)
                assert state['partners'] == migrated['partners']
                assert state['evidence'] == before['evidence']
                upgraded.capture(f'migrated-lead-{partner}')
            finish(upgraded)
            after = snapshot(database)
            save_state('current-committed-state', after)
            result['cases'].append('migrated-partner-lead-switches-preserve-facts-modes-and-evidence')

            # Attempt the real older executable against the migrated database.
            # It must refuse, not rewrite the newer schema or its task records.
            before_downgrade = database.read_bytes()
            refused = launch(previous, 'previous-downgrade-refusal')
            wait_for(refused, 'Storage: save failed')
            refused.capture('newer-schema-refused')
            # This is an unsupported older writer, not the current runtime's
            # clean-exit journey. Some published writers retain failed startup
            # preferences and cannot settle a graceful quit against a newer
            # schema. End this owned refusal probe without pretending otherwise.
            refused.close()
            result['older_writer_shutdown'] = 'owned refusal probe terminated; graceful old-runtime exit not asserted'
            assert database.read_bytes() == before_downgrade, 'Downgrade modified newer database bytes'
            assert snapshot(database) == after
            result['cases'].append('actual-older-binary-refuses-newer-schema-byte-for-byte')

            resumed = launch(current, 'current-resume')
            wait_for(resumed, 'Storage: saved')
            wait_for(resumed, 'schema continuity 漢字')
            resumed.capture('resume-after-refused-downgrade')
            assert snapshot(database) == after
            for partner in (2, 3, 1):
                select_lead(resumed, partner)
                state = snapshot(database)
                assert task_facts(state) == task_facts(before)
                assert state['partners'] == migrated['partners']
                assert state['evidence'] == before['evidence']
                resumed.capture(f'resumed-lead-{partner}')
            finish(resumed)
            final = snapshot(database)
            assert task_facts(final) == task_facts(before)
            assert final['partners'] == migrated['partners']
            assert final['evidence'] == before['evidence']
            assert final['tasks'][0]['lead_partner'] == 1 and final['tasks'][0]['form'] == 0
            save_state('current-resumed-state', final)
            result['retained_evidence_counts'] = {
                table: len(rows) for table, rows in final['evidence'].items()}
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
