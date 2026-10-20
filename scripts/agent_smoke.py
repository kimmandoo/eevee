#!/usr/bin/env python3
"""Real PTY acceptance for passive discovery and explicit executable trust (POSIX)."""
import argparse
import json
import os
from pathlib import Path
import platform
import tempfile

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

END = '\x1b[F'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/agent-smoke')
    parser.add_argument('--installed-version', required=True, help='Independently observed Codex version')
    args = parser.parse_args()
    if os.name != 'posix':
        raise SystemExit('This fixture uses POSIX scripts; no native Windows acceptance is claimed.')
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'host': platform.platform(), 'cases': []}
    sessions = []

    def passed(name):
        result['cases'].append(name)
        print('passed:', name, flush=True)

    try:
        with tempfile.TemporaryDirectory(prefix='ev-agent-空 格-') as temporary:
            root = Path(temporary)
            project = root / 'project'
            project.mkdir()
            missing = root / 'empty-path'
            missing.mkdir()
            marker = project / 'probe-marker'
            executable = project / 'codex'

            def launch(name, path=None, size=(120, 36)):
                environment = terminal_environment('mono')
                if path is not None:
                    environment['PATH'] = str(path)
                for key in ('OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'GEMINI_API_KEY', 'GOOGLE_API_KEY', 'CODEX_HOME'):
                    environment.pop(key, None)
                environment['HOME'] = str(root / 'empty-home')
                session = Session([str(binary), '--data-dir', str(root / ('data-' + name)),
                                   '--images', 'off', '--color', 'mono'], project,
                                  environment, size, output / name)
                sessions.append(session)
                wait_for(session, 'Storage: saved')
                session.command('connect')
                wait_for(session, 'Connections')
                return session

            def preview(session):
                session.command('version')
                wait_for(session, 'Review version check')

            def approve(session):
                wait_for(session, 'Tab Choose')
                session.send('\t')
                wait_for(session, '[Check version]')
                session.send('\r')
                wait_for(session, 'Connections')

            def quit_session(session):
                session.send('\x1b')  # Actions -> Providers.
                session.send('\x1b')  # Providers -> workspace.
                session.command('quit')
                assert session.wait_exit() == 0
                session.close()

            session = launch('missing', missing)
            session.capture('missing-codex')
            for provider in ('claude', 'opencode', 'gemini', 'codex'):
                session.command('connect ' + provider)
                assert not marker.exists(), 'connection discovery executed repository code'
                session.capture('profile-' + provider)
            session.send('\x1b')
            session.send('\x1b')
            session.command('goal')
            session.paste('Local-only 漢字 goal')
            session.send('\r')
            wait_for(session, 'Storage: saved')
            session.command('quit')
            confirm_exit(session, '\t\r')
            assert session.wait_exit() == 0
            session.close()
            passed('missing-agent profiles preserve local goal editing and terminal restoration')

            executable.write_text('#!/bin/sh\nprintf ran >> "' + str(marker) + '"\nprintf "codex-cli 0.147.0\\n"\n')
            executable.chmod(0o755)
            session = launch('repository-trust', project)
            assert not marker.exists(), 'passive discovery executed repository code'
            preview(session)
            session.send('\r')
            assert not marker.exists(), 'neutral Enter executed repository code'
            preview(session)
            wait_for(session, 'Tab Choose')
            session.resize((39, 10))
            session.send('\t\r')
            assert not marker.exists(), 'undersized review executed repository code'
            session.resize((60, 18))
            session.capture('compact-trust')
            session.send('\x1b')
            assert not marker.exists(), 'cancel executed repository code'
            preview(session)
            approve(session)
            wait_for(session, 'Observed CLI version 0.147.0')
            assert marker.read_text() == 'ran', 'probe did not execute exactly once'
            session.capture('explicit-probe')
            quit_session(session)
            passed('repository executable requires reviewed explicit consent; neutral/cancel/undersize deny')

            marker.unlink()
            session = launch('replacement', project)
            preview(session)
            executable.write_text('#!/bin/sh\nprintf replaced >> "' + str(marker) + '"\nprintf "codex-cli 0.147.0\\n"\n')
            approve(session)
            wait_for(session, 'identity changed')
            assert not marker.exists(), 'changed executable bypassed trust revalidation'
            session.capture('changed-identity-refusal')
            quit_session(session)
            passed('executable replacement after preview refuses dispatch')

            secret = 'sk-preflight-private-canary-never-display'
            executable.write_text(f"#!/bin/sh\nprintf '%s\\n' 'codex-cli 0.147.0' '{secret}'\n")
            session = launch('unrecognized-output', project)
            preview(session)
            approve(session)
            wait_for(session, 'Unrecognized version output')
            assert secret not in session.text, 'raw probe output leaked to terminal'
            session.capture('raw-output-withheld')
            quit_session(session)
            passed('unrecognized probe output never exposes raw credential-like text')

            session = launch('installed-codex')
            preview(session)
            approve(session)
            wait_for(session, args.installed_version, timeout=15)
            session.capture('installed-codex-version')
            session.command('diagnostics')
            wait_for(session, 'Installed:')
            session.send(END)
            session.capture('installed-capabilities')
            session.idle()
            session.send('\x1b')  # Diagnostics -> Actions before the common exit path.
            quit_session(session)
            passed('actual installed Codex version observed without model request; idle emits nothing')
        result['status'] = 'passed'
    finally:
        for session in sessions:
            session.close()
        (output / 'results.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
