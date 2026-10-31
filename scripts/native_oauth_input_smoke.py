#!/usr/bin/env python3
"""Credential-free native browser-login input/callback recovery in a real POSIX PTY.

Browser launchers are disposable no-op peers. Only invalid-state loopback callbacks
are supplied, so no authorization-code exchange or public identity/model request
can occur. Successful token exchange/persistence remain Rust fixture/native setup
gates, not a claim of live account eligibility.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import sqlite3
import sys
import tempfile
import time
import traceback
from urllib.parse import parse_qs, urlsplit

from terminal_smoke import Session, WINDOWS, confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC = '\x1b'


def run(binary, output, size):
    output.mkdir(parents=True)
    result = {'status': 'failed', 'size': list(size), 'cases': []}
    with tempfile.TemporaryDirectory(prefix='ev-native-oauth-input-') as temporary:
        root = Path(temporary)
        project, home, tools, data = [root / name for name in ('project', 'home', 'bin', 'data')]
        for directory in (project, home, tools):
            directory.mkdir(mode=0o700)
        for name in ('open', 'xdg-open'):
            opener = tools / name
            opener.write_text('#!' + sys.executable + '\nimport sys\nsys.exit(0)\n')
            opener.chmod(0o700)
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_',
                               'GOOGLE_', 'GEMINI_', 'AWS_', 'OPENCODE_', 'DEVIN_', 'WINDSURF_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), CODEX_HOME=str(home / '.codex'),
                           XDG_CONFIG_HOME=str(home / '.config'), XDG_DATA_HOME=str(home / '.local/share'),
                           XDG_CACHE_HOME=str(home / '.cache'), SSH_CONNECTION='controlled remote fixture',
                           PATH=os.pathsep.join([str(tools), environment.get('PATH', '')]))
        session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                          project, environment, size, output / 'terminal', controlling_tty=True)
        try:
            wait_for(session, 'Storage: saved')
            session.command('connect chatgpt')
            wait_for(session, 'Browser login')
            assert any('>' in line and 'Device login' in line for line in session.display().splitlines()), 'remote native account setup did not suggest Device'
            result['cases'].append('SSH native account chooser suggests Device without launching login')
            # Explicit Browser selection remains available, with neutral review.
            for _ in range(4):
                if any('>' in line and 'Browser login' in line for line in session.display().splitlines()):
                    break
                session.send(ESC + '[A')
            else:
                raise AssertionError('Explicit Browser method is not reachable')
            session.send('\r')
            wait_for(session, 'Review connection')
            wait_for(session, 'Browser login')
            assert 'Device login' not in session.display(), 'Refusing to authorize a live Device request in a Browser-only fixture'
            assert '[Back]' in session.display(), 'login review was not neutral'
            session.send('\t')
            session.send('\r')
            wait_for(session, 'OpenAI account sign-in')
            assert 'Callback ports unavailable' not in session.text, 'both registered callback ports are occupied; controlled callback cannot be qualified'
            match = re.search(r'https://auth\.openai\.com/oauth/authorize\?[^\r\n\x1b]+', session.text)
            assert match, 'native sign-in did not display its official manual browser URL'
            values = parse_qs(urlsplit(match.group(0)).query)
            callback = urlsplit(values['redirect_uri'][0])
            assert callback.hostname == '127.0.0.1' and callback.port in (1455, 1457)
            assert values['state'] and values['code_challenge_method'] == ['S256']
            with socket.create_connection((callback.hostname, callback.port), timeout=2) as peer:
                peer.sendall(b'GET /auth/callback?code=fixture-invalid&state=wrong-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n')
                assert b'400 Bad Request' in peer.recv(1024), 'wrong-state callback was admitted'
            assert session.alive(), 'unrelated callback terminated native login'
            result['cases'].append('real native loopback callback refuses wrong state without exchange or ending login')
            # Auth handoff disables bracketed paste: reproduce ordinary pasted Key
            # events, not a synthetic one-event bracketed Paste shortcut.
            pasted = values['redirect_uri'][0] + '?code=' + 'x' * 4096 + '&state=wrong-state'
            started = time.monotonic()
            session.send(pasted + '\r', settle=False)
            wait_for(session, 'Redirect not recognized', timeout=5)
            result['manual_input_seconds'] = round(time.monotonic() - started, 3)
            assert pasted not in session.text, 'hidden redirect was echoed'
            session.send('\x15')
            session.send(ESC)
            wait_for(session, 'Connections')
            wait_for(session, 'cancelled')
            session.capture('cancelled-with-actionable-recovery')
            assert not (home / '.eevee-connections').exists(), 'cancelled login created a saved account'
            with closing(sqlite3.connect(data / 'workbench.sqlite3')) as database:
                assert database.execute('SELECT count(*) FROM provider_journal').fetchone()[0] == 0
                assert database.execute('SELECT count(*) FROM executions').fetchone()[0] == 0
            result['cases'].append('4096-byte unbracketed hidden redirect is handled promptly; clear/cancel restores actionable TUI with no saved grant or task')
            session.send('\x03')
            confirm_exit(session)
            assert session.wait_exit() == 0
            result['status'] = 'passed'
        finally:
            # Authorization links and redirects are transient even on assertion
            # failure. Do not put a raw screen or exception display in reports.
            session.text = re.sub(r'https://auth\.openai\.com/oauth/authorize\?[^\r\n\x1b]+', '[authorization link omitted]', session.text)
            session.text = re.sub(r'(enter: )[A-Z0-9-]+', r'\1[device code omitted]', session.text)
            for event in session.events:
                if '/auth/callback?' in event.get('input', ''):
                    event['input'] = '[hidden fixture redirect omitted]'
            session.close()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if WINDOWS:
        parser.error('POSIX-only no-op browser isolation; native Windows browser launch is not intercepted')
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'journeys': [],
              'limits': 'Local invalid-state callback/input/cancellation only; no successful live OAuth, public identity request, account entitlement or model call.'}
    for size in ((120, 36), (60, 18)):
        try:
            report['journeys'].append(run(binary, output / str(size[0]), size))
        except Exception as error:
            site = traceback.extract_tb(error.__traceback__)[-1]
            report['journeys'].append({'status': 'failed', 'size': list(size),
                'error_type': type(error).__name__, 'function': site.name, 'line': site.lineno})
    if all(row['status'] == 'passed' for row in report['journeys']):
        report['status'] = 'passed'
    (output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'results': str(output / 'results.json')}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
