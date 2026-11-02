#!/usr/bin/env python3
"""Native PS5.1/PS7 installer invocation evidence using real qualified bytes.

No builds, network requests, accounts, execution-policy changes or mocks. Runs
both -File and the README's called ScriptBlock semantics, with offline archive
arguments replacing only the network transport. The latter installs, updates,
rejects a bad checksum, repairs an absent executable, launches the installed TUI
and explicitly quits, then uninstalls in the SAME PowerShell host. Retains caller
continuation/error markers and
ConPTY transcripts; does not certify another device or physical terminal.

  python scripts/windows_installer_smoke.py \
      --archive target/packages/ev-windows-x86_64.zip \
      --checksums target/packages/SHA256SUMS --output target/windows-installer
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

from terminal_smoke import Session, confirm_exit


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def encoded_command(source):
    return base64.b64encode(source.encode('utf-16-le')).decode('ascii')


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n', encoding='utf-8')


def environment(home):
    env = os.environ.copy()
    for key in list(env):
        upper = key.upper()
        if upper.startswith(('EV_', 'EEVEE_', 'OPENAI_', 'ANTHROPIC_', 'CLAUDE_',
                             'CODEX_', 'GEMINI_', 'XDG_')) or upper in (
                'TERM', 'COLORTERM', 'NO_COLOR', 'FORCE_COLOR', 'LANG', 'LC_ALL',
                'LC_CTYPE', 'WT_SESSION', 'TERM_PROGRAM'):
            env.pop(key)
    local, roaming = home / 'AppData/Local', home / 'AppData/Roaming'
    local.mkdir(parents=True)
    roaming.mkdir(parents=True)
    env.update(HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(local),
               APPDATA=str(roaming), RUST_BACKTRACE='1')
    env['HOMEDRIVE'], env['HOMEPATH'] = os.path.splitdrive(str(home))
    return env


def scriptblock_source(installer, archive, checksums, bad_checksums, case):
    """Use the exact & ([scriptblock]::Create(text)) call, not a -File wrapper."""
    destination = case / 'ScriptBlock install with spaces'
    binary = destination / 'ev.exe'
    arguments = (' -Archive ' + quote(archive) + ' -Checksums ' + quote(checksums)
                 + ' -InstallDir ' + quote(destination))
    failed_arguments = (' -Archive ' + quote(archive) + ' -Checksums '
                        + quote(bad_checksums) + ' -InstallDir '
                        + quote(destination))
    # These used to be overwritten by $script: assignments when invoked online.
    fields = ('animation', 'animationClock', 'animationFrame', 'animationTop',
              'oldCursorVisible', 'ownerProcess', 'ownerStartTicks', 'ballArt',
              'ballFlashArt', 'ballColors', 'ballUpper', 'ballLower', 'ballFull',
              'ballLight', 'ballMedium', 'ballAscii')
    source = [
        "$ErrorActionPreference = 'Continue'", 'Set-StrictMode -Off',
        '$scopeSentinel = [object]::new()',
        '$fields = @(' + ', '.join(quote(name) for name in fields) + ')',
        'foreach ($name in $fields) { Set-Variable -Scope Script -Name $name -Value $scopeSentinel }',
        '$beforePath = $env:PATH',
        '$beforeSecurity = [Net.ServicePointManager]::SecurityProtocol',
        '$installerText = [IO.File]::ReadAllText(' + quote(installer) + ')',
        'function Read-ProbeHash([string]$path) {',
        '    $inputStream = [IO.File]::OpenRead($path)',
        '    $sha = [Security.Cryptography.SHA256]::Create()',
        "    try { return [BitConverter]::ToString($sha.ComputeHash($inputStream)).Replace('-', '') }",
        '    finally { $sha.Dispose(); $inputStream.Dispose() }',
        '}',
        'function Assert-CallerState {',
        "    if ($ErrorActionPreference -cne 'Continue') { throw 'Caller error preference changed.' }",
        "    if ($env:PATH -cne $beforePath) { throw 'Caller PATH changed.' }",
        "    if ([Net.ServicePointManager]::SecurityProtocol -ne $beforeSecurity) { throw 'Caller TLS settings changed.' }",
        '    foreach ($name in $fields) {',
        '        if (-not [object]::ReferenceEquals((Get-Variable -Scope Script -Name $name -ValueOnly), $scopeSentinel)) {',
        '            throw (\'Caller script variable changed: \' + $name)',
        '        }',
        '    }',
        '}',
        'try {',
        '    $PSVersionTable | Out-String | Set-Content -LiteralPath ' + quote(case / 'engine.txt'),
        '    & ([scriptblock]::Create($installerText))' + arguments,
        '    Assert-CallerState',
        "    if ($null -ne $evInstallerUndefinedScopeProbe) { throw 'Unexpected strict-mode probe value.' }",
        '    [IO.File]::WriteAllText(' + quote(case / 'continued-after-install') + ", 'continued')",
        '    & ([scriptblock]::Create($installerText))' + arguments,
        '    Assert-CallerState',
        '    [IO.File]::WriteAllText(' + quote(case / 'continued-after-update') + ", 'continued')",
        '    $beforeHash = Read-ProbeHash ' + quote(binary),
        '    $beforeOwnerHash = Read-ProbeHash ' + quote(destination / '.ev-install/.native-install'),
        '    $caught = $false',
        '    try { & ([scriptblock]::Create($installerText))' + failed_arguments + ' }',
        '    catch {',
        '        $caught = $true',
        "        if ($_.Exception.Message -notmatch '(?i)checksum|sha-?256') { throw }",
        '        [IO.File]::WriteAllText(' + quote(case / 'expected-error.txt') + ', ($_ | Out-String))',
        '    }',
        "    if (-not $caught) { throw 'Bad checksum did not raise a catchable error.' }",
        '    Assert-CallerState',
        "    if ($null -ne $evInstallerUndefinedScopeProbe) { throw 'Unexpected strict-mode probe value.' }",
        '    if ((Read-ProbeHash ' + quote(binary) + ') -cne $beforeHash) {',
        "        throw 'Failed update changed installed bytes.'",
        '    }',
        '    if ((Read-ProbeHash ' + quote(destination / '.ev-install/.native-install') + ') -cne $beforeOwnerHash) {',
        "        throw 'Failed update changed installation ownership.'",
        '    }',
        '    [IO.File]::WriteAllText(' + quote(case / 'installed-sha256.txt') + ', $beforeHash)',
        '    [IO.File]::WriteAllText(' + quote(case / 'continued-after-failure') + ", 'caught; continued')",
        '    [IO.File]::Delete(' + quote(binary) + ')',
        '    & ([scriptblock]::Create($installerText))' + arguments,
        '    Assert-CallerState',
        '    if ((Read-ProbeHash ' + quote(binary) + ') -cne $beforeHash -or',
        '        (Read-ProbeHash ' + quote(destination / '.ev-install/.native-install') + ') -cne $beforeOwnerHash) {',
        "        throw 'Repair did not restore the exact verified installation.'",
        '    }',
        '    [IO.File]::WriteAllText(' + quote(case / 'continued-after-repair') + ", 'continued')",
        '    & ' + quote(binary) + ' --images off',
        '    $nativeExit = $LASTEXITCODE',
        '    [IO.File]::WriteAllText(' + quote(case / 'native-exit.txt') + ', [string]$nativeExit)',
        "    if ($nativeExit -ne 0) { throw ('Installed TUI failed: ' + $nativeExit) }",
        '    [IO.File]::WriteAllText(' + quote(case / 'continued-after-native-quit') + ", 'continued')",
        '    & ([scriptblock]::Create($installerText)) -Uninstall -InstallDir ' + quote(destination) + ' -NoAnimation',
        '    Assert-CallerState',
        '    if ([IO.File]::Exists(' + quote(binary) + ') -or [IO.Directory]::Exists(' + quote(destination / '.ev-install') + ')) {',
        "        throw 'Owned uninstall left installed files.'",
        '    }',
        '    [IO.File]::WriteAllText(' + quote(case / 'continued-after-uninstall') + ", 'continued')",
        '    [IO.File]::WriteAllText(' + quote(case / 'scriptblock-passed') + ", 'passed')",
        '    exit 0',
        '} catch {',
        '    [IO.File]::WriteAllText(' + quote(case / 'unexpected-error.txt') + ', ($_ | Out-String))',
        '    exit 1',
        '}',
    ]
    return '\n'.join(source) + '\n'


def run_file_case(shell, installer, archive, checksums, bad_checksums, case, env):
    destination = case / 'File install with spaces'
    rows = []
    for name, arguments, expected in [
        ('install', ['-Archive', archive, '-Checksums', checksums], 0),
        ('update', ['-Archive', archive, '-Checksums', checksums], 0),
        ('failure', ['-Archive', archive, '-Checksums', bad_checksums], 1),
        ('modified-binary', ['-Archive', archive, '-Checksums', checksums], 1),
        ('missing-binary-bad-checksum', ['-Archive', archive, '-Checksums', bad_checksums], 1),
        ('missing-binary-invalid-metadata', ['-Archive', archive, '-Checksums', checksums], 1),
        ('missing-binary-invalid-marker', ['-Archive', archive, '-Checksums', checksums], 1),
        ('missing-binary-invalid-notices', ['-Archive', archive, '-Checksums', checksums], 1),
        ('missing-binary-extra-metadata', ['-Archive', archive, '-Checksums', checksums], 1),
        ('missing-binary-reviewed', ['-Archive', archive, '-Checksums', checksums,
                                     '-ExpectedOwner', '0' * 64], 1),
        ('missing-binary-running-owner', ['-Archive', archive, '-Checksums', checksums,
                                          '-OwnerProcessId', '1'], 1),
        ('missing-binary-uninstall', ['-Uninstall'], 1),
        ('repair', ['-Archive', archive, '-Checksums', checksums], 0),
        ('uninstall', ['-Uninstall'], 0),
    ]:
        binary, marker = destination / 'ev.exe', destination / '.ev-install/.native-install'
        mutation = {
            'modified-binary': binary,
            'missing-binary-invalid-metadata': marker.parent / 'LICENSE',
            'missing-binary-invalid-marker': marker,
            'missing-binary-invalid-notices': marker.parent / 'THIRD_PARTY_NOTICES',
            'missing-binary-extra-metadata': marker.parent / 'foreign.txt',
        }.get(name)
        if name == 'missing-binary-bad-checksum':
            installed_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
            binary.unlink()
        if mutation is not None:
            original = mutation.read_bytes() if mutation.exists() else None
            mutation.write_bytes((original or b'') + b'\nmodified')
        before = {str(path.relative_to(destination)): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in destination.rglob('*') if path.is_file()} if expected else None
        command = [shell, '-NoLogo', '-NoProfile', '-NonInteractive', '-File',
                   str(installer), *map(str, arguments), '-InstallDir', str(destination), '-NoAnimation']
        completed = subprocess.run(command, env=env, cwd=case / 'project',
                                   capture_output=True, timeout=120)
        (case / ('file-' + name + '-stdout.bin')).write_bytes(completed.stdout)
        (case / ('file-' + name + '-stderr.bin')).write_bytes(completed.stderr)
        row = {'operation': name, 'exit_code': completed.returncode, 'expected_exit_code': expected}
        rows.append(row)
        save(case / 'file-results.json', rows)
        if completed.returncode != expected:
            raise RuntimeError(f'-File {name} returned {completed.returncode}, expected {expected}')
        if before is not None:
            after = {str(path.relative_to(destination)): hashlib.sha256(path.read_bytes()).hexdigest()
                     for path in destination.rglob('*') if path.is_file()}
            if before != after:
                raise RuntimeError('-File rejected operation changed retained installation: ' + name)
        if mutation is not None:
            if original is None:
                mutation.unlink()
            else:
                mutation.write_bytes(original)
        if name == 'repair' and hashlib.sha256(binary.read_bytes()).hexdigest() != installed_hash:
            raise RuntimeError('-File repair did not restore the exact verified executable')
        if name == 'uninstall' and (binary.exists() or marker.parent.exists()):
            raise RuntimeError('-File uninstall left installed files')
    return rows


def run_scriptblock_case(shell, source, case, env):
    (case / 'invocation.ps1').write_text(source, encoding='utf-8')
    session = None
    try:
        env = env.copy()
        env.pop('CI', None)
        env.update(TERM='xterm-256color', COLORTERM='truecolor')
        session = Session([shell, '-NoLogo', '-NoProfile', '-EncodedCommand', encoded_command(source)],
                          case / 'project', env, (120, 36), case / 'terminal')
        deadline = time.monotonic() + 120
        while 'Storage: saved' not in session.display():
            if not session.alive() or (case / 'unexpected-error.txt').exists():
                raise RuntimeError('ScriptBlock host ended before installed TUI startup; inspect retained error/transcript')
            if time.monotonic() >= deadline:
                raise RuntimeError('Installed TUI startup deadline exceeded')
            session.pump(.05)
        for marker in ('continued-after-install', 'continued-after-update', 'continued-after-failure',
                       'continued-after-repair'):
            if not (case / marker).exists():
                raise RuntimeError('Missing caller continuation marker: ' + marker)
        if not any(marker in session.text for marker in ('▀', '▄', '|=====(o)====|', '|=====(O)====|')):
            raise RuntimeError('Default interactive installer did not render its real animation')
        session.capture('installed-ui')
        session.pump(.5)
        if not session.alive() or (case / 'native-exit.txt').exists():
            raise RuntimeError('Installed TUI ended without explicit quit')
        session.send('\x03')
        confirm_exit(session)
        # Installer status text legitimately scrolls the shell's primary buffer;
        # assert mode restoration, not the direct-TUI primary-screen sentinel.
        if session.wait_exit(tui=False) != 0:
            raise RuntimeError('ScriptBlock host did not complete after explicit native quit')
        for marker in ('continued-after-native-quit', 'continued-after-uninstall', 'scriptblock-passed'):
            if not (case / marker).exists():
                raise RuntimeError('Missing caller continuation marker: ' + marker)
        if int((case / 'native-exit.txt').read_text(encoding='utf-8-sig')) != 0:
            raise RuntimeError('Installed TUI quit returned nonzero')
        return {'continuation_after_install': True, 'continuation_after_update': True,
                'default_interactive_animation_observed': True,
                'caught_failure_and_continued': True, 'caller_state_preserved': True,
                'missing_binary_repaired': True,
                'installed_tui_explicit_quit_exit': 0, 'continuation_after_uninstall': True,
                'installed_sha256': (case / 'installed-sha256.txt').read_text(encoding='utf-8-sig')}
    except Exception:
        if session is not None:
            session.capture('failed')
        raise
    finally:
        if session is not None:
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--installer', type=Path, default=Path('install.ps1'))
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--checksums', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--powershell', action='append', help='repeat for engines; default requires powershell.exe and pwsh.exe')
    args = parser.parse_args()
    if os.name != 'nt':
        parser.error('native Windows only')
    installer, archive, checksums, output = (path.resolve() for path in
                                           (args.installer, args.archive, args.checksums, args.output))
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'archive_sha256': hashlib.sha256(archive.read_bytes()).hexdigest(),
              'installer_sha256': hashlib.sha256(installer.read_bytes()).hexdigest(), 'cases': [],
              'limits': ['Offline transport with actual called ScriptBlock semantics; no online service or affected-device certification',
                         'Real PowerShell under native Windows ConPTY, not physical terminal rendering']}
    checksum_text = checksums.read_text(encoding='utf-8-sig')
    bad_lines = []
    replaced = False
    for line in checksum_text.splitlines():
        if line.endswith('  ev-windows-x86_64.zip'):
            line = ('1' if line[0] == '0' else '0') + line[1:]
            replaced = True
        bad_lines.append(line)
    if not replaced:
        raise RuntimeError('Checksums fixture has no ev-windows-x86_64.zip entry')
    bad_checksums = output / 'bad-SHA256SUMS'
    bad_checksums.write_text('\n'.join(bad_lines) + '\n', encoding='ascii')
    try:
        for index, executable in enumerate(args.powershell or ['powershell.exe', 'pwsh.exe']):
            case = output / f'engine-{index + 1}'
            case.mkdir()
            (case / 'project').mkdir()
            env = environment(case / 'home')
            row = {'powershell': executable, 'status': 'failed'}
            report['cases'].append(row)
            try:
                shell = shutil.which(executable)
                if shell is None:
                    raise RuntimeError('Required native PowerShell engine unavailable: ' + executable)
                row['executable'] = shell
                version = subprocess.run([shell, '-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
                                          '$PSVersionTable.PSVersion.ToString()'], env=env,
                                         capture_output=True, timeout=30, check=True)
                row['version_output'] = version.stdout.decode('utf-8-sig', errors='replace').strip()
                row['file'] = run_file_case(shell, installer, archive, checksums, bad_checksums, case, env)
                source = scriptblock_source(installer, archive, checksums, bad_checksums, case)
                row['scriptblock'] = run_scriptblock_case(shell, source, case, env)
                row['status'] = 'passed'
            except Exception as error:
                row.update(error=str(error), traceback=traceback.format_exc())
            finally:
                save(output / 'results.json', report)
        if report['cases'] and all(row['status'] == 'passed' for row in report['cases']):
            report['status'] = 'passed'
    finally:
        save(output / 'results.json', report)
    print(json.dumps(report, indent=2))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
