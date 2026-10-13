#!/usr/bin/env python3
"""Opt-in real auth initiation/cancellation. Isolated provider homes; never completes user login.

No OAuth URL, device code or native authentication transcript is retained. No model
request is sent. Linux/POSIX PTY evidence only; browser invocation is suppressed by
BROWSER=/bin/true so the human's desktop/account is not touched.
"""
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import time

from terminal_smoke import Session, terminal_environment
from workbench_smoke import wait_for

END = '\x1b[F'
RIGHT = '\x1b[C'
TITLE = 'Agent connections'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', action='store_true')
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--provider-runtime', required=True)
    parser.add_argument('--output', default='target/auth-smoke')
    args = parser.parse_args()
    if not args.live or os.name != 'posix':
        parser.error('--live and POSIX required; native official authentication will start and be cancelled')
    binary, runtime, output = Path(args.binary).resolve(), Path(args.provider_runtime).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = {'status': 'failed', 'cases': [], 'limits': ['No human OAuth consent or token exchange completed', 'No native Windows/macOS authentication qualification']}

    def passed(case):
        results['cases'].append(case)
        print('passed:', case, flush=True)

    with tempfile.TemporaryDirectory(prefix='ev-auth-空 格-') as temp:
        root = Path(temp)
        project, home, data = root / 'project', root / 'home', root / 'data'
        project.mkdir()
        home.mkdir()
        (home / '.codex').mkdir()
        environment = terminal_environment('mono')
        # This opt-in test exercises a user's native terminal, not headless CI.
        # Gemini treats these flags as headless even when stdin/stdout are PTYs.
        environment.pop('CI', None)
        environment.pop('GITHUB_ACTIONS', None)
        environment.update(HOME=str(home), CODEX_HOME=str(home / '.codex'), GEMINI_CLI_HOME=str(home),
                           XDG_CONFIG_HOME=str(home / '.config'), XDG_DATA_HOME=str(home / '.local/share'),
                           XDG_CACHE_HOME=str(home / '.cache'), BROWSER='/bin/true')
        environment['PATH'] = os.pathsep.join([str(runtime / 'node_modules/opencode-linux-x64/bin'),
                                              str(runtime / 'node_modules/.bin'), environment.get('PATH', '')])
        for key in list(environment):
            if key.startswith(('OPENAI_', 'CODEX_API_', 'ANTHROPIC_', 'CLAUDE_CODE_', 'GEMINI_API_', 'GOOGLE_', 'AWS_', 'OPENCODE_')):
                environment.pop(key, None)
        # Acquire a controlling TTY for the official auth child's foreground group.
        launcher = root / 'controlling_tty.py'
        launcher.write_text('import fcntl,os,sys,termios\nfcntl.ioctl(0,termios.TIOCSCTTY,0)\nos.execv(sys.argv[1],sys.argv[1:])\n')
        session = Session([sys.executable, str(launcher), str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                          project, environment, (120, 36), output / 'journey')
        try:
            wait_for(session, 'Storage: saved')
            session.send('g')
            session.paste('Keep this local task through authentication cancellation.')
            session.send('\r')
            wait_for(session, 'Storage: saved')
            session.send('o')
            wait_for(session, TITLE)

            def preview(key='l'):
                session.send(key)
                wait_for(session, 'Explicit authentication', timeout=40)
                session.send(END)
                wait_for(session, 'Enter Stay')

            def authorize():
                session.send('\t\r')

            def wait_native(predicate, description, timeout=60):
                deadline = time.monotonic() + timeout
                while not predicate():
                    if not session.alive() or time.monotonic() >= deadline:
                        raise AssertionError(description + ' (native output withheld)')
                    session.pump(.1)

            def cancel_native(label, markers):
                before = len(session.text)
                authorize()
                if label == 'gemini-google':
                    wait_native(lambda: TITLE in session.display() or any(text in session.display() for text in ('Login with Google', 'Sign in with Google', 'Select a theme', 'Do you trust this folder?')), 'official Gemini auth chooser')
                    assert TITLE not in session.display(), 'Gemini exited before its login chooser (native output withheld)'
                    if 'Do you trust this folder?' in session.display():
                        # Refuse native folder trust even for this disposable cwd.
                        session.send('\x1b[B\x1b[B\r')
                        wait_native(lambda: 'Login with Google' in session.display() or 'Sign in with Google' in session.display(), 'official Gemini Google choice')
                    if 'Select a theme' in session.display():
                        session.send('\r')
                        wait_native(lambda: 'Login with Google' in session.display() or 'Sign in with Google' in session.display(), 'official Gemini Google choice')
                    session.send('\r')
                wait_native(lambda: any(marker in session.text[before:] for marker in markers), label + ' official auth initiation')
                session.send('\x03')
                if label == 'gemini-google' and TITLE not in session.display():
                    session.send('\x03')
                wait_native(lambda: TITLE in session.display(), label + ' return to Eevee', 30)
                view = session.display().lower()
                assert any(word in view for word in ('cancelled', 'unknown', 'failed')), label + ' must not claim authenticated after cancellation'
                assert 'provider reports authentication configured' not in view
                session.capture(label + '-returned')
                passed(label + ' official auth starts, cancellation returns to the same local TUI without false success')

            preview()
            session.send('\r')
            wait_for(session, TITLE)
            preview()
            session.resize((60, 18))
            session.send(END + '\t\r')
            assert TITLE in session.display() or 'authentication' in session.display().lower()
            session.resize((120, 36))
            if 'Explicit authentication' in session.display():
                session.send('\x1b')
            wait_for(session, TITLE)
            passed('neutral Enter and undersize terminal cannot authorize login')

            session.send('m')
            wait_for(session, 'device authorization')
            preview()
            session.send('\x1b')
            session.send('m')
            preview('s')
            authorize()
            wait_native(lambda: 'Provider reports not logged in' in session.display(), 'real Codex empty-home login status', 30)
            session.capture('codex-not-authenticated')
            passed('real Codex status reports absent login without reading credentials or sending a model request')
            preview()
            cancel_native('codex-browser', ('Starting local login server', 'auth.openai.com', 'Log in with your browser'))
            session.send('m')
            preview()
            cancel_native('codex-device', ('auth.openai.com', 'one-time code', 'device code'))
            session.send('m')

            session.send('3')
            preview()
            cancel_native('opencode-chooser', ('Select provider', 'Select a provider', 'Add credential', 'Log in'))

            session.send('4')
            preview()
            cancel_native('gemini-google', ('accounts.google.com', 'Opening authentication page', 'Waiting for authentication'))

            session.send('2l')
            wait_for(session, 'cannot reuse claude.ai')
            session.capture('claude-sdk-subscription-restriction')
            passed('Claude SDK explains supported API/cloud route rather than reusing subscription OAuth')
            with closing(sqlite3.connect(data / 'workbench.sqlite3')) as db:
                assert db.execute('SELECT goal FROM tasks').fetchone()[0] == 'Keep this local task through authentication cancellation.'
                assert db.execute('SELECT count(*) FROM provider_journal').fetchone()[0] == 0
                assert db.execute('SELECT count(*) FROM executions').fetchone()[0] == 0
            session.send('\x1b')
            session.send('q')
            wait_for(session, 'Confirm exit')
            session.send(RIGHT + '\r')
            assert session.wait_exit() == 0
            passed('authentication never creates task execution evidence; safe exit restores original terminal modes')
            session.text = ''
            session.close()
            session = Session([sys.executable, str(launcher), str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                              project, dict(environment, CI='true'), (120, 36), output / 'headless-refusal')
            wait_for(session, 'Storage: saved')
            session.send('o')
            wait_for(session, TITLE)
            session.send('4l')
            wait_for(session, 'no login was started')
            session.capture('gemini-headless-refusal')
            session.send('\x1b')
            session.send('q')
            wait_for(session, 'Confirm exit')
            session.send(RIGHT + '\r')
            assert session.wait_exit() == 0
            passed('headless CI environment refuses native Gemini login with an actionable reason before launch')
            results['status'] = 'passed-initiation-and-cancellation-only'
        finally:
            # Session.close normally writes the raw transcript; auth output is secret-bearing.
            session.text = ''
            session.close()
            (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
