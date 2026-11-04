#!/usr/bin/env python3
"""Actual connection UI: private save/replacement, explicit session override and restart.

Synthetic credentials and isolated homes only. Loopback metadata validates which
source was used; no inference, paid account or official OAuth endpoint is called.
Native account post-issuer persistence is separately covered by the Rust regression.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import traceback

import native_connection_smoke as native
from response_smoke import SurfaceJourney, SurfaceSession
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ROUTES = {'openai': 'responses', 'anthropic': 'messages', 'google': 'gemini',
          'compatible': 'chat_completions'}


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def protect_new_fixture_root(root):
    if os.name != 'nt':
        return
    assert root.name.startswith('ev-credential-ui-') and not list(root.iterdir())
    powershell = Path(os.environ['SYSTEMROOT']) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
    script = r"""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$p = $env:EV_CREDENTIAL_FIXTURE_ROOT
$owner = (Get-Acl -LiteralPath $p).GetOwner([System.Security.Principal.SecurityIdentifier])
$acl = [System.Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$rights = [System.Security.AccessControl.FileSystemRights]::FullControl
$inherit = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $user, $rights, $inherit, [System.Security.AccessControl.PropagationFlags]::None,
    [System.Security.AccessControl.AccessControlType]::Allow)
$acl.AddAccessRule($rule)
[System.IO.Directory]::SetAccessControl($p, $acl)
$after = (Get-Acl -LiteralPath $p).GetOwner([System.Security.Principal.SecurityIdentifier])
if (-not $after.Equals($owner)) { throw 'Fixture construction changed default ownership' }
"""
    environment = dict(os.environ, EV_CREDENTIAL_FIXTURE_ROOT=str(root))
    result = subprocess.run([str(powershell), '-NoLogo', '-NoProfile', '-NonInteractive',
                             '-Command', script], env=environment, capture_output=True, timeout=25)
    assert result.returncode == 0, 'could not construct a private owned Windows fixture root: ' + result.stderr.decode('utf-8', errors='replace')


def quit_session(journey):
    journey.workspace()
    wait_for(journey.session, 'Storage: saved')
    journey.session.command('quit')
    confirm_exit(journey.session)
    assert journey.session.wait_exit() == 0
    journey.session.close()


def run(binary, output, route, size, unsafe_home=False):
    output.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'route': route, 'size': list(size),
              'scenario': 'unsafe-home' if unsafe_home else 'saved-session-restart'}
    journey = None
    with tempfile.TemporaryDirectory(prefix='ev-credential-ui-') as temporary:
        with native.Peer(ROUTES[route], 'sk-fixture-private-first-c214f9', False) as peer:
            try:
                protect_new_fixture_root(Path(temporary).resolve())
                journey = SurfaceJourney(binary, output, Path(temporary).resolve(), size,
                                         size[0] == 60, route, ROUTES[route], False, True, peer)
                wait_for(journey.session, 'Storage: saved')
                path = journey.home / '.eevee-connections' / (route + '.json')
                if unsafe_home:
                    if os.name == 'nt':
                        icacls = Path(os.environ['SYSTEMROOT']) / 'System32' / 'icacls.exe'
                        changed = subprocess.run([str(icacls), str(journey.home), '/grant',
                                                  '*S-1-1-0:(WD)'], capture_output=True, timeout=20)
                        assert changed.returncode == 0, 'could not construct owned unsafe-home fixture'
                    else:
                        journey.home.chmod(0o777)
                    from connection_smoke_helpers import open_setup, fill_text, focus_field
                    open_setup(journey.session, route, advanced=True)
                    journey.private_paste(peer.secret)
                    fill_text(journey.session, 'endpoint', peer.base)
                    fill_text(journey.session, 'model', native.MODEL)
                    focus_field(journey.session, 'save')
                    journey.session.send('\r')
                    journey.capture('unsafe-home-save-result')
                    # Drain a real shutdown before observing effects; lack of a
                    # quick metadata frame alone cannot prove save refusal.
                    quit_session(journey)
                    assert not path.parent.exists(), 'refused save created a credential directory'
                    assert not peer.snapshot(), 'refused save sent metadata/inference'
                    peer.assert_clean()
                    result.update(status='passed', credential_store_created=False,
                                  metadata_requests=0, inference_requests=0)
                    return result

                native.Journey.setup(journey, secret=peer.secret, initial=True)
                first = json.loads(path.read_text(encoding='utf-8'))
                assert first['secret'] == peer.secret and first['base_url'] == peer.base
                assert first['model'] == native.MODEL and first['protocol'] == ROUTES[route]
                # An unrelated broken provider file must not displace a selected
                # app-owned or explicit in-memory credential after a good save.
                config = journey.home / 'config' / 'eevee' / 'providers.toml'
                config.parent.mkdir(parents=True, exist_ok=True)
                config.write_text('invalid provider settings = [', encoding='utf-8')
                journey.workspace()
                replacement = journey.secrets[1]
                native.Journey.setup(journey, secret=replacement)
                durable = path.read_bytes()
                assert json.loads(durable)['secret'] == replacement
                journey.workspace()
                session_secret = 'sk-fixture-explicit-session-d071b2'
                journey.secrets.append(session_secret)
                native.Journey.setup(journey, secret=session_secret, persist=False)
                assert path.read_bytes() == durable, 'session override changed saved credential bytes'
                assert not peer.snapshot('POST'), 'connection setup performed inference'
                journey.capture('explicit-session-kept-durable-credential')
                quit_session(journey)

                peer.secret = replacement
                journey.session = SurfaceSession([*journey.argv, '--continue'], journey.project,
                                                  journey.environment, size, output / 'restart')
                wait_for(journey.session, 'Storage: saved')
                # No key is re-entered. The fresh process must authenticate from
                # the durable route, not its previous process's session override.
                native.Journey.setup(journey, secret=None)
                assert json.loads(path.read_bytes())['secret'] == replacement
                requests = peer.snapshot('GET')
                assert len(requests) == 4, 'save/replacement/session/restart must each qualify metadata'
                assert all(row['authentication_matches_selected_source'] for row in requests)
                assert not peer.snapshot('POST'), 'restart/setup performed inference'
                assert config.read_text(encoding='utf-8') == 'invalid provider settings = ['
                journey.capture('restart-used-saved-source-without-reentering-key')
                quit_session(journey)
                # Windows byte-range ownership locks intentionally deny raw reads
                # while the writer is alive; scan the coherent closed store.
                journey.privacy()
                peer.assert_clean()
                result.update(status='passed', metadata_requests=len(requests), inference_requests=0,
                              replaced_saved_credential=True, session_preserved_durable_bytes=True,
                              fresh_process_used_saved_credential_without_reentry=True,
                              unrelated_provider_file_unchanged=True,
                              credentials_absent_from_terminal_task_and_project=True)
            except Exception as error:
                result.update(error=str(error), traceback=traceback.format_exc())
                if journey is not None:
                    journey.capture('failed')
            finally:
                if journey is not None:
                    journey.session.close()
                save(output / 'http.json', peer.snapshot())
                save(output / 'results.json', result)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--routes', nargs='+', choices=tuple(ROUTES), default=list(ROUTES))
    args = parser.parse_args(argv)
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'platform': os.name, 'journeys': [],
              'limits': 'Actual TUI/process/files and authenticated loopback metadata with synthetic keys. No paid model, official OAuth exchange or real user credential store. Account post-issuer save is covered by the native Rust regression.'}
    for size in ((120, 36), (60, 18)):
        for route in args.routes:
            destination = output / ('%s-%dx%d' % (route, *size))
            result['journeys'].append(run(binary, destination, route, size))
    result['journeys'].append(run(binary, output / 'unsafe-home', args.routes[0], (120, 36), True))
    if all(row['status'] == 'passed' for row in result['journeys']):
        result['status'] = 'passed'
    save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'output': str(output)}))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
