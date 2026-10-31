#!/usr/bin/env python3
"""Actual PowerShell parent under ConPTY, with isolated data and pinned sprites.

Complements the separate classic-console driver. Neither certifies another
Windows device's antivirus verdict or physical terminal rendering.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from terminal_smoke import Session, confirm_exit

parser = argparse.ArgumentParser()
parser.add_argument('--binary', required=True, type=Path)
parser.add_argument('--assets-dir', required=True, type=Path)
parser.add_argument('--output', required=True, type=Path)
args = parser.parse_args()
if os.name != 'nt':
    parser.error('native Windows only')
binary, assets, output = args.binary.resolve(), args.assets_dir.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=False)
report = {'status': 'failed', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'cases': [], 'limits': ['Real PowerShell under ConPTY, not classic conhost or the affected user device']}
quote = lambda value: "'" + str(value).replace("'", "''") + "'"
try:
    for width, height in [(120, 36), (60, 18)]:
        for images in ['default', 'off']:
            case = output / f'{width}x{height}-{images}'
            case.mkdir()
            project, home = case / 'project', case / 'home'
            project.mkdir(); home.mkdir()
            local = home / 'AppData/Local'
            local.mkdir(parents=True)
            shutil.copytree(assets, local / 'eevee/sprites')
            env = os.environ.copy()
            for key in list(env):
                upper = key.upper()
                if upper.startswith(('EV_', 'EEVEE_', 'OPENAI_', 'ANTHROPIC_', 'CLAUDE_', 'CODEX_', 'GEMINI_', 'XDG_')) or upper in ('TERM', 'COLORTERM', 'NO_COLOR', 'FORCE_COLOR', 'LANG', 'LC_ALL', 'LC_CTYPE', 'WT_SESSION', 'TERM_PROGRAM'):
                    env.pop(key)
            env.update(HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(local), APPDATA=str(home / 'AppData/Roaming'), RUST_BACKTRACE='1')
            env['HOMEDRIVE'], env['HOMEPATH'] = os.path.splitdrive(str(home))
            status = case / 'native-exit.txt'
            command = '& ' + quote(binary) + (' --images off' if images == 'off' else '') + '; $code = $LASTEXITCODE; [IO.File]::WriteAllText(' + quote(status) + ', [string]$code); exit $code'
            row = {'size': [width, height], 'images': images, 'status': 'failed'}
            session = None
            try:
                session = Session(['powershell.exe', '-NoLogo', '-NoProfile', '-Command', command], project, env, (width, height), case / 'terminal')
                deadline = time.monotonic() + 20
                while 'Storage: saved' not in session.display():
                    if status.exists() or not session.alive():
                        raise RuntimeError('native application ended before first saved UI')
                    if time.monotonic() >= deadline:
                        raise RuntimeError('first saved UI deadline exceeded')
                    session.pump(.05)
                session.capture('first-saved-ui')
                session.pump(.5)
                if status.exists() or not session.alive():
                    raise RuntimeError('native application ended without explicit quit')
                session.send('\x03')
                confirm_exit(session)
                if session.wait_exit() != 0:
                    raise RuntimeError('PowerShell/native app did not exit zero after explicit quit')
                code = int(status.read_text(encoding='utf-8-sig'))
                if code != 0:
                    raise RuntimeError('nonzero native exit')
                row.update(status='passed', first_saved_ui=True, explicit_quit_exit=code)
            except Exception as error:
                row.update(error=str(error), traceback=traceback.format_exc())
                if session is not None:
                    session.capture('failed')
            finally:
                if status.exists():
                    value = status.read_text(encoding='utf-8-sig').strip()
                    row['native_status'] = value
                    if value.lstrip('-').isdigit():
                        row['ntstatus_hex'] = f'0x{int(value) & 0xffffffff:08X}'
                if session is not None:
                    session.close()
                report['cases'].append(row)
                (output / 'results.json').write_text(json.dumps(report, indent=2))
    if all(row['status'] == 'passed' for row in report['cases']):
        report['status'] = 'passed'
finally:
    (output / 'results.json').write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
raise SystemExit(0 if report['status'] == 'passed' else 1)
