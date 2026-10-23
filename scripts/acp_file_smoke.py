#!/usr/bin/env python3
"""Actual binary ACP file/consent/display journeys using an isolated JSONL peer.

No paid provider, credentials or original installs are used. The peer supplies
protocol only; Eevee's normal durable lease and file executor perform effects.
This POSIX PTY driver is not live-provider or physical-terminal conformance.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import traceback

from native_connection_smoke import until
from file_creation_smoke import file_review_pending
from response_smoke import SurfaceSession, SURFACE, ALT_END
from terminal_smoke import terminal_environment, confirm_exit, permission_run_available
from workbench_smoke import wait_for

ESC = '\x1b'
ORIGINAL = b'original\r\nkeep dirty bytes\r\n'
REQUEST = 'Create nested/new.txt and edit existing.txt, preserving unrelated dirty working bytes.'


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def journey(binary, output, route, language, size, mode):
    result = dict(status='failed', route=route, language=language, size=list(size), mode=mode, cases=[])
    session = None
    with tempfile.TemporaryDirectory(prefix='eevee-acp-files-') as temporary:
        root = Path(temporary).resolve()
        project, home, runtime = (root / name for name in ('project', 'home', 'runtime'))
        for directory in (project, home, runtime):
            directory.mkdir()
        data = project / '.eevee-data'
        database = data / 'workbench.sqlite3'
        (project / 'existing.txt').write_bytes(ORIGINAL)
        peer = runtime / route
        shutil.copyfile(Path(__file__).with_name('acp_file_peer_fixture.py'), peer)
        peer.chmod(0o700)
        log = root / 'peer.jsonl'
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_', 'DEVIN_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                           XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                           PATH=str(runtime) + os.pathsep + environment.get('PATH', ''), EV_PROVIDER=route,
                           EEVEE_ACP_FILE_TEST_CHILD='1', EEVEE_ACP_FILE_MODE=mode, EEVEE_ACP_FILE_LOG=str(log),
                           EEVEE_ACP_GENERIC_PERMISSION='1')
        argv = [str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono']
        if size[0] == 60:
            argv.append('--ascii')

        def snapshot():
            with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                db.row_factory = sqlite3.Row
                tasks = [dict(row) for row in db.execute('SELECT * FROM tasks')]
                claims = [dict(row) for row in db.execute('SELECT * FROM active_work')]
                journals = [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_journal')]
            for task in tasks:
                task['partner_drafts'] = json.loads(task['partner_drafts'])
            return dict(tasks=tasks, claims=claims, journals=journals)

        def packets():
            return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

        def workspace():
            for _ in range(12):
                footer = session.display().rstrip().splitlines()[-1].strip()
                if 'Alt+A' in footer or footer.startswith(tuple(SURFACE[language][key] for key in ('home', 'edit', 'draft'))):
                    return footer
                session.send(ESC)
            raise AssertionError('Workspace unavailable\n' + session.display())

        try:
            session = SurfaceSession(argv, project, environment, size, output, language=language)
            wait_for(session, 'Storage: saved', timeout=40)
            if workspace().startswith(SURFACE[language]['home']):
                session.send('\r')
            session.paste(REQUEST)
            session.send('\x13')
            until(session, lambda: any(draft['text'] == REQUEST for task in snapshot()['tasks']
                  for state in task['partner_drafts'].values() for draft in state.get('drafts', [])), 'request not durably saved')
            session.send('\r')

            neutral = ('Enter to review (not approve)', 'Enter 검토 (승인 아님)')
            until(session, lambda: any(label in session.display() for label in neutral),
                  'required external-policy review did not open automatically', timeout=40)
            session.send('y')
            assert not any(row['packet'].get('method') == 'session/prompt' for row in packets()), 'neutral arrival approved from typing'
            session.send('\r')

            def preview():
                if permission_run_available(session):
                    return True
                session.send('\x1b[6~')
                return False

            until(session, preview, 'explicit external-policy request review missing', timeout=40)
            assert not any(row['packet'].get('method') == 'session/prompt' for row in packets()), 'ACP auto-dispatched before external-policy review'
            assert (project / 'existing.txt').read_bytes() == ORIGINAL and not (project / 'nested').exists()
            session.capture('external-policy-review-before-dispatch')
            session.send('y')
            for decision in ('n', 'a'):
                until(session, lambda: any(label in session.display() for label in neutral),
                      'generic tool permission did not open automatically', timeout=40)
                before = packets()
                session.send('y')
                assert packets() == before, 'typing approved a newly arriving generic permission'
                session.capture('generic-neutral-' + decision)
                session.send('\r')
                for _ in range(12):
                    session.send('\x1b[6~')
                session.capture('generic-reviewed-' + decision)
                session.send(decision)
            result['cases'].append('prepared external policy and generic deny/allow opened automatically; arrival typing granted nothing')
            until(session, lambda: file_review_pending(session), 'actual ACP file approval missing', timeout=40)
            state = snapshot()
            assert len(state['claims']) == 1 and state['claims'][0]['writing'], 'ACP bypassed normal durable writing lease'
            assert (project / 'existing.txt').read_bytes() == ORIGINAL and not (project / 'nested').exists(), 'callback wrote before exact consent'
            session.capture('automatic-owned-file-proposal')
            result['cases'].append('external-policy Run review and durable writer lease preceded actual read callbacks and exact file consent')

            # Defer onto the inline home transcript. /response deliberately
            # reopens the Planning drawer and its still-pending exact approval;
            # it is not a transcript-only navigation command during consent.
            # ACP supplies complete arguments, not synthetic generating edges.
            before_deferral = packets()
            session.send(ESC)
            until(session, lambda: not file_review_pending(session)
                  and not any(label in session.display() for label in ('a/y Allow once', 'a/y 한 번')),
                  'file proposal did not defer to the inline response')
            session.send(ALT_END)
            wanted = 'actual new file' if mode == 'happy' else 'must not report success'
            until(session, lambda: wanted in session.display()
                  and any(label in session.display() for label in ('[FILE]', '[파일]'))
                  and not file_review_pending(session)
                  and not any(label in session.display() for label in ('a/y Allow once', 'a/y 한 번')),
                  'received source body and typed identity absent from inline output')
            assert packets() == before_deferral, 'deferring/reading source answered the pending callback'
            assert (project / 'existing.txt').read_bytes() == ORIGINAL and not (project / 'nested').exists(), 'transcript navigation changed files before consent'
            view = session.display()
            assert any(label in view for label in ('[FILE]', '[파일]')), 'ACP file observation lacks typed identity'
            assert any(line.lstrip().startswith('| ') and wanted in line for line in view.splitlines()), 'ACP source body looks like prose'
            assert not any(line.lstrip().startswith('> ') and wanted in line for line in view.splitlines()), 'ACP fabricated a generating edge for complete arguments'
            session.capture('received-code-distinct-from-prose')
            workspace()
            session.command('approvals')
            path = 'nested/new.txt' if mode == 'happy' else 'existing.txt'
            until(session, lambda: path in session.display() and any(label in session.display() for label in ('Allow once', '한 번', '1회')), 'exact file review not reopened')
            session.capture('exact-one-shot-file-consent')
            session.send('a' if mode == 'happy' else 'n')
            if mode == 'happy':
                until(session, lambda: file_review_pending(session), 'existing-file edit review missing')
                assert (project / 'nested/new.txt').read_bytes() == b'actual new file\n'
                assert (project / 'existing.txt').read_bytes() == ORIGINAL, 'existing file edited before its separate consent'
                session.send('\r')
                until(session, lambda: 'existing.txt' in session.display() and any(label in session.display() for label in ('Allow once', '한 번', '1회')), 'replacement summary missing')
                session.capture('dirty-baseline-exact-replacement-review')
                session.send('a')
            until(session, lambda: not snapshot()['claims'], 'ACP ownership/journals did not settle', timeout=40)
            state = snapshot()
            mediated = [row for row in state['journals'] if 'mediated' in row['origin']]
            assert len(mediated) == (2 if mode == 'happy' else 1), 'mediated result journal missing or duplicated'
            assert any('session/update' in row['origin'] and 'External diff observation' in row['text'] for row in state['journals']), 'original correlated external tool journal lost'
            if mode == 'happy':
                assert (project / 'existing.txt').read_bytes() == b'edited\r\nkeep dirty bytes\r\n'
                assert all('source_sha256' in row['text'] for row in mediated), 'success lacks actual local receipt'
            else:
                assert (project / 'existing.txt').read_bytes() == ORIGINAL and not (project / 'nested').exists()
                assert all('source_sha256' not in row['text'] for row in mediated), 'denial manufactured a successful receipt'
            assert not (project / 'reported.rs').exists(), 'external diff display performed a local write'
            assert sum(row['packet'].get('method') == 'session/prompt' for row in packets()) == 1, 'turn retried or fabricated a continuation'
            session.capture('actual-file-outcome-and-owned-settlement')
            save(output / 'wire.json', packets())
            save(output / 'durable.json', state)
            result['cases'].append('actual current-folder read/create/dirty-preserving edit or denial settled with correlated receipts and no external-diff effect')
            workspace()
            session.command('quit')
            confirm_exit(session)
            assert session.wait_exit() == 0
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if session:
                session.capture('failed')
            save(output / 'wire.json', packets())
        finally:
            if session:
                session.close()
            save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    args = parser.parse_args()
    if os.name != 'posix':
        raise SystemExit('This driver uses POSIX executable fixtures; cross-platform callback tests run in cargo test.')
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = dict(status='failed', binary=str(binary), sha256=digest, journeys=[],
                  limits=['Deterministic ACP protocol peer, not paid/live-provider conformance.',
                          'Agent-owned tools remain external policy; callbacks are not their sandbox.',
                          'PTY evidence, not physical GUI-terminal certification.'])
    for route in ('opencode', 'gemini', 'devin'):
        for size in ((120, 36), (60, 18)):
            for mode in ('happy', 'deny'):
                label = f'{route}-{args.language}-{size[0]}-{mode}'
                report['journeys'].append(journey(binary, output / label, route, args.language, size, mode))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'supplied binary changed'
    report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    save(output / 'results.json', report)
    raise SystemExit(0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
