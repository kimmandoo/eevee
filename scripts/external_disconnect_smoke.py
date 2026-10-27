#!/usr/bin/env python3
"""Actual-binary bounded external stdio disconnect journeys; no live model calls.

Reuses the established correlated Codex/ACP and complete SDKbridge11 consumers.
Each peer exits before initialization or after a real prompt/partial response;
local diagnostic remediation must not start another process or resend anything.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import sys
import tempfile
import time
import traceback

from journey_conformance_smoke import Journey, SOURCE, STREAMING, protocol_peer, save
from native_connection_smoke import until
from response_smoke import SurfaceSession, saved_request
from terminal_smoke import terminal_environment
from workbench_smoke import wait_for

ESC = '\x1b'
ROUTES = ('codex', 'claude', 'opencode', 'gemini', 'devin')
FUTURE = 'Future 한글界e\u0301Z-end'
PRIVATE_CANARY = 'DISCONNECT_PRIVATE_CANARY'
SDK_WRAPPER = r'''
import { query as original, createSdkMcpServer, tool } from './journey-base.mjs';
export { createSdkMcpServer, tool };
import { appendFileSync, existsSync } from 'node:fs';
function record(kind) { appendFileSync(process.env.EEVEE_JOURNEY_LOG, JSON.stringify({kind, pid:process.pid}) + '\n'); }
async function gate() {
  const deadline = Date.now() + 60000;
  while (!existsSync(process.env.EEVEE_JOURNEY_RELEASE)) {
    if (Date.now() > deadline) throw new Error('controlled disconnect gate expired');
    await new Promise(resolve => setTimeout(resolve, 20));
  }
}
async function disconnect(code) {
  await new Promise(resolve => process.stderr.write('DISCONNECT_PRIVATE_CANARY', resolve));
  await new Promise(resolve => process.stdout.write('{"private":"DISCONNECT_PRIVATE_CANARY', resolve));
  process.exit(code);
}
record('peer_launch');
if (process.env.EEVEE_DISCONNECT_STAGE === 'before' && process.argv.includes('--inspect')) {
  record('disconnect_ready');
  await gate();
  await disconnect(71);
}
export function query(input) {
  const current = original(input);
  return { ...current, async *[Symbol.asyncIterator]() {
    for await (const message of current) {
      yield message;
      if (message.type === 'assistant' && message.message?.id === 'streaming') {
        record('disconnect_ready');
        await gate();
        await disconnect(72);
      }
    }
  }};
}
'''


def peer(route):
    log = Path(os.environ['EEVEE_JOURNEY_LOG'])
    def record(kind):
        with log.open('a') as stream:
            stream.write(json.dumps(dict(kind=kind, pid=os.getpid())) + '\n')
    def disconnect():
        sys.stderr.write(PRIVATE_CANARY)
        sys.stderr.flush()
        sys.stdout.write('{"private":"' + PRIVATE_CANARY)
        sys.stdout.flush()
    record('peer_launch')
    if '--version' in sys.argv or 'version' in sys.argv:
        protocol_peer(route)
        return
    original = sys.stdin
    def packets():
        for line in original:
            packet = json.loads(line)
            if os.environ['EEVEE_DISCONNECT_STAGE'] == 'before' and packet.get('method') == 'initialize':
                record('disconnect_ready')
                release = Path(os.environ['EEVEE_JOURNEY_RELEASE'])
                deadline = time.monotonic() + 60
                while not release.exists():
                    assert time.monotonic() < deadline, 'controlled disconnect gate expired'
                    time.sleep(.02)
                disconnect()
                return
            yield line
            if packet.get('method') in ('turn/start', 'session/prompt'):
                # The established peer has emitted a correlated acknowledgment
                # and partial text, then consumed the external stream gate.
                # Closing here cannot fabricate an authoritative terminal frame.
                disconnect()
                return
    sys.stdin = packets()
    protocol_peer(route)


def row(binary, node, sidecar, output, route, stage, size):
    output.mkdir(parents=True)
    result = dict(status='failed', route=route, stage=stage, size=list(size))
    session = None
    with tempfile.TemporaryDirectory(prefix='eevee-disconnect-한글-') as temporary:
        root = Path(temporary).resolve()
        project, home, runtime, data = (root / name for name in ('project', 'home', 'runtime', 'data'))
        for directory in (project, home, runtime):
            directory.mkdir()
        (project / 'source.txt').write_text(SOURCE)
        log, release = root / 'peer.jsonl', root / 'release'
        log.touch()
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'EEVEE_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_', 'DEVIN_', 'AWS_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
            XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'), CODEX_HOME=str(home / 'codex'),
            PATH=str(runtime) + os.pathsep + environment.get('PATH', ''), EV_PROVIDER=route,
            EEVEE_JOURNEY_MODEL='journey-fixed', EEVEE_JOURNEY_LOG=str(log),
            EEVEE_JOURNEY_RELEASE=str(release), EEVEE_DISCONNECT_STAGE=stage)
        if route == 'claude':
            package = root / 'sidecar'
            shutil.copytree(sidecar, package, ignore=shutil.ignore_patterns('.git', '.DS_Store'))
            sdk = package / 'node_modules/@anthropic-ai/claude-agent-sdk'
            shutil.copyfile(sdk / 'sdk.mjs', sdk / 'sdk-real.mjs')
            shutil.copyfile(Path(__file__).with_name('journey_sdk_peer.mjs'), sdk / 'journey-base.mjs')
            (sdk / 'sdk.mjs').write_text(SDK_WRAPPER)
            environment.update(ANTHROPIC_API_KEY='controlled-not-real', EEVEE_CLAUDE_NODE=str(node), EEVEE_CLAUDE_SIDECAR=str(package / 'main.ts'))
        else:
            executable = runtime / route
            executable.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' ' + shlex.quote(str(Path(__file__).resolve())) + ' --peer ' + route + ' "$@"\n')
            executable.chmod(0o700)
            environment['CODEX_API_KEY'] = 'controlled-not-real'
        def rows():
            return [json.loads(line) for line in log.read_text().splitlines() if line.strip()]
        def count(kind):
            return sum(event['kind'] == kind for event in rows())
        try:
            session = SurfaceSession([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono', *(['--ascii'] if size[0] == 60 else [])], project, environment, size, output / 'tui')
            app = Journey(session, data, output)
            wait_for(session, 'Storage: saved', timeout=40)
            session.command('model journey-fixed')
            wait_for(session, 'Reasoning')
            app.choose_auto('journey-fixed')
            app.request('Read source.txt without changing files. 연결 종료 확인 한글界e\u0301')
            session.send('\r', settle=False)
            app.approve_until(lambda: count('disconnect_ready') > 0 if stage == 'before' or route == 'claude' else count('submitted') > 0)
            if stage == 'after':
                def partial_visible():
                    if 'Evolution!' in session.display():
                        session.send(ESC)
                    return 'GP10_STREAMING' in session.display()

                until(session, partial_visible, 'submitted public partial was not visible', timeout=8)
            assert count('submitted') == (stage == 'after'), rows()
            assert app.claims(), 'controlled transport never acquired its actual task owner'
            app.request(FUTURE)
            session.send((ESC + '[D') * 3)
            session.send('\x13')
            cursor = len(FUTURE[:-3].encode())
            until(session, lambda: saved_request(app, FUTURE, cursor), 'future text/caret did not save before disconnect')
            release.touch()
            if stage == 'before':
                until(session, lambda: not app.claims() and 'Storage: saved' in session.display(), 'predispatch failure did not release its task lease', timeout=40)
            else:
                until(session, lambda: 'outcome unknown' in session.display().lower(), 'postdispatch disconnect lost unknown outcome', timeout=40)
                # Finished retires the durable active transport lease after
                # journal/usage settlement. Opaque uncertainty is a separate
                # process-local admission barrier, not a zombie DB claim.
                until(session, lambda: not app.claims() and 'Storage: saved' in session.display(),
                    'terminal external transport lease did not settle', timeout=40)
                counts_before_blocked_send = {kind: count(kind) for kind in ('peer_launch', 'query_created', 'submitted')}
                session.send('\r')
                until(session, lambda: any(reason in session.display() for reason in (
                    "Finish or cancel this partner",
                    'Local effects remain unknown')),
                    'unknown external outcome did not block a deliberate future Send', timeout=20)
                assert counts_before_blocked_send == {kind: count(kind) for kind in counts_before_blocked_send}, \
                    'unknown-effect admission launched another process/query or sent the future draft'
                assert not app.claims(), 'blocked future Send reacquired a transport lease'
                assert saved_request(app, FUTURE, cursor), 'blocked future Send changed exact draft/caret'
                session.capture('unknown-effects-blocked-deliberate-future-send')
            assert saved_request(app, FUTURE, cursor), 'disconnect changed saved future text/caret'
            app.workspace()
            session.command('response')
            session.capture('owned-terminal-response')
            terminal = session.display()
            if stage == 'before':
                assert 'failed' in terminal.lower(), terminal
            else:
                assert 'unknown' in terminal.lower(), terminal
            counts = {kind: count(kind) for kind in ('peer_launch', 'query_created', 'submitted')}
            session.command('connect')
            session.command('diagnostics')
            wait_for(session, 'Transport failed')
            session.capture('retained-transport-diagnostic')
            session.send('n')
            session.pump(.5)
            session.capture('explicit-existing-local-remediation')
            assert 'Transport failed' not in session.display(), 'next action did not open existing local setup'
            app.workspace()
            assert saved_request(app, FUTURE, cursor), 'remediation changed future draft/caret'
            # Inspecting the restored editor and inserting at its old Unicode
            # byte caret proves actual input ownership, not only DB retention.
            session.send('!')
            session.send('\x13')
            expected = FUTURE[:-3] + '!' + FUTURE[-3:]
            until(session, lambda: saved_request(app, expected, cursor + 1), 'remediation moved the restored insertion caret')
            assert counts == {kind: count(kind) for kind in counts}, 'local remediation launched a process/query or replayed a turn'
            assert app.db('SELECT count(*) FROM executions')[0][0] == 0
            assert not any(event['kind'] == 'finished' for event in rows()), 'controlled peer unexpectedly supplied a terminal result'
            assert PRIVATE_CANARY not in session.text, 'raw peer stdout/stderr entered the terminal'
            assert all(PRIVATE_CANARY.encode() not in path.read_bytes()
                for path in data.glob('workbench.sqlite3*')), 'raw peer output entered retained storage'
            result.update(status='passed', terminal=terminal, process_counts=counts,
                outcome='failed' if stage == 'before' else 'unknown',
                retained_claims=app.claims(), future_text=expected, future_cursor=cursor + 1)
            result['future_send_blocked'] = stage == 'after'
        except Exception:
            result['error'] = traceback.format_exc()
            if session is not None:
                session.capture('failure')
        finally:
            release.touch()
            save(output / 'wire.json', rows())
            if session is not None:
                try:
                    session.close()
                except Exception:
                    result.update(status='failed', close_error=traceback.format_exc())
            save(output / 'results.json', result)
    return result


def main():
    if '--peer' in sys.argv:
        peer(sys.argv[sys.argv.index('--peer') + 1])
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--node', type=Path, default=shutil.which('node'))
    parser.add_argument('--sidecar', type=Path, default=Path(__file__).resolve().parents[1] / 'sidecar')
    parser.add_argument('--route', choices=ROUTES)
    parser.add_argument('--stage', choices=('before', 'after'))
    parser.add_argument('--size', choices=('120x36', '60x18'))
    args = parser.parse_args()
    routes = (args.route,) if args.route else ROUTES
    if not args.binary.is_file():
        parser.error('an existing built binary is required')
    if 'claude' in routes and (args.node is None or not (args.sidecar / 'node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs').is_file()):
        parser.error('Claude rows require Node >=24 and installed complete sidecar dependencies')
    args.output.mkdir(parents=True, exist_ok=False)
    results = [row(args.binary.resolve(), Path(args.node).resolve() if args.node else None, args.sidecar.resolve(),
        args.output / f'{route}-{stage}-{size[0]}', route, stage, size)
        for route in routes for stage in ((args.stage,) if args.stage else ('before', 'after'))
        for size in ((tuple(map(int, args.size.split('x'))),) if args.size else ((120, 36), (60, 18)))]
    save(args.output / 'matrix.json', dict(binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(), rows=results,
        limits='Controlled actual stdio/process/UI evidence only; no live account/model calls, billing or provider interoperability claim.'))
    return int(any(result['status'] != 'passed' for result in results))


if __name__ == '__main__':
    raise SystemExit(main())
