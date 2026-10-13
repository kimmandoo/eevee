#!/usr/bin/env python3
"""Opt-in real Codex→OpenCode handoff/ACP acceptance plus honest SDK/Gemini blockers."""
import argparse
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time

from terminal_smoke import Session, terminal_environment
from workbench_smoke import wait_for

END = '\x1b[F'
HOME = '\x1b[H'
RIGHT = '\x1b[C'
DELETE = '\x1b[3~'
IDLE = 'IDLE / explicit New'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', action='store_true')
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/provider-smoke')
    parser.add_argument('--provider-runtime', required=True, help='Isolated installed native peer npm prefix')
    parser.add_argument('--sidecar', required=True, help='Explicit installed pinned optional SDK main.ts')
    args = parser.parse_args()
    if not args.live:
        parser.error('--live required: bounded real Codex/OpenCode model requests')
    binary, output, runtime = Path(args.binary).resolve(), Path(args.output).resolve(), Path(args.provider_runtime).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': [], 'blockers': []}
    sessions = []

    def passed(name):
        result['cases'].append(name)
        print('passed:', name, flush=True)

    def cleanup():
        for session in sessions:
            session.close()

    try:
        with ExitStack() as stack:
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='ev-providers-空 格-'))).resolve()
            stack.callback(cleanup)
            project, data = root / 'project', root / 'data'
            project.mkdir()
            environment = terminal_environment('mono')
            environment['PATH'] = os.pathsep.join([str(runtime / 'node_modules/opencode-linux-x64/bin'),
                                                   str(runtime / 'node_modules/.bin'), environment.get('PATH', '')])
            environment['EEVEE_CLAUDE_SIDECAR'] = str(Path(args.sidecar).resolve())
            # Disposable, inherited peer configuration; no global/provider config file mutation.
            environment['OPENCODE_CONFIG_CONTENT'] = json.dumps({'permission': {'*': 'ask'}})
            session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                              project, environment, (120, 36), output / 'journey')
            sessions.append(session)
            wait_for(session, 'Storage: saved')
            session.send('\x1b')  # Leave the clean first-run Goal composer.

            def task():
                with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                    return db.execute('SELECT goal,next_action FROM tasks ORDER BY id LIMIT 1').fetchone()

            def journals():
                with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                    return [json.loads(row[0]) for row in db.execute('SELECT record FROM provider_journal')]

            def until(predicate, description, seconds=180):
                deadline = time.monotonic() + seconds
                while not predicate():
                    assert session.alive(), description + ': app exited'
                    assert time.monotonic() < deadline, description + '\n' + session.display()
                    session.pump(0.1)

            def goal(text):
                old = task()
                session.send('g')
                wait_for(session, 'Enter Save locally')
                if old:
                    session.send(HOME)
                    for offset in range(0, len(old[0]), 32):
                        session.send(DELETE * min(32, len(old[0]) - offset))
                session.paste(text)
                session.send('\r')
                until(lambda: task() is not None and task()[0] == text, 'goal commit', 20)

            def preview(resume=False):
                session.send('r' if resume else 'n')
                wait_for(session, 'LOCAL PREVIEW', timeout=40)
                session.send(END)
                wait_for(session, 'Enter Stay', timeout=40)
                session.capture('preview-' + str(len(result['cases'])))

            def run():
                session.send('\t\r')

            def select_agent(digit, label):
                session.send(digit)
                wait_for(session, 'IDLE AGENT SWITCH')
                session.send(END)
                wait_for(session, 'Enter Stay')
                session.capture('switch-' + digit)
                session.send('\t\r')
                wait_for(session, 'Espeon / ' + label + ' planning')

            goal('Plan a two-step local note workflow and state one concise decision. Include EEVEE_HANDOFF_CONTINUED. Do not read files or run tools.')
            session.send('e')
            wait_for(session, 'Espeon / Codex planning')
            preview()
            run()
            wait_for(session, IDLE, timeout=180)
            for _ in range(128):
                session.send('d')
                if 'Enter Save locally' in session.display():
                    break
                session.send('\x1b[B')
            else:
                raise AssertionError('no selected Codex plan/message')
            session.send('\r')
            until(lambda: bool(task()[1]), 'approved Codex decision committed', 20)
            original_decision = task()[1]
            if 'Espeon /' not in session.display():
                session.send('e')
            select_agent('3', 'OpenCode ACP')
            preview()
            session.capture('outgoing-portable-handoff')
            # Exact payload excludes provider IDs despite the locally stored decision provenance.
            assert 'User-selected, unverified Codex Plan; thread' not in session.display()
            run()
            wait_for(session, IDLE, timeout=180)
            session.capture('real-opencode-planning')
            assert 'ACP prompt stopped: end_turn' in session.display(), session.display()
            assert 'agent message / complete' in session.display(), session.display()
            if 'EEVEE_HANDOFF_CONTINUED' not in session.display():
                session.send(END)
                session.capture('real-opencode-planning-tail')
            assert 'EEVEE_HANDOFF_CONTINUED' in session.display(), session.display()
            session.send(HOME)
            passed('real Codex-to-OpenCode continuation uses reviewed portable facts not provider sessions/grants')

            select_agent('1', 'Codex')
            assert 'Session-local resumable thread:' in session.display(), 'old Codex session not retained under original provider'
            select_agent('3', 'OpenCode ACP')
            passed('idle agent switches preserve original per-provider sessions and reset permissions')
            session.send('s')
            wait_for(session, 'LOCAL PREVIEW', timeout=40)
            session.send(END)
            wait_for(session, 'Enter Stay')
            run()
            wait_for(session, 'Displayed ', timeout=60)
            session.capture('real-opencode-session-list')
            assert 'session/list' in session.display() and 'no model' in session.display(), session.display()
            passed('OpenCode negotiated session/list inspection runs without creating or prompting a model session')

            session.send('\x1b')
            goal("Invoke your bash tool now with exactly `printf 'EEVEE_%s_%s' ACP TOOL_OUTPUT`. "
                 "Do not ask a conversational permission question: the ACP permission callback must pause the tool "
                 "before execution. Do not write files. If the tool permission is denied, stop and report denial; "
                 "otherwise report its observed stdout.")
            session.send('e')
            session.send('b')  # Deliberately deselect stale old-revision handoff before a fresh task preview.
            preview()
            run()
            wait_for(session, 'PERMISSION / EXACT PROVIDER RESPONSE', timeout=180)
            session.capture('real-acp-permission-deny')
            session.send('\r')
            wait_for(session, IDLE, timeout=180)
            passed('real ACP permission denied with its exact offered response')

            preview()
            run()
            wait_for(session, 'PERMISSION / EXACT PROVIDER RESPONSE', timeout=180)
            session.send(END)
            for _ in range(16):
                if any(line.strip().startswith('│ > Allow once') or '> Allow once:' in line for line in session.display().splitlines()):
                    break
                session.send('\t')
            else:
                raise AssertionError('peer did not offer once-only Allow option')
            session.capture('real-acp-permission-allow')
            session.send('\r')
            wait_for(session, IDLE, timeout=180)
            until(lambda: any(row['agent'] == 'OpenCode' and 'EEVEE_ACP_TOOL_OUTPUT' in row['text']
                              and 'completed' in row['text'] for row in journals()), 'ACP final native-tool journal', 20)
            session.capture('real-acp-native-tool-completion')
            passed('ACP allow-once completes actual provider-owned tool with origin-labelled journal')

            preview(resume=True)
            run()
            wait_for(session, 'PERMISSION / EXACT PROVIDER RESPONSE', timeout=180)
            session.send('c')
            wait_for(session, IDLE, timeout=30)
            session.send('\t\r')
            if 'Enter Save locally' in session.display():
                session.send('\x1b')
            assert 'PROVIDER REQUEST' not in session.display()
            passed('real ACP explicit session load and pending-permission cancellation reject stale response')

            select_agent('2', 'Claude Agent')
            session.send('n')
            until(lambda: 'Run blocked' in session.display() or 'requires a permitted' in session.display(), 'SDK credential blocker', 40)
            session.send(HOME)
            session.capture('sdk-permitted-auth-blocker')
            assert 'subscription' in session.display().lower() or 'API' in session.display(), session.display()
            result['blockers'].append('Claude SDK: permitted API/Bedrock/Vertex credentials unavailable; installed subscription not reused')
            session.send('\x1b')
            wait_for(session, IDLE, timeout=10)
            passed('installed real SDK remains blocked without permitted credentials; no subscription fallback')

            select_agent('4', 'Gemini ACP')
            session.send('s')
            wait_for(session, 'LOCAL PREVIEW', timeout=40)
            session.send(END)
            wait_for(session, 'Enter Stay')
            run()
            wait_for(session, 'Method NOT invoked', timeout=60)
            session.capture('real-gemini-unavailable-session-list')
            assert 'did not negotiate sessionCapabilities.list' in session.display(), session.display()
            assert 'No session/new/load/prompt' in session.display(), session.display()
            passed('Gemini missing session/list is explained on the real TUI and never invoked')
            session.send('\x1b')
            wait_for(session, IDLE)
            preview()
            run()
            wait_for(session, IDLE, timeout=60)
            session.capture('real-gemini-authentication-refusal')
            assert any(word in session.display().lower() for word in ('api key', 'auth', '401', 'credential')), session.display()
            result['blockers'].append('Gemini ACP: real session/new fails401 for missing configured credentials')
            passed('same ACP adapter initializes Gemini but reports actual authentication refusal without prompt')
            assert task()[1] == original_decision
            session.send('\x1b')
            session.send('q')
            wait_for(session, 'Confirm exit')
            session.send(RIGHT + '\r')
            assert session.wait_exit() == 0
            session.close()
        result['status'] = 'passed-with-external-auth-blockers'
    finally:
        cleanup()
        (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
