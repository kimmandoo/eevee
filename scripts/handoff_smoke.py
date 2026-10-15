#!/usr/bin/env python3
"""Actual local TUI JSON/Markdown handoff acceptance with all providers absent."""
import argparse
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

END = '\x1b[F'
RIGHT = '\x1b[C'
TITLE = 'Portable task handoff / local-only'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/handoff-smoke')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    if os.name == 'nt' and not binary.exists():
        binary = binary.with_suffix('.exe')
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
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='ev-handoff-空 格-'))).resolve()
            stack.callback(cleanup)
            project, data, empty = root / 'project', root / 'data', root / 'empty-path'
            project.mkdir()
            empty.mkdir()
            environment = terminal_environment('mono')
            environment['PATH'] = str(empty)
            session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                              project, environment, (120, 36), output / 'manual')
            sessions.append(session)
            wait_for(session, 'Storage: saved')
            session.send('\x1b')  # Leave the clean first-run Goal composer.
            session.command('goal')
            session.paste('Portable local goal 漢字; password=handoff-private-canary')
            session.send('\r')
            wait_for(session, 'Storage: saved')
            session.command('next')
            session.paste('Choose Markdown notes.\n\n[User-selected, unverified Codex planning; thread opaque-provider-id; '
                          'turn opaque-turn-id; item opaque-item-id. Not local execution evidence.]')
            session.send('\r')
            wait_for(session, 'Storage: saved')
            session.command('handoff')
            wait_for(session, TITLE)

            def review_export(command, destination):
                session.command(command)
                wait_for(session, 'LITERAL FILE PATH')
                session.paste(str(destination))
                session.send('\r')
                wait_for(session, 'SANITIZED EXPORT PREVIEW')
                session.send(END)
                wait_for(session, 'review complete')

            def accept():
                session.send(RIGHT + '\r')

            exported = root / 'approved.json'
            review_export('export-json', exported)
            session.send('\r')
            assert not exported.exists(), 'neutral Enter wrote an export'
            session.resize((39, 10))
            session.send(RIGHT + '\r')
            assert not exported.exists(), 'undersize review wrote an export'
            session.resize((60, 18))
            session.send(END)
            wait_for(session, 'review complete')
            session.capture('compact-export-review')
            accept()
            deadline = time.monotonic() + 10
            while not exported.exists():
                assert time.monotonic() < deadline, session.display()
                session.pump(0.1)
            document = json.loads(exported.read_text())
            assert document['schema_version'] == 1
            assert 'handoff-private-canary' not in exported.read_text()
            assert 'opaque-provider-id' not in exported.read_text()
            assert document['task']['next_action'] == 'Choose Markdown notes.'
            passed('provider-free JSON export preserves curated facts without secret/session provenance')

            markdown = root / 'approved.md'
            review_export('export-markdown', markdown)
            accept()
            deadline = time.monotonic() + 10
            while not markdown.exists():
                assert time.monotonic() < deadline, session.display()
                session.pump(0.1)
            assert 'opaque-provider-id' not in markdown.read_text()
            passed('Markdown manual export uses the same versioned portable document')

            def database_state():
                with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                    return (db.execute('SELECT goal,next_action,complete FROM tasks').fetchall(),
                            db.execute('SELECT count(*) FROM executions').fetchone()[0],
                            db.execute('SELECT count(*) FROM provider_journal').fetchone()[0])

            before = database_state()
            session.command('import-handoff')
            wait_for(session, 'LITERAL FILE PATH')
            session.paste(str(markdown))
            session.send('\r')
            wait_for(session, 'USER-SUPPLIED IMPORT REVIEW')
            session.send(END)
            wait_for(session, 'review complete')
            session.send('\r')
            assert database_state() == before, 'neutral import created task/evidence state'
            accept()
            session.capture('accepted-unverified-import')
            assert database_state() == before, 'accepted context became verified/completed task evidence'
            passed('manual import acceptance remains unverified context without execution or completion')

            newer = root / 'newer.json'
            document['schema_version'] = 99
            newer.write_text(json.dumps(document))
            session.command('import-handoff')
            wait_for(session, 'LITERAL FILE PATH')
            session.paste(str(newer))
            session.send('\r')
            session.pump(0.5)
            session.capture('newer-schema-refused')
            assert database_state() == before
            assert 'version' in session.display().lower() or 'schema' in session.display().lower(), session.display()
            session.send('\x1b')
            passed('unknown newer schema import is refused without task/evidence mutation')

            session.send('\x1b')
            session.command('quit')
            confirm_exit(session)
            assert session.wait_exit() == 0
            session.close()
        result['status'] = 'passed'
    finally:
        cleanup()
        (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
