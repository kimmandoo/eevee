#!/usr/bin/env python3
"""Opt-in production SDK bridge + actual binary/compiler proof, without a model.

Requires an already-built Eevee, Node >=24 and installed sidecar dependencies.
Copies the sidecar into disposable storage and replaces ONLY query production
with sdk_command_peer.mjs. Production main.ts/policy.ts, real SDK MCP protocol,
normal UI file approvals and the shared durable host command runner remain real.
No real credential, network/model call, SQL writes or direct compiler invocation.
The optional --limit turns|budget mode produces an authoritative SDK exhaustion
result through the production bridge, without file/command effects. This proves
local protocol settlement, not live SDK engine turn counting or billing.
"""
from __future__ import annotations

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
from types import SimpleNamespace

import agent_coding_smoke as coding
import file_creation_smoke as files
from native_connection_smoke import until
from response_smoke import SurfaceSession, SURFACE, response_view, ALT_HOME, PAGE_DOWN
from terminal_smoke import terminal_environment, permission_run_available, inspect_permission_scope, confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--compiler', required=True, type=Path, help='genuine installed rustc')
    parser.add_argument('--node', type=Path, default=shutil.which('node'))
    parser.add_argument('--sidecar', type=Path, default=Path(__file__).resolve().parents[1] / 'sidecar')
    parser.add_argument('--limit', choices=('turns', 'budget'), help='controlled SDK exhaustion instead of compiler workflow')
    parser.add_argument('--automatic', action='store_true', help='prove fresh-default Automatic host effects without file/command approval keys')
    args = parser.parse_args(argv)
    if args.automatic and args.limit:
        parser.error('--automatic cannot be combined with --limit')
    binary, compiler, output, sidecar = (path.resolve() for path in (args.binary, args.compiler, args.output, args.sidecar))
    if args.node is None:
        parser.error('Node >=24 required; pass --node')
    node = args.node.resolve()
    for path in (binary, compiler, node, sidecar / 'node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs'):
        if not path.is_file():
            parser.error('required existing input missing: ' + str(path))
    if os.name != 'posix':
        parser.error('this opt-in driver requires POSIX PTY; no physical Windows certification')
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    result = {'status': 'failed', 'binary': str(binary), 'binary_sha256': digest,
              'compiler': str(compiler), 'node': str(node),
              'limits': ['Controlled query production, not live SDK/model/account conformance.',
                         'Fresh-default Automatic host effects.' if args.automatic else
                         'Actual compiler/program run only after separate exact host command approval.']}
    if args.limit:
        result.update(scenario='limit-' + args.limit)
        result['limits'] = ['Controlled SDK result production, not live engine >16-turn behavior or billing conformance.',
                            'Production bridge/app incomplete settlement; no compiler, command or file-write effect.']
    session = None
    with tempfile.TemporaryDirectory(prefix='eevee-sdk-command-') as temporary:
        root = Path(temporary).resolve()
        project, home, package = (root / name for name in ('project', 'home', 'sidecar'))
        project.mkdir()
        home.mkdir()
        data, log = root / 'data', root / 'peer.jsonl'
        log.touch()
        # Keep all imports and executable identities inside the reviewed package
        # tree. Never mutate the installed SDK or user sidecar.
        shutil.copytree(sidecar, package, ignore=shutil.ignore_patterns('.git', '.DS_Store'))
        sdk = package / 'node_modules/@anthropic-ai/claude-agent-sdk'
        shutil.copyfile(sdk / 'sdk.mjs', sdk / 'sdk-real.mjs')
        shutil.copyfile(Path(__file__).with_name('sdk_command_peer.mjs'), sdk / 'sdk.mjs')
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'EEVEE_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_', 'AWS_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                           XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                           ANTHROPIC_API_KEY='controlled-fixture-not-a-real-key', EEVEE_CLAUDE_NODE=str(node),
                           EEVEE_CLAUDE_SIDECAR=str(package / 'main.ts'), EV_SDK_COMPILER=str(compiler), EV_SDK_PROOF_LOG=str(log))
        if args.limit:
            environment['EV_SDK_LIMIT'] = args.limit

        def rows():
            return [json.loads(line) for line in log.read_text().splitlines() if line.strip()]

        def claims():
            with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                return list(db.execute('SELECT partner FROM active_work'))

        def workspace():
            for _ in range(16):
                footer = session.display().rstrip().splitlines()[-1].strip()
                if 'Alt+A' in footer or footer.startswith(tuple(SURFACE[key] for key in ('home', 'edit', 'draft'))):
                    return footer
                session.send(ESC)
            raise AssertionError('workspace unavailable\n' + session.display())

        try:
            session = SurfaceSession([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                                     project, environment, (120, 36), output, )
            wait_for(session, 'Storage: saved', timeout=40)
            if args.automatic:
                wait_for(session, 'AUTO-APPROVE')
            elif not args.limit:
                session.command('auto-approve-off')
                wait_for(session, 'ASK')
            session.command('provider-claude')
            workspace()
            journey = SimpleNamespace(session=session, project=project, data=data, surface=SURFACE, workspace=workspace)
            if workspace().startswith(SURFACE['home']):
                session.send('\r')
            session.paste('Report the controlled SDK limit outcome without running commands or writing files.' if args.limit else
                          'Create main.rs, build it, read and fix the actual compiler error, rebuild and run the program.')
            session.send('\x13')
            wait_for(session, 'Storage: saved')
            session.send('\r')
            approved = False

            def started():
                nonlocal approved
                if rows():
                    return True
                if permission_run_available(session) and not approved:
                    session.send('y')
                    approved = True
                elif not approved and inspect_permission_scope(session):
                    pass
                elif 'Enter to review (not approve)' in session.display():
                    session.send('\r')
                elif 'Alt+A' in session.display():
                    session.send(ESC + 'a')
                return False

            # First visible file review implies the reviewed SDK submission was
            # released; do not consume that independent file approval here.
            until(session, lambda: files.file_review_pending(session) or started(), 'SDK request/file review did not start', timeout=90)
            if args.limit:
                until(session, lambda: any(row['kind'] == 'finished' for row in rows()) and not claims(),
                      'SDK limit terminal/claim settlement', timeout=90)
                until(session, lambda: 'Incomplete' in session.display(), 'SDK exhaustion was not incomplete in the app')
                session.capture('sdk-limit-compact-state')
                session.command('verbose')
                session.send(ALT_HOME)
                transcript = {}
                for _ in range(64):
                    view = response_view(session)
                    first, last, total = view['range']
                    assert len(view['body']) == last - first + 1
                    transcript.update(zip(range(first, last + 1), view['body']))
                    before = session.display()
                    session.send(PAGE_DOWN)
                    if session.display() == before:
                        break
                else:
                    raise AssertionError('limit transcript inspection exceeded its bound')
                assert set(transcript) == set(range(1, total + 1)), 'retained limit transcript has missing rows'
                text = ''.join(''.join(transcript[index] for index in range(1, total + 1)).split())
                subtype = 'error_max_turns' if args.limit == 'turns' else 'error_max_budget_usd'
                assert subtype in text, text
                assert ('128-turnlimit' if args.limit == 'turns' else 'USD5querybudget') in text, text
                assert 'SDK-LIMIT:completedoutputretainedbeforeauthoritativeexhaustion.' in text, text
                proof = rows()
                assert len([row for row in proof if row['kind'] == 'query']) == 1, 'limit automatically resubmitted'
                assert len([row for row in proof if row['kind'] == 'finished']) == 1, proof
                assert not any(row['kind'] in ('step', 'command_result') for row in proof), proof
                assert not coding.runs(journey), 'controlled limit executed a command'
                assert not (project / 'main.rs').exists(), 'controlled limit wrote source'
                session.capture('sdk-limit-incomplete-retained-output-no-replay')
                session.command('verbose')
                result.update(status='passed', peer=proof, runs=[], outcome='incomplete',
                              logical_queries=1, terminal_reason=subtype, retained_transcript=text)
            else:
                for identity in ('write-broken', 'compile-failure', 'write-fix', 'compile-success', 'run-program'):
                    if not args.automatic:
                        if identity.startswith('write-'):
                            coding.file_decision(journey)
                        else:
                            coding.command_decision(journey)
                    until(session, lambda step=identity: any(row['kind'] == 'step' and row['id'] == step for row in rows()),
                          'real SDK tool result did not reach query: ' + identity, timeout=120)
                until(session, lambda: any(row['kind'] == 'finished' for row in rows()) and not claims(), 'SDK terminal/claim settlement', timeout=90)
                observed = [row for row in rows() if row['kind'] == 'command_result']
                assert [row['id'] for row in observed] == ['compile-failure', 'compile-success', 'run-program'], observed
                assert observed[0]['is_error'] and not observed[1]['is_error'] and not observed[2]['is_error']
                records = coding.runs(journey)
                assert len(records) == 3, records
                assert '42' in (project / 'main.rs').read_text()
                assert (project / ('sdk-program.exe' if os.name == 'nt' else 'sdk-program')).is_file()
                assert 'SDK-CODING:42' in observed[-1]['result']['stdout']
                result.update(status='passed', peer=rows(), runs=records)
            workspace()
            session.command('quit')
            confirm_exit(session)
            if args.limit:
                final_proof = rows()
                assert len([row for row in final_proof if row['kind'] == 'query']) == 1, 'limit resubmitted before exit'
                assert len([row for row in final_proof if row['kind'] == 'finished']) == 1, final_proof
                result['peer'] = final_proof
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc(), peer=rows())
            if session:
                session.capture('failed')
        finally:
            if session:
                session.close()
            assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
            files.save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'output': str(output)}))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
