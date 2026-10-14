#!/usr/bin/env python3
"""Native auth lifecycle coverage; supplied Eevee binary must already be built.

--fixture is credential-free: a local listener checks terminal ownership, callback
lifetime, explicit method selection and cancellation through the real TUI.
--wsl-device starts the real pinned Codex device flow on WSL, keeps callback port
1455 occupied, observes the official code prompt, then cancels without signing in.
--live starts/cancels official providers in isolated homes without human consent;
--ant adds Claude's Anthropic Console OAuth handoff through the official ant CLI.
No OAuth URL, device code or live native transcript is retained; BROWSER=/bin/true.
"""
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import sqlite3
import sys
import tempfile
import time

from terminal_smoke import Session, run_owned, terminal_environment
from workbench_smoke import wait_for

END = '\x1b[F'
RIGHT = '\x1b[C'
TITLE = 'Connections'

FIXTURE = r'''
import json, os, socket, sys
from pathlib import Path

root = Path(os.environ["EEVEE_AUTH_FIXTURE"])
args = sys.argv[1:]
with (root / "invocations.jsonl").open("a") as trace:
    trace.write(json.dumps({"argv": args, "pid": os.getpid()}) + "\n")
if args == ["--version"]:
    print("codex-cli 0.152.1")
    sys.exit(0)
if args == ["login", "status"]:
    print("Not logged in", file=sys.stderr)
    sys.exit(1)
if args == ["login", "--device-auth"]:
    print("Fixture device authorization unavailable; no fallback", flush=True)
    sys.exit(1)
if args != ["login"]:
    sys.exit(99)
print("Fixture waiting for terminal input", flush=True)
sys.stdin.readline()
with socket.socket() as server:
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen()
    (root / "listener.json").write_text(json.dumps({
        "pid": os.getpid(), "port": server.getsockname()[1],
        "foreground": os.tcgetpgrp(0) == os.getpgrp(),
    }))
    print("Fixture callback listener ready", flush=True)
    while True:
        connection, peer = server.accept()
        with connection:
            request = b""
            while b"\r\n\r\n" not in request and len(request) < 8192:
                chunk = connection.recv(1024)
                if not chunk:
                    break
                request += chunk
            body, status = b"ok", b"200 OK"
            complete = request.startswith(b"GET /complete ")
            connection.sendall(b"HTTP/1.1 " + status + b"\r\nContent-Length: " +
                               str(len(body)).encode() + b"\r\nConnection: close\r\n\r\n" + body)
            if complete:
                break
sys.exit(0)
'''


def fixture_journey(binary, output):
    results = {'status': 'failed', 'cases': [], 'limits': [
        'Local POSIX fixture, not provider authentication or WSL host-browser forwarding',
        'No OAuth consent, token exchange, or model request',
    ]}
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='ev-auth-fixture-') as temp:
        root = Path(temp)
        project, home, data, tools = [root / name for name in ('project', 'home', 'data', 'bin')]
        for directory in (project, home, tools, home / '.codex'):
            directory.mkdir()
        executable = tools / 'codex'
        executable.write_text('#!' + sys.executable + '\n' + FIXTURE)
        executable.chmod(0o700)
        launcher = root / 'controlling_tty.py'
        launcher.write_text('import fcntl,os,sys,termios\nfcntl.ioctl(0,termios.TIOCSCTTY,0)\nos.execv(sys.argv[1],sys.argv[1:])\n')
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GEMINI_', 'GOOGLE_', 'AWS_', 'OPENCODE_')):
                environment.pop(key, None)
        environment.update(HOME=str(home), CODEX_HOME=str(home / '.codex'),
                           XDG_CONFIG_HOME=str(home / '.config'), XDG_DATA_HOME=str(home / '.local/share'),
                           XDG_CACHE_HOME=str(home / '.cache'), BROWSER='/bin/true',
                           EEVEE_AUTH_FIXTURE=str(root), SSH_CONNECTION='fixture remote terminal',
                           EEVEE_AUTH_WSL_FIXTURE='0',
                           PATH=os.pathsep.join([str(tools), environment.get('PATH', '')]))
        session = Session([sys.executable, str(launcher), str(binary), '--data-dir', str(data),
                           '--images', 'off', '--color', 'mono'], project, environment, (120, 36), output / 'fixture')

        def invocations():
            path = root / 'invocations.jsonl'
            return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

        def argv():
            return [entry['argv'] for entry in invocations()]

        def preview(label):
            session.send('\r')
            wait_for(session, 'Review connection')
            wait_for(session, label)
            session.send(END)
            wait_for(session, 'Tab Choose')

        def authorize():
            session.send('\t\r')

        def listener():
            wait_for(session, 'Fixture waiting for terminal input')
            session.send('\r')
            wait_for(session, 'Fixture callback listener ready')
            value = json.loads((root / 'listener.json').read_text())
            assert value['foreground'], 'native child does not own terminal foreground'
            return value

        def request(value, path):
            with socket.create_connection(('127.0.0.1', value['port']), timeout=2) as connection:
                connection.sendall(('GET ' + path + ' HTTP/1.1\r\nHost: localhost\r\n\r\n').encode())
                assert b'200 OK' in connection.recv(256)

        def closed(value):
            try:
                with socket.create_connection(('127.0.0.1', value['port']), timeout=.5):
                    raise AssertionError('callback listener survives return to Eevee')
            except ConnectionRefusedError:
                pass
            try:
                os.kill(value['pid'], 0)
            except ProcessLookupError:
                return
            raise AssertionError('native login process survives return to Eevee')

        try:
            wait_for(session, 'Storage: saved')
            session.send('\x1b')
            session.command('connect')
            wait_for(session, TITLE)
            session.send('\r')
            wait_for(session, 'ACTIONS')
            session.send('\r')
            wait_for(session, 'CHOOSE SIGN-IN METHOD')
            wait_for(session, '> ChatGPT / device authorization (default)')
            preview('device authorization')
            session.send('\r')  # Neutral Back; not authorization.
            wait_for(session, TITLE)
            assert argv() == [], 'discovery/review ran a provider without consent'
            preview('device authorization')
            authorize()
            wait_for(session, 'Authentication/status operation failed')
            assert argv() == [['--version'], ['login', '--device-auth']], 'unapproved device fallback'
            results['cases'].append('remote default uses exact device argv; failure cannot launch browser or model')
            session.send('\x1b[A')  # Explicit Browser selection despite remote default.
            preview('browser OAuth')
            authorize()
            value = listener()
            for _ in range(3):
                request(value, '/waiting')
                session.pump(.2)
                os.kill(value['pid'], 0)
                assert argv()[-1] == ['login'], 'login returned before its callback'
            request(value, '/complete')
            wait_for(session, 'Provider reports not logged in')
            closed(value)
            assert argv() == [['--version'], ['login', '--device-auth'],
                              ['--version'], ['login'], ['login', 'status']]
            results['cases'].append('foreground login/listener stays live through callback; reviewed status refresh follows exit')
            (root / 'listener.json').unlink()
            preview('browser OAuth')
            authorize()
            value = listener()
            request(value, '/waiting')
            session.send('\x03')
            wait_for(session, 'Authentication cancelled')
            closed(value)
            assert argv()[-2:] == [['--version'], ['login']], 'cancel triggered an unapproved operation'
            wait_for(session, 'localhost connection refused')
            results['cases'].append('Ctrl+C reaps login/listener, restores TUI and offers callback recovery without retry')
            with closing(sqlite3.connect(data / 'workbench.sqlite3')) as db:
                assert db.execute('SELECT count(*) FROM provider_journal').fetchone()[0] == 0
                assert db.execute('SELECT count(*) FROM executions').fetchone()[0] == 0
            session.send('\x1b')
            session.send('\x1b')
            session.send('\x1b')
            session.command('quit')
            assert session.wait_exit() == 0
            results['cases'].append('no task/model execution; final terminal modes restored')
            results['status'] = 'passed-local-fixture'
        finally:
            session.close()
            for entry in invocations():
                try:
                    os.killpg(entry['pid'], signal.SIGKILL)
                except ProcessLookupError:
                    pass
            (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    for case in results['cases']:
        print('passed:', case, flush=True)


def wsl_device_journey(binary, output, codex):
    if 'microsoft' not in os.uname().release.lower():
        raise RuntimeError('--wsl-device requires real WSL, not simulated environment flags')
    executable = Path(codex or shutil.which('codex') or '').resolve()
    version = run_owned([str(executable), '--version'], timeout=15, text=True)
    if version.returncode or version.stdout.strip() != 'codex-cli 0.152.1':
        raise RuntimeError('--wsl-device requires the real pinned Codex 0.152.1 executable')
    output.mkdir(parents=True, exist_ok=True)
    results = {'status': 'failed', 'provider': version.stdout.strip(),
               'wsl_kernel': os.uname().release, 'limits': [
                   'Official device authorization initiation and cancellation only',
                   'No human consent, token exchange, or completed account login',
                   'No Windows settings, firewall rules or port reservations changed']}
    with tempfile.TemporaryDirectory(prefix='ev-wsl-device-') as temporary, socket.socket() as blocker:
        root = Path(temporary)
        project, home, tools, data = [root / name for name in ('project', 'home', 'bin', 'data')]
        for directory in (project, home, tools, home / '.codex'):
            directory.mkdir()
        # A browser fallback cannot acquire its normal callback port.
        blocker.bind(('127.0.0.1', 1455))
        blocker.listen()
        trace = root / 'invocations.jsonl'
        wrapper = tools / 'codex'
        wrapper.write_text(
            '#!' + sys.executable + '\nimport json,os,sys\n'
            + 'with open(' + repr(str(trace)) + ', "a") as log:\n'
            + '    log.write(json.dumps({"argv":sys.argv[1:],"pid":os.getpid()})+"\\n")\n'
            + 'os.execv(' + repr(str(executable)) + ',[' + repr(str(executable)) + ']+sys.argv[1:])\n')
        wrapper.chmod(0o700)
        launcher = root / 'tty.py'
        launcher.write_text('import fcntl,os,sys,termios\nfcntl.ioctl(0,termios.TIOCSCTTY,0)\nos.execv(sys.argv[1],sys.argv[1:])\n')
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GEMINI_', 'GOOGLE_', 'AWS_', 'OPENCODE_', 'SSH_')):
                environment.pop(key, None)
        environment.update(HOME=str(home), CODEX_HOME=str(home / '.codex'),
                           XDG_CONFIG_HOME=str(home / '.config'), XDG_DATA_HOME=str(home / '.local/share'),
                           XDG_CACHE_HOME=str(home / '.cache'), BROWSER='/bin/true',
                           PATH=os.pathsep.join([str(tools), environment.get('PATH', '')]))
        session = Session([sys.executable, str(launcher), str(binary), '--data-dir', str(data),
                           '--images', 'off', '--color', 'mono'], project, environment,
                          (120, 40), output / 'device')

        def wait_private(predicate, label, timeout=60):
            deadline = time.monotonic() + timeout
            while not predicate():
                assert session.alive() and time.monotonic() < deadline, label + ' (native output withheld)'
                session.pump(.05)

        def visible(text):
            wait_private(lambda: text in session.display(), text)

        def invocations():
            return [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []

        try:
            visible('Storage: saved')
            session.send('\x1b')
            session.command('connect')
            visible(TITLE)
            session.send('\r')
            visible('ACTIONS')
            session.send('\r')
            visible('CHOOSE SIGN-IN METHOD')
            visible('> ChatGPT / device authorization (default)')
            session.send('\r')
            visible('Review connection')
            visible('["login", "--device-auth"]')
            session.send(END)
            visible('Tab Choose')
            start = len(session.text)
            session.send('\t\r')
            wait_private(lambda: '/codex/device' in session.text[start:],
                         'official device-code authorization prompt')
            assert [entry['argv'] for entry in invocations()] == [
                ['--version'], ['login', '--device-auth']], 'non-device operation launched'
            # The official CLI must still be waiting for the user's authorization.
            login_pid = invocations()[-1]['pid']
            os.kill(login_pid, 0)
            assert blocker.getsockname() == ('127.0.0.1', 1455)
            results['official_device_prompt_observed'] = True
            results['callback_port_occupied_throughout'] = True
            session.send('\x03')
            wait_private(lambda: TITLE in session.display()
                         and any(word in session.display().lower() for word in ('cancelled', 'failed', 'unknown')),
                         'native device cancellation returns to Connections')
            assert 'Provider reports authentication configured' not in session.display()
            results['return_classification'] = next(
                word for word in ('cancelled', 'failed', 'unknown') if word in session.display().lower())
            try:
                os.kill(login_pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError('official device login survives cancellation')
            assert not (home / '.codex/auth.json').exists(), 'unexpected credential creation'
            with closing(sqlite3.connect(data / 'workbench.sqlite3')) as db:
                assert db.execute('SELECT count(*) FROM provider_journal').fetchone()[0] == 0
                assert db.execute('SELECT count(*) FROM executions').fetchone()[0] == 0
            session.command('quit')
            assert session.wait_exit() == 0
            results.update(status='passed-official-wsl-device-initiation',
                           provider_argv=[entry['argv'] for entry in invocations()],
                           cancelled_child_reaped=True, terminal_restored=True,
                           native_transcript_retained=False)
            print('passed: real WSL official Codex device flow starts with callback port blocked; cancellation restores TUI, no browser fallback or credentials')
        finally:
            if session.alive():
                session.send('\x03')
                session.pump(1)
            session.text = ''
            session.close()
            for entry in invocations():
                try:
                    os.killpg(entry['pid'], signal.SIGKILL)
                except ProcessLookupError:
                    pass
            (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--live', action='store_true')
    mode.add_argument('--fixture', action='store_true')
    mode.add_argument('--wsl-device', action='store_true')
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--provider-runtime')
    parser.add_argument('--codex', help='real installed Codex executable for --wsl-device')
    parser.add_argument('--ant', help='real installed ant executable for the --live Claude Console OAuth case')
    parser.add_argument('--output', default='target/auth-smoke')
    args = parser.parse_args()
    if os.name != 'posix':
        parser.error('POSIX required; native Windows authentication is not qualified by this script')
    if args.wsl_device:
        wsl_device_journey(Path(args.binary).resolve(), Path(args.output).resolve(), args.codex)
        return
    if args.fixture:
        fixture_journey(Path(args.binary).resolve(), Path(args.output).resolve())
        return
    if not args.provider_runtime:
        parser.error('--provider-runtime is required with --live')
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
        environment['PATH'] = os.pathsep.join(([str(Path(args.ant).resolve().parent)] if args.ant else []) +
                                             [str(runtime / 'node_modules/opencode-linux-x64/bin'),
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
            session.send('\x1b')  # Leave the clean first-run Goal composer.
            session.command('goal')
            session.paste('Keep this local task through authentication cancellation.')
            session.send('\r')
            wait_for(session, 'Storage: saved')
            session.command('connect')
            wait_for(session, TITLE)
            # Deliberately start the browser route on every host; do not assume
            # the environment-aware default is browser on WSL or SSH.
            session.command('connect-method')
            wait_for(session, 'Review connection', timeout=40)
            default_is_device = 'device authorization' in session.display()
            session.send('\x1b')
            if default_is_device:
                session.command('method-next')

            def preview(command='connect-method'):
                session.command(command)
                wait_for(session, 'Review connection', timeout=40)
                session.send(END)
                wait_for(session, 'Tab Choose')

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
            assert 'Review connection' in session.display() or TITLE in session.display()
            session.resize((120, 36))
            if 'Review connection' in session.display():
                session.send('\x1b')
            wait_for(session, TITLE)
            passed('neutral Enter and undersize terminal cannot authorize login')

            session.command('method-next')
            preview()
            wait_for(session, 'device authorization')
            session.send('\x1b')
            session.command('method-next')
            preview('status')
            authorize()
            wait_native(lambda: 'Provider reports not logged in' in session.display(), 'real Codex empty-home login status', 30)
            session.capture('codex-not-authenticated')
            passed('real Codex status reports absent login without reading credentials or sending a model request')
            preview()
            cancel_native('codex-browser', ('Starting local login server', 'auth.openai.com', 'Log in with your browser'))
            session.command('method-next')
            preview()
            cancel_native('codex-device', ('auth.openai.com', 'one-time code', 'device code'))
            session.command('method-next')

            session.send('\x1b[D\x1b[B\x1b[B\r')  # Visible provider list: Codex -> OpenCode.
            preview()
            cancel_native('opencode-chooser', ('Select provider', 'Select a provider', 'Add credential', 'Log in'))

            session.send('\x1b[D\x1b[B\r')  # Visible provider list: OpenCode -> Gemini.
            preview()
            cancel_native('gemini-google', ('accounts.google.com', 'Opening authentication page', 'Waiting for authentication'))

            if args.ant:
                console = home / '.config/anthropic'
                session.send('\x1b[D\x1b[A\x1b[A\r')  # Visible provider list: Gemini -> Claude.
                preview('status')
                authorize()
                wait_native(lambda: 'Provider reports not logged in' in session.display(), 'real ant empty-profile status', 30)
                passed('claude-console status reports an absent Console profile without reading credentials or sending a model request')
                for _ in range(2):
                    preview()
                    cancel_native('claude-console', ('platform.claude.com/oauth/authorize', 'Paste it here', 'paste the code here'))
                    assert not (console / 'credentials').exists(), 'cancelled Console login wrote credentials'
                    session.command('method-next')

            with closing(sqlite3.connect(data / 'workbench.sqlite3')) as db:
                assert db.execute('SELECT goal FROM tasks').fetchone()[0] == 'Keep this local task through authentication cancellation.'
                assert db.execute('SELECT count(*) FROM provider_journal').fetchone()[0] == 0
                assert db.execute('SELECT count(*) FROM executions').fetchone()[0] == 0
            session.send('\x1b')
            session.command('quit')
            wait_for(session, 'Confirm exit')
            session.send(RIGHT + '\r')
            assert session.wait_exit() == 0
            passed('authentication never creates task execution evidence; safe exit restores original terminal modes')
            results['status'] = 'passed-initiation-and-cancellation-only'
        finally:
            # Session.close normally writes the raw transcript; auth output is secret-bearing.
            session.text = ''
            session.close()
            (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
