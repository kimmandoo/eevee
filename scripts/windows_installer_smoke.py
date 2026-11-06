#!/usr/bin/env python3
"""Native PS5.1/PS7 installer invocation evidence using real qualified bytes.

No builds, network requests, accounts, execution-policy changes or mocks. Runs
both -File and the README's called ScriptBlock semantics, with offline archive
arguments replacing only the network transport. The latter installs, updates,
rejects a bad checksum, repairs an absent executable, launches the installed TUI
and explicitly quits, then uninstalls in the SAME PowerShell host. Exercises raw
user PATH persistence, duplicate prevention, fresh-process command discovery and
denied-write rollback. Requires --allow-user-path-changes on a disposable Windows
account: the real user PATH is temporarily changed and restored in finally.
Retains caller continuation/error markers and ConPTY transcripts; does not
certify another device or physical terminal.

  python scripts/windows_installer_smoke.py \
      --archive target/packages/ev-windows-x86_64.zip \
      --checksums target/packages/SHA256SUMS --output target/windows-installer \
      --allow-user-path-changes
"""
from __future__ import annotations

import argparse
import base64
import hashlib
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

from terminal_smoke import Session, confirm_exit, run_owned


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


def read_user_path():
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, 'Environment') as key:
        try:
            return winreg.QueryValueEx(key, 'Path')
        except FileNotFoundError:
            return None


def write_user_path(value):
    import winreg
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, 'Environment') as key:
        if value is None:
            try:
                winreg.DeleteValue(key, 'Path')
            except FileNotFoundError:
                pass
        else:
            winreg.SetValueEx(key, 'Path', 0, value[1], value[0])


@contextmanager
def restore_registry_acl():
    # The parent retains READ_CONTROL/WRITE_DAC without requesting SetValue.
    # Restore the original ACL even if the denied-write child times out or dies.
    import ctypes
    from ctypes import wintypes
    import winreg
    security = ctypes.WinDLL('advapi32', use_last_error=True)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    pointer = ctypes.c_void_p
    security.GetSecurityInfo.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                                        pointer, pointer, ctypes.POINTER(pointer),
                                        pointer, ctypes.POINTER(pointer)]
    security.GetSecurityInfo.restype = wintypes.DWORD
    security.SetSecurityInfo.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                                        pointer, pointer, pointer, pointer]
    security.SetSecurityInfo.restype = wintypes.DWORD
    kernel.LocalFree.argtypes = [pointer]
    kernel.LocalFree.restype = pointer

    def acl_bytes(value):
        if not value.value:
            return None
        size = ctypes.c_ushort.from_address(value.value + 2).value
        return ctypes.string_at(value, size)

    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, 'Environment', 0, 0x20000 | 0x40000) as key:
        dacl, descriptor = pointer(), pointer()
        error = security.GetSecurityInfo(int(key), 4, 4, None, None,
                                         ctypes.byref(dacl), None, ctypes.byref(descriptor))
        if error:
            raise ctypes.WinError(error)
        original_acl = acl_bytes(dacl)
        try:
            yield
        finally:
            try:
                error = security.SetSecurityInfo(int(key), 4, 4, None, None, dacl, None)
                if error:
                    raise ctypes.WinError(error)
                restored, restored_descriptor = pointer(), pointer()
                error = security.GetSecurityInfo(int(key), 4, 4, None, None,
                                                 ctypes.byref(restored), None,
                                                 ctypes.byref(restored_descriptor))
                if error:
                    raise ctypes.WinError(error)
                try:
                    if acl_bytes(restored) != original_acl:
                        raise RuntimeError('Denied-write fixture did not restore the original registry DACL')
                finally:
                    kernel.LocalFree(restored_descriptor)
            finally:
                kernel.LocalFree(descriptor)


@contextmanager
def user_path_fixture(case):
    import winreg
    original = read_user_path()
    save(case / 'user-path-before.json', {'value': None if original is None else original[0],
                                        'kind': None if original is None else original[1]})
    # Keep literal expandable placeholders, quotes, spaces, Unicode and empty
    # entries. They must survive byte-for-byte, not be expanded and rewritten.
    seed = ('"%USERPROFILE%\\unrelated path";"' + str(case / 'unrelated \uacbd\ub85c')
            + '";;')
    try:
        with restore_registry_acl():
            write_user_path((seed, winreg.REG_EXPAND_SZ))
            yield seed
    finally:
        write_user_path(original)
        save(case / 'user-path-restored.json', {'restored': read_user_path() == original})
        if read_user_path() != original:
            raise RuntimeError('Failed to restore the original user PATH')


def assert_user_path(expected):
    import winreg
    if read_user_path() != (expected, winreg.REG_EXPAND_SZ):
        raise RuntimeError('User PATH value or expandable-string type changed unexpectedly')


def probe_persisted_command(shell, destination, case, env):
    # A new shell inherits the Python runner, never the installer's updated PATH.
    # Refresh it from persisted machine/user state, as a new desktop session does.
    source = '\n'.join([
        "$ErrorActionPreference = 'Stop'",
        "$env:PATH = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')",
        "$command = Get-Command ev -CommandType Application -ErrorAction Stop",
        'if (-not [string]::Equals([IO.Path]::GetFullPath($command.Source), ' + quote(destination / 'ev.exe')
        + ", [StringComparison]::OrdinalIgnoreCase)) { throw ('Wrong ev discovered: ' + $command.Source) }",
        '& ev --version',
        "if ($LASTEXITCODE -ne 0) { throw 'Persisted PATH ev --version failed.' }",
    ])
    completed = run_owned([shell, '-NoLogo', '-NoProfile', '-NonInteractive',
                           '-EncodedCommand', encoded_command(source)],
                          env=env, cwd=case / 'project', timeout=30)
    (case / 'persisted-path-stdout.bin').write_bytes(completed.stdout)
    (case / 'persisted-path-stderr.bin').write_bytes(completed.stderr)
    if completed.returncode != 0:
        raise RuntimeError('Fresh-process persisted PATH command discovery failed')


def denied_path_source(installer, arguments, case):
    """Inject a real deny; the parent owns exact held-handle DACL restoration."""
    parameters = {str(arguments[index]).lstrip('-'): arguments[index + 1]
                  for index in range(0, len(arguments) - 1, 2)}
    invocation = ' '.join('-' + name + ' ' + quote(value) for name, value in parameters.items())
    return '\n'.join([
        "$ErrorActionPreference = 'Stop'",
        '$beforePath = $env:PATH',
        "$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree, [Security.AccessControl.RegistryRights]::FullControl)",
        'function Read-RegistrySecurity {',
        "    if ($null -ne $key.PSObject.Methods['GetAccessControl']) { return $key.GetAccessControl() }",
        '    return [Microsoft.Win32.RegistryAclExtensions]::GetAccessControl($key)',
        '}',
        'function Write-RegistrySecurity($value) {',
        "    if ($null -ne $key.PSObject.Methods['SetAccessControl']) { $key.SetAccessControl($value) }",
        '    else { [Microsoft.Win32.RegistryAclExtensions]::SetAccessControl($key, $value) }',
        '}',
        '$acl = Read-RegistrySecurity',
        '$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User',
        "$rule = [Security.AccessControl.RegistryAccessRule]::new($sid, [Security.AccessControl.RegistryRights]::SetValue, [Security.AccessControl.AccessControlType]::Deny)",
        '$acl.AddAccessRule($rule)',
        'try {',
        '    Write-RegistrySecurity $acl',
        '    $caught = $false',
        '    try { & ([scriptblock]::Create([IO.File]::ReadAllText(' + quote(installer) + '))) '
        + invocation + ' -NoAnimation }',
        '    catch {',
        '        $caught = $true',
        "        if ($_.Exception.Message -notmatch '(?i)user PATH.*previous installation was preserved') { throw }",
        '        [IO.File]::WriteAllText(' + quote(case / 'denied-path-error.txt') + ', ($_ | Out-String))',
        '    }',
        "    if (-not $caught) { throw 'Denied user PATH write was not reported as failed.' }",
        '    $childHost = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName',
        '    & $childHost -NoLogo -NoProfile -NonInteractive -File ' + quote(installer)
        + ' ' + invocation + ' -NoAnimation',
        "    if ($LASTEXITCODE -eq 0) { throw '-File denied user PATH write returned success.' }",
        "    if ($env:PATH -cne $beforePath) { throw 'Denied PATH registration changed the invoking process PATH.' }",
        '} finally { $key.Dispose() }',
        'exit 0',
    ])


def scriptblock_source(installer, archive, checksums, bad_checksums, case):
    """Use the exact & ([scriptblock]::Create(text)) call, not a -File wrapper."""
    destination = case / 'ScriptBlock install with spaces \uacbd\ub85c'
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
        '$expectedPath = $beforePath + $(if ([string]::IsNullOrEmpty($beforePath) -or $beforePath.EndsWith(";")) { "" } else { ";" }) + ' + quote(destination),
        "$pathKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')",
        "try { $beforeUserPath = $pathKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } finally { $pathKey.Dispose() }",
        '$expectedUserPath = $beforeUserPath + $(if ([string]::IsNullOrEmpty($beforeUserPath) -or $beforeUserPath.EndsWith(";")) { "" } else { ";" }) + ' + quote(destination),
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
        "    if ($env:PATH -cne $expectedPath) { throw 'Caller PATH did not retain the exact one-time addition.' }",
        "    $pathKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')",
        '    try {',
        "        if ($pathKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) -cne $expectedUserPath -or",
        "            $pathKey.GetValueKind('Path') -ne [Microsoft.Win32.RegistryValueKind]::ExpandString) { throw 'Persistent user PATH changed unexpectedly.' }",
        '    } finally { $pathKey.Dispose() }',
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
        "    if (-not [string]::Equals((Get-Command ev -CommandType Application -ErrorAction Stop).Source, " + quote(binary) + ", [StringComparison]::OrdinalIgnoreCase)) { throw 'ScriptBlock ev command discovery failed.' }",
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
        '    & ev --images off',
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


def run_file_case(shell, installer, archive, checksums, bad_checksums, case, env, seed):
    import winreg
    destination = case / 'File install with spaces \uacbd\ub85c'
    expected_path = seed
    rows = []
    for name, arguments, expected in [
        ('initial-failure', ['-Archive', archive, '-Checksums', bad_checksums], 1),
        ('initial-denied-user-path', ['-Archive', archive, '-Checksums', checksums], 0),
        ('install', ['-Archive', archive, '-Checksums', checksums], 0),
        ('denied-user-path', ['-Archive', archive, '-Checksums', checksums], 0),
        ('equivalent-path-update', ['-Archive', archive, '-Checksums', checksums], 0),
        ('update', ['-Archive', archive, '-Checksums', checksums], 0),
        ('repeat-update', ['-Archive', archive, '-Checksums', checksums], 0),
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
        denied = name in ('initial-denied-user-path', 'denied-user-path')
        if denied:
            expected_path = seed
            write_user_path((expected_path, winreg.REG_EXPAND_SZ))
        elif name in ('equivalent-path-update', 'update'):
            # An unquoted equivalent is functional and needs no duplicate.
            # A quoted spelling is preserved but must not suppress registration.
            alias = str(destination).replace('\\', '/').upper() + '/'
            expected_path = seed + ('"' + alias + '"' if name == 'update' else alias)
            write_user_path((expected_path, winreg.REG_EXPAND_SZ))
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
                  for path in destination.rglob('*') if path.is_file()} if expected or denied else None
        invocation = [*map(str, arguments), '-InstallDir', str(destination), '-NoAnimation']
        if denied:
            source = denied_path_source(installer, invocation, case)
            command = [shell, '-NoLogo', '-NoProfile', '-NonInteractive',
                       '-EncodedCommand', encoded_command(source)]
            with restore_registry_acl():
                completed = run_owned(command, env=env, cwd=case / 'project', timeout=120)
        else:
            command = [shell, '-NoLogo', '-NoProfile', '-NonInteractive', '-File',
                       str(installer), *invocation]
            completed = run_owned(command, env=env, cwd=case / 'project', timeout=120)
        (case / ('file-' + name + '-stdout.bin')).write_bytes(completed.stdout)
        (case / ('file-' + name + '-stderr.bin')).write_bytes(completed.stderr)
        row = {'operation': name, 'exit_code': completed.returncode, 'expected_exit_code': expected,
               'invocation': 'ScriptBlock and -File with real registry write denial' if denied else '-File'}
        rows.append(row)
        save(case / 'file-results.json', rows)
        if completed.returncode != expected:
            raise RuntimeError(f'{name} returned {completed.returncode}, expected {expected}')
        if before is not None:
            after = {str(path.relative_to(destination)): hashlib.sha256(path.read_bytes()).hexdigest()
                     for path in destination.rglob('*') if path.is_file()}
            if before != after:
                raise RuntimeError('Rejected operation changed retained installation: ' + name)
        if name == 'install':
            expected_path = seed + str(destination)
        elif name == 'update':
            expected_path += ';' + str(destination)
        assert_user_path(expected_path)
        if name in ('install', 'equivalent-path-update', 'update', 'repeat-update', 'repair'):
            probe_persisted_command(shell, destination, case, env)
            row['fresh_process_persisted_ev_discovered'] = True
        if mutation is not None:
            if original is None:
                mutation.unlink()
            else:
                mutation.write_bytes(original)
        if name == 'repair' and hashlib.sha256(binary.read_bytes()).hexdigest() != installed_hash:
            raise RuntimeError('-File repair did not restore the exact verified executable')
        if name == 'uninstall' and (binary.exists() or marker.parent.exists()):
            raise RuntimeError('-File uninstall left installed files')
    save(case / 'file-results.json', rows)
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
    parser.add_argument('--allow-user-path-changes', action='store_true',
                        help='acknowledge temporary real user PATH/ACL changes on a disposable account')
    args = parser.parse_args()
    if not args.allow_user_path_changes:
        parser.error('--allow-user-path-changes is required on a disposable Windows account; the smoke temporarily changes real user PATH and registry ACLs')
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
                with user_path_fixture(case) as seed:
                    row['file'] = run_file_case(shell, installer, archive, checksums, bad_checksums, case, env, seed)
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
