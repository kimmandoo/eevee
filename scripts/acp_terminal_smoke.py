#!/usr/bin/env python3
"""Opt-in actual-binary ACP terminal journeys; no live model, credentials or SQL writes.

An isolated ACP-v1 peer only requests callbacks. Eevee alone writes/reads source,
launches real rustc, receives real failure/stderr, repairs, rebuilds and runs the
native program through the existing one-shot runner. No default CI matrix change.
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

from acp_file_smoke import save
from acp_terminal_peer_fixture import BROKEN, FIXED
from agent_coding_smoke import command_decision, file_decision
from native_connection_smoke import until
from response_smoke import SurfaceSession, SURFACE
from terminal_smoke import terminal_environment, confirm_exit, permission_run_available
from workbench_smoke import wait_for

ESC = '\x1b'
REQUEST = 'Create main.rs, read the actual source, build it, inspect the real compiler error, fix the code, rebuild and run the native program.'


class Journey:
    def __init__(self, session, project, data, ):
        self.session, self.project, self.data,  = session, project, data, 

    def workspace(self):
        for _ in range(16):
            footer = self.session.display().rstrip().splitlines()[-1].strip()
            if 'Alt+A' in footer or footer.startswith(tuple(SURFACE[key] for key in ('home', 'edit', 'draft'))):
                return footer
            self.session.send(ESC)
        raise AssertionError('Workspace unavailable\n' + self.session.display())

    def snapshot(self):
        database = self.data / 'workbench.sqlite3'
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            return dict(tasks=[dict(row) for row in db.execute('SELECT * FROM tasks')],
                        claims=[dict(row) for row in db.execute('SELECT * FROM active_work')],
                        runs=[json.loads(row[0]) for row in db.execute('SELECT record FROM executions ORDER BY rowid')],
                        journals=[json.loads(row[0]) for row in db.execute('SELECT record FROM provider_journal')])


def journey(binary, compiler, output, route, size, mode):
    result = dict(status='failed', route=route, size=list(size), mode=mode, cases=[])
    session = None
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix='eevee-acp-terminal-界-') as temporary:
        root = Path(temporary).resolve()
        project, home, runtime = (root / name for name in ('project', 'home', 'runtime'))
        for directory in (project, home, runtime):
            directory.mkdir()
        data = project / '.eevee-data'
        peer = runtime / route
        shutil.copyfile(Path(__file__).with_name('acp_terminal_peer_fixture.py'), peer)
        peer.chmod(0o700)
        log, control = root / 'peer.jsonl', root / 'owned-run-observed'
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_', 'DEVIN_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                           XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                           PATH=str(runtime) + os.pathsep + environment.get('PATH', ''), EV_PROVIDER=route,
                           EEVEE_ACP_TERMINAL_CHILD='1', EEVEE_ACP_TERMINAL_MODE=mode,
                           EEVEE_ACP_TERMINAL_LOG=str(log), EEVEE_ACP_TERMINAL_CONTROL=str(control),
                           EEVEE_ACP_TERMINAL_COMPILER=str(compiler))
        argv = [str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono']
        if size[0] == 60:
            argv.append('--ascii')

        def packets():
            return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

        try:
            session = SurfaceSession(argv, project, environment, size, output, )
            app = Journey(session, project, data, )
            wait_for(session, 'Storage: saved', timeout=40)
            if mode == 'readonly':
                app.workspace()
                session.command('role-plan')
            if app.workspace().startswith(SURFACE['home']):
                session.send('\r')
            session.paste(REQUEST)
            session.send('\x13')
            until(session, lambda: any(REQUEST == draft['text'] for task in app.snapshot()['tasks']
                  for state in json.loads(task['partner_drafts']).values() for draft in state.get('drafts', [])), 'request did not save')
            session.send('\r')
            neutral = ('Enter to review (not approve)', )
            until(session, lambda: any(label in session.display() for label in neutral),
                  'external-policy preview missing', timeout=45)
            session.send('y')
            assert not any(row['packet'].get('method') == 'session/prompt' for row in packets())
            session.send('\r')

            def preview():
                if permission_run_available(session):
                    return True
                session.send(ESC + '[6~')
                return False

            until(session, preview, 'exact external-policy review incomplete', timeout=45)
            session.capture('external-policy-before-dispatch')
            session.send('y')
            if mode != 'readonly':
                file_decision(app)
                until(session, lambda: (project / 'main.rs').exists(), 'first source effect missing')
                assert (project / 'main.rs').read_text() == BROKEN
                # Wait for the protocol's pre-approval concurrency and identity probes.
                until(session, lambda: any(row['packet'].get('method') == 'terminal/input' for row in packets()),
                      'async terminal handle did not process concurrent peer callbacks', timeout=45)
                assert not (project / 'agent-program').exists()
                assert not app.snapshot()['runs'], 'compiler launched before exact approval'
                command_decision(app)
                file_decision(app)
                until(session, lambda: (project / 'main.rs').read_text() == FIXED, 'actual fix was not applied')
                command_decision(app, decision=mode if mode in ('deny', 'cancel') else 'allow')
                if mode not in ('deny', 'cancel'):
                    until(session, lambda: (project / 'agent-program').is_file(), 'genuine compiler product missing', timeout=90)
                    command_decision(app)
                    if mode in ('kill', 'release'):
                        until(session, lambda: any(run['state'] == 'Running' and '--slow-marker' in run['prepared']['spec']['Direct']['args']
                              for run in app.snapshot()['runs']), 'slow owned command did not reach durable Running', timeout=60)
                        session.capture('actual-running-before-' + mode)
                        control.write_text('observed actual owned Running; no command or approval\n')
                result['cases'].extend(['normal durable writer lease and exact source consent',
                    'async create returned before command approval; output pending with no invented exit',
                    'concurrent peer output/read/invalid-ID/unsupported callbacks remained serviceable',
                    'overlapping command and filesystem write refused',
                    'genuine rustc failure returned actual stderr, source repaired with separate consent'])
            until(session, lambda: not app.snapshot()['claims'], 'owned ACP/runner durability did not settle', timeout=100)
            state = app.snapshot()
            runs = state['runs']
            assert not (project / 'must-not-exist').exists(), 'cancelled command produced its forbidden effect'
            expected = 0 if mode == 'readonly' else 1 if mode in ('deny', 'cancel') else 3
            assert len(runs) == expected, (mode, runs)
            if mode == 'readonly':
                assert not (project / 'main.rs').exists() and not (project / 'agent-program').exists()
                result['cases'].append('explicit Plan did not advertise terminal and actively refused malicious create/kill')
            else:
                assert runs[0]['exit_code'] not in (None, 0)
                if mode in ('deny', 'cancel'):
                    assert not (project / 'agent-program').exists(), 'denied/cancelled compiler had a side effect'
                    result['cases'].append('denied/cancelled pending exact command produced no executable and no execution row')
                else:
                    assert runs[1]['exit_code'] == 0
                    assert runs[2]['tree'] == 'OwnedTreeExited'
                    if mode in ('kill', 'release'):
                        assert runs[2]['cancelled'] and runs[2]['state'] == 'Cancelled', runs[2]
                        result['cases'].append('terminal/' + mode + ' stopped only the actual owned native command; no delayed marker effect')
                    else:
                        assert runs[2]['exit_code'] == 0
                        result['cases'].append('native compiler product executed through separate exact approval and actual output')
            initialize = [row['packet'] for row in packets() if row['packet'].get('method') == 'initialize']
            assert len(initialize) == 1 and initialize[0]['params']['clientCapabilities']['terminal'] is (mode != 'readonly')
            prompts = [row['packet'] for row in packets() if row['packet'].get('method') == 'session/prompt']
            assert len(prompts) == 1, 'prompt replayed'
            completions = [row['packet'] for row in packets() if row['direction'] == 'peer'
                           and row['packet'].get('id') == prompts[0]['id'] and 'result' in row['packet']]
            assert len(completions) == 1 and completions[0]['result']['stopReason'] in ('end_turn', 'cancelled'), \
                'peer assertions never completed; process exit/ownership release is not a successful tool journey'
            save(output / 'wire.json', packets())
            save(output / 'durable.json', state)
            session.capture('settled-actual-outcome')
            app.workspace()
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
    parser.add_argument('--binary', required=True, type=Path, help='already-built actual Eevee binary')
    parser.add_argument('--compiler', required=True, type=Path, help='genuine installed rustc, not rustup or a fixture')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--route', choices=('opencode', 'gemini', 'devin'), default='opencode')
    parser.add_argument('--size', choices=('120x36', '60x18'), default='120x36')
    parser.add_argument('--modes', nargs='+', choices=('happy', 'deny', 'cancel', 'kill', 'release', 'readonly'),
                        default=['happy', 'deny', 'cancel', 'kill', 'release', 'readonly'])
    args = parser.parse_args()
    if os.name != 'posix':
        parser.error('This opt-in executable-fixture PTY proof requires POSIX; Rust callback regressions are platform-independent')
    binary, compiler, output = args.binary.resolve(), args.compiler.resolve(), args.output.resolve()
    if not binary.is_file() or not compiler.is_file():
        parser.error('--binary and --compiler must already exist')
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = dict(status='failed', binary=str(binary), binary_sha256=digest, compiler=str(compiler), journeys=[],
                  limits=['Deterministic actual ACP-v1 peer is not live model/provider conformance.',
                          'PTY evidence is not physical terminal/WSL certification.',
                          'No private credentials, paid calls, SQL seeding, private executor or default CI expansion.'])
    size = tuple(map(int, args.size.split('x')))
    for mode in args.modes:
        report['journeys'].append(journey(binary, compiler, output / mode, args.route, size, mode))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'supplied binary changed'
    report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    save(output / 'results.json', report)
    print(json.dumps(dict(status=report['status'], output=str(output))))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
