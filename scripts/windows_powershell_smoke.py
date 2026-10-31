#!/usr/bin/env python3
"""Exercise a real classic Windows console and PowerShell, without ConPTY.

Windows-only; uses the already-built/installed executable and the existing pinned
sprite fixture (sources.json plus nine PNGs). No builds, downloads, providers or
accounts. Example from the repository root:

  python scripts/windows_powershell_smoke.py --binary target/release/ev.exe \
      --assets-dir target/installed-sprites --output target/powershell-smoke

Eight isolated journeys cover 60x18/120x36, default images/--images off, and two
launches: ordinary PowerShell `&` with ALL native handles inherited, plus a paired
Start-Process -NoNewWindow diagnostic with ONLY stderr redirected to a raw file.
The latter is deliberately not claimed as an all-inherited launch. Separating
these passes avoids concealing handle-dependent failures behind redirection.

Each worker is launched directly with CREATE_NEW_CONSOLE. Host evidence must show
a visible native ConsoleWindowClass; a pseudoconsole's hidden/message-only
GetConsoleWindow is rejected regardless of its owning process name. Pre-app
bootstrap failure stops the matrix immediately and retains a failed report.

results.json, per-case result.json, PowerShell status/error files, changed native
screen snapshots (including cell attributes), and diagnostic stderr.bin survive
failure. Ordinary inherited stderr is visible in console snapshots, not claimed
as a lossless byte stream; the paired pass provides complete separate stderr.
Every pass must reach saved UI, persist genuine console key input, explicitly
quit, exit zero, and restore native console modes/code pages/cursor and primary
buffer. Early exit (even zero) fails. Exit values retain unsigned/signed/hex forms
and known NTSTATUS names. UI timeout is not asserted to be an input EOF: actual
stderr/status evidence distinguishes it from initialization/panic/stack failures.

A fresh output directory is required. All homes, caches, stores and projects are
inside it and retained as evidence. The user's console and data are never used.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import traceback

from terminal_smoke import WindowsJob
from windows_conpty import BOOL, COORD, DWORD, HANDLE, WORD


class Rectangle(ctypes.Structure):
    _fields_ = [(name, ctypes.c_int16) for name in ('left', 'top', 'right', 'bottom')]


class BufferInfo(ctypes.Structure):
    _fields_ = [('size', COORD), ('cursor', COORD), ('attributes', WORD),
                ('window', Rectangle), ('maximum', COORD)]


class CursorInfo(ctypes.Structure):
    _fields_ = [('size', DWORD), ('visible', BOOL)]


class Cell(ctypes.Structure):
    _fields_ = [('character', WORD), ('attributes', WORD)]


class KeyEvent(ctypes.Structure):
    _fields_ = [('down', BOOL), ('repeat', WORD), ('virtual_key', WORD),
                ('scan', WORD), ('character', WORD), ('control', DWORD)]


class EventData(ctypes.Union):
    _fields_ = [('key', KeyEvent), ('padding', ctypes.c_ubyte * 16)]


class InputRecord(ctypes.Structure):
    _fields_ = [('kind', WORD), ('event', EventData)]


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=True) + '\n', encoding='utf-8')


def check(success):
    if not success:
        raise ctypes.WinError(ctypes.get_last_error())


def kernel32():
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    pointer = ctypes.c_void_p
    signatures = {
        'CreateFileW': (HANDLE, [ctypes.c_wchar_p, DWORD, DWORD, pointer, DWORD, DWORD, HANDLE]),
        'CloseHandle': (BOOL, [HANDLE]),
        'SetStdHandle': (BOOL, [DWORD, HANDLE]),
        'SetHandleInformation': (BOOL, [HANDLE, DWORD, DWORD]),
        'GetConsoleMode': (BOOL, [HANDLE, ctypes.POINTER(DWORD)]),
        'GetConsoleScreenBufferInfo': (BOOL, [HANDLE, ctypes.POINTER(BufferInfo)]),
        'GetConsoleCursorInfo': (BOOL, [HANDLE, ctypes.POINTER(CursorInfo)]),
        'SetConsoleScreenBufferSize': (BOOL, [HANDLE, COORD]),
        'SetConsoleWindowInfo': (BOOL, [HANDLE, BOOL, ctypes.POINTER(Rectangle)]),
        'ReadConsoleOutputW': (BOOL, [HANDLE, ctypes.POINTER(Cell), COORD, COORD,
                                      ctypes.POINTER(Rectangle)]),
        'WriteConsoleOutputCharacterW': (BOOL, [HANDLE, ctypes.c_wchar_p, DWORD, COORD,
                                               ctypes.POINTER(DWORD)]),
        'WriteConsoleInputW': (BOOL, [HANDLE, ctypes.POINTER(InputRecord), DWORD,
                                     ctypes.POINTER(DWORD)]),
        'FlushConsoleInputBuffer': (BOOL, [HANDLE]),
        'GetConsoleCP': (DWORD, []),
        'GetConsoleOutputCP': (DWORD, []),
        'GetConsoleWindow': (HANDLE, []),
        'OpenProcess': (HANDLE, [DWORD, BOOL, DWORD]),
        'QueryFullProcessImageNameW': (BOOL, [HANDLE, DWORD, ctypes.c_wchar_p,
                                             ctypes.POINTER(DWORD)]),
    }
    for name, (result, arguments) in signatures.items():
        function = getattr(kernel, name)
        function.restype, function.argtypes = result, arguments
    return kernel


class Console:
    """The worker's private native console; never attaches to the user's console."""

    def __init__(self):
        self.kernel = kernel32()
        self.input = self.open('CONIN$')
        self.output = self.open('CONOUT$')
        for handle in (self.input, self.output):
            check(self.kernel.SetHandleInformation(handle, 1, 1))
        for kind, handle in ((-10, self.input), (-11, self.output), (-12, self.output)):
            check(self.kernel.SetStdHandle(kind & 0xFFFFFFFF, handle))

    def open(self, name):
        handle = self.kernel.CreateFileW(name, 0xC0000000, 3, None, 3, 0, None)
        if handle == ctypes.c_void_p(-1).value:
            raise ctypes.WinError(ctypes.get_last_error())
        return handle

    def close(self):
        self.kernel.CloseHandle(self.input)
        self.kernel.CloseHandle(self.output)

    def host(self):
        window = self.kernel.GetConsoleWindow()
        if not window:
            raise RuntimeError('worker has no native console window')
        user = ctypes.WinDLL('user32', use_last_error=True)
        user.GetWindowThreadProcessId.argtypes = [HANDLE, ctypes.POINTER(DWORD)]
        user.GetWindowThreadProcessId.restype = DWORD
        user.GetClassNameW.argtypes = [HANDLE, ctypes.c_wchar_p, ctypes.c_int32]
        user.GetClassNameW.restype = ctypes.c_int32
        user.IsWindowVisible.argtypes = [HANDLE]
        user.IsWindowVisible.restype = BOOL
        window_class = ctypes.create_unicode_buffer(256)
        check(user.GetClassNameW(window, window_class, len(window_class)))
        pid = DWORD()
        check(user.GetWindowThreadProcessId(window, ctypes.byref(pid)))
        metadata = {'pid': pid.value, 'window': int(window),
                    'window_class': window_class.value,
                    'visible': bool(user.IsWindowVisible(window))}
        process = self.kernel.OpenProcess(0x1000, False, pid)
        check(process)
        try:
            name, length = ctypes.create_unicode_buffer(32768), DWORD(32768)
            check(self.kernel.QueryFullProcessImageNameW(process, 0, name, ctypes.byref(length)))
            metadata['image'] = name.value
            return metadata
        finally:
            self.kernel.CloseHandle(process)

    def resize(self, width, height):
        # Normal PowerShell commonly has scrollback much taller than its visible
        # window. Keep that real inherited-console geometry in the normal cases.
        buffer_rows = 3000 if (width, height) == (120, 36) else height
        small = Rectangle(0, 0, 0, 0)
        check(self.kernel.SetConsoleWindowInfo(self.output, True, ctypes.byref(small)))
        check(self.kernel.SetConsoleScreenBufferSize(self.output, COORD(width, buffer_rows)))
        rectangle = Rectangle(0, 0, width - 1, height - 1)
        check(self.kernel.SetConsoleWindowInfo(self.output, True, ctypes.byref(rectangle)))

    def state(self):
        # CONOUT$ is reopened to observe the ACTIVE buffer, not a retained handle
        # to the original buffer after the application switches to an alternate.
        output = self.open('CONOUT$')
        try:
            modes = []
            for handle in (self.input, output):
                value = DWORD()
                check(self.kernel.GetConsoleMode(handle, ctypes.byref(value)))
                modes.append(value.value)
            cursor = CursorInfo()
            check(self.kernel.GetConsoleCursorInfo(output, ctypes.byref(cursor)))
            return {'modes': modes,
                    'code_pages': [self.kernel.GetConsoleCP(), self.kernel.GetConsoleOutputCP()],
                    'cursor': {'size': cursor.size, 'visible': bool(cursor.visible)}}
        finally:
            self.kernel.CloseHandle(output)

    def screen(self, full=False):
        output = self.open('CONOUT$')
        try:
            info = BufferInfo()
            check(self.kernel.GetConsoleScreenBufferInfo(output, ctypes.byref(info)))
            region = (Rectangle(0, 0, info.size.X - 1, info.size.Y - 1) if full
                      else info.window)
            width = region.right - region.left + 1
            height = region.bottom - region.top + 1
            if not 0 < width * height <= 524288:
                raise RuntimeError('unexpected console buffer dimensions')
            cells = []
            # Small native transfers also work on legacy console implementations.
            chunk_rows = max(1, 8192 // width)
            for row in range(0, height, chunk_rows):
                rows = min(chunk_rows, height - row)
                chunk = (Cell * (width * rows))()
                rectangle = Rectangle(region.left, region.top + row, region.right,
                                      region.top + row + rows - 1)
                check(self.kernel.ReadConsoleOutputW(output, chunk, COORD(width, rows),
                                                    COORD(0, 0), ctypes.byref(rectangle)))
                cells.extend([cell.character, cell.attributes] for cell in chunk)
            lines = [''.join(chr(cells[y * width + x][0]) for x in range(width))
                     for y in range(height)]
            return {'size': [width, height], 'buffer_size': [info.size.X, info.size.Y],
                    'window': [info.window.left, info.window.top, info.window.right, info.window.bottom],
                    'text': '\n'.join(lines), 'cursor': [info.cursor.X, info.cursor.Y],
                    'cells': cells}
        finally:
            self.kernel.CloseHandle(output)

    def marker(self):
        value = 'EEVEE_PRIMARY_SENTINEL'
        count = DWORD()
        check(self.kernel.WriteConsoleOutputCharacterW(self.output, value, len(value),
                                                       COORD(0, 0), ctypes.byref(count)))
        if count.value != len(value):
            raise RuntimeError('short primary-buffer sentinel write')
        return value

    def key(self, character='', virtual_key=0, control=0):
        records = (InputRecord * 2)()
        for index, down in enumerate((True, False)):
            records[index].kind = 1  # KEY_EVENT
            records[index].event.key = KeyEvent(down, 1, virtual_key, 0,
                                                ord(character) if character else 0, control)
        count = DWORD()
        check(self.kernel.WriteConsoleInputW(self.input, records, 2, ctypes.byref(count)))
        if count.value != 2:
            raise RuntimeError('short native console input write')

    def type(self, text):
        for character in text:
            self.key(character, 0x0D if character == '\r' else ord(character.upper()))
            time.sleep(0.025)


def ps_quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def status_details(value):
    unsigned = int(value) & 0xFFFFFFFF
    names = {0xC00000FD: 'STATUS_STACK_OVERFLOW', 0xC0000005: 'STATUS_ACCESS_VIOLATION',
             0xC0000409: 'STATUS_STACK_BUFFER_OVERRUN / fail-fast',
             0x40000015: 'STATUS_FATAL_APP_EXIT', 0xC000013A: 'STATUS_CONTROL_C_EXIT',
             0xC000001D: 'STATUS_ILLEGAL_INSTRUCTION', 0xC0000135: 'STATUS_DLL_NOT_FOUND'}
    return {'unsigned': unsigned, 'signed': ctypes.c_int32(unsigned).value,
            'hex': f'0x{unsigned:08X}', 'ntstatus': names.get(unsigned)}


def powershell_script(case, binary, images, launch):
    arguments = [] if images == 'default' else ['--images', 'off']
    argv = ', '.join(ps_quote(value) for value in arguments)
    if launch == 'inherited':
        invoke = '& ' + ps_quote(binary) + ''.join(' ' + ps_quote(value) for value in arguments)
        invoke += '\n$nativeExit = $LASTEXITCODE'
    else:
        argument_option = ' -ArgumentList @(' + argv + ')' if arguments else ''
        invoke = ('$native = Start-Process -FilePath ' + ps_quote(binary) + argument_option
                  + ' -NoNewWindow -PassThru -RedirectStandardError '
                  + ps_quote(case / 'stderr.bin')
                  + '\n$null = $native.Handle\n$native.WaitForExit()\n$native.Refresh()\n$nativeExit = $native.ExitCode')
    # Keep the shell alive after native exit so the worker can inspect restored
    # console state before PowerShell or conhost itself goes away.
    return '\n'.join([
        "$ErrorActionPreference = 'Stop'", '$nativeExit = $null', 'try {',
        '[IO.File]::WriteAllText(' + ps_quote(case / 'shell-ready') + ", 'ready')",
        '$stdio = @{ input_redirected = [Console]::IsInputRedirected; output_redirected = [Console]::IsOutputRedirected; error_redirected = [Console]::IsErrorRedirected }',
        '[IO.File]::WriteAllText(' + ps_quote(case / 'shell-stdio.json') + ', ($stdio | ConvertTo-Json -Compress))',
        'while (-not (Test-Path -LiteralPath ' + ps_quote(case / 'launch')
        + ')) { Start-Sleep -Milliseconds 20 }', invoke,
        '$status = @{ exit_code = $nativeExit; shell_pid = $PID }',
        '} catch {',
        '[IO.File]::WriteAllText(' + ps_quote(case / 'powershell-error.txt')
        + ', ($_ | Out-String))',
        '$status = @{ exit_code = $nativeExit; shell_error = ($_ | Out-String) }',
        '}', '$json = $status | ConvertTo-Json -Compress',
        '[IO.File]::WriteAllText(' + ps_quote(case / 'status.pending') + ', $json)',
        'Move-Item -LiteralPath ' + ps_quote(case / 'status.pending') + ' -Destination '
        + ps_quote(case / 'status.json'),
        'while (-not (Test-Path -LiteralPath ' + ps_quote(case / 'finish')
        + ')) { Start-Sleep -Milliseconds 20 }',
        'if ($null -eq $nativeExit) { exit 1 }', 'exit $nativeExit', '',
    ])


def isolated_environment(case):
    environment = os.environ.copy()
    # Deliberately leave ordinary Windows terminal capabilities to conhost;
    # inherited CI/Unix terminal emulation hints would change the default path.
    for key in list(environment):
        upper = key.upper()
        if (upper.startswith(('EV_', 'EEVEE_', 'OPENAI_', 'ANTHROPIC_', 'GEMINI_', 'CODEX_',
                              'CLAUDE_', 'XDG_'))
                or upper in ('CI', 'TERM', 'COLORTERM', 'TERM_PROGRAM', 'WT_SESSION',
                             'WT_PROFILE_ID', 'NO_COLOR', 'FORCE_COLOR', 'LC_ALL',
                             'LC_CTYPE', 'LANG', 'HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA',
                             'HOMEDRIVE', 'HOMEPATH')):
            environment.pop(key)
    home = case / 'home'
    for key, relative in (('HOME', 'home'), ('USERPROFILE', 'home'),
                          ('LOCALAPPDATA', 'home/AppData/Local'),
                          ('APPDATA', 'home/AppData/Roaming'),
                          ('XDG_DATA_HOME', 'home/data'), ('XDG_CACHE_HOME', 'home/cache'),
                          ('XDG_CONFIG_HOME', 'home/config'), ('TEMP', 'home/tmp'), ('TMP', 'home/tmp')):
        path = case / relative
        path.mkdir(parents=True, exist_ok=True)
        environment[key] = str(path)
    environment['HOMEDRIVE'], environment['HOMEPATH'] = os.path.splitdrive(str(home))
    return environment


def fixture(source):
    artwork = (Path(__file__).resolve().parents[1] / 'src/ui/artwork.rs').read_text(encoding='utf-8')
    revision = re.search(r'const REVISION: &str = "([0-9a-f]{40})"', artwork).group(1)
    species = [int(value) for value in re.search(r'pub const SPECIES:.*?= \[(.*?)\];', artwork).group(1).split(',') if value.strip()]
    hashes = re.findall(r'"([0-9a-f]{64})"', artwork.split('const HASHES:', 1)[1].split('];', 1)[0])
    manifest = json.loads((source / 'sources.json').read_text(encoding='utf-8'))
    if manifest['revision'] != revision or len(manifest['images']) != len(species):
        raise ValueError('sprite fixture does not match pinned revision/species')
    records = {record['id']: record for record in manifest['images']}
    for identity, digest in zip(species, hashes):
        expected_url = f'https://raw.githubusercontent.com/PokeAPI/sprites/{revision}/sprites/pokemon/{identity}.png'
        if (hashlib.sha256((source / f'{identity}.png').read_bytes()).hexdigest() != digest
                or records[identity]['sha256'] != digest or records[identity]['url'] != expected_url):
            raise ValueError(f'pinned sprite verification failed: {identity}')
    return {'revision': revision, 'files': ['sources.json', *(f'{identity}.png' for identity in species)]}


def worker(case, binary, shell, width, height, images, launch, timeout):
    result = {'status': 'failed', 'phase': 'worker-start', 'images': images, 'launch': launch,
              'size': [width, height], 'safe_quit_sent': False, 'snapshots': 0}
    console = process = None
    started = time.monotonic()
    try:
        deadline = time.monotonic() + 15
        while not (case / 'owned').exists():
            if time.monotonic() > deadline:
                raise TimeoutError('parent did not establish process-tree ownership')
            time.sleep(0.02)
        result['phase'] = 'console-bootstrap'
        console = Console()
        result['console_host'] = console.host()
        # GetConsoleWindow can return a client-owned, message-only window in a
        # pseudoconsole. An executable basename alone is not host verification.
        # A visible native ConsoleWindowClass is required; Terminal/ConPTY's
        # hidden compatibility window is not accepted as classic-console proof.
        if (result['console_host']['window_class'] != 'ConsoleWindowClass'
                or not result['console_host']['visible']):
            raise RuntimeError('worker does not own a visible classic console window')
        console.resize(width, height)
        result['phase'] = 'powershell-start'
        # Execute the retained script text as a command: no policy bypass or
        # global execution-policy changes are needed for generated local code.
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESTDHANDLES
        startup.hStdInput = console.input
        startup.hStdOutput = startup.hStdError = console.output
        process = subprocess.Popen([shell, '-NoLogo', '-NoProfile', '-Command',
                                    (case / 'launch.ps1').read_text(encoding='utf-8-sig')],
                                   cwd=case / 'project', startupinfo=startup, close_fds=False)
        previous = None
        transcript = (case / 'screens.jsonl').open('w', encoding='utf-8')

        def sample():
            nonlocal previous
            screen = console.screen()
            if screen['cells'] != previous:
                previous = screen['cells']
                screen['elapsed_seconds'] = round(time.monotonic() - started, 3)
                transcript.write(json.dumps(screen, ensure_ascii=True) + '\n')
                transcript.flush()
                result['snapshots'] += 1
            return screen['text']

        def until(predicate, phase, allow_exit=False):
            result['phase'] = phase
            deadline = time.monotonic() + timeout
            while True:
                text = sample()
                if not allow_exit and ((case / 'status.json').exists() or process.poll() is not None):
                    raise RuntimeError('application/shell exited before ' + phase)
                if predicate(text):
                    return text
                if time.monotonic() >= deadline:
                    raise TimeoutError('waiting for ' + phase)
                time.sleep(0.04)

        try:
            until(lambda _: (case / 'shell-ready').exists() and (case / 'shell-stdio.json').exists(), 'shell-ready')
            result['shell_stdio'] = json.loads((case / 'shell-stdio.json').read_text(encoding='utf-8-sig'))
            if any(result['shell_stdio'].values()):
                raise RuntimeError('private PowerShell inherited redirected standard handles')
            check(console.kernel.FlushConsoleInputBuffer(console.input))
            result['before'] = console.state()
            primary = console.marker()
            (case / 'launch').touch()
            result['application_launch_released'] = True
            (case / 'application-launched').touch()
            until(lambda text: 'Storage: saved' in text, 'first-saved-ui')
            result['first_usable_ui_seconds'] = round(time.monotonic() - started, 3)
            result['during'] = console.state()
            result['alternate_buffer_observed'] = not sample().startswith(primary)
            if not result['alternate_buffer_observed']:
                raise RuntimeError('primary console buffer was not hidden during UI')
            if images == 'default':
                console.type('/images\r')
                until(lambda text: re.search(r'\b9/9\b', text) is not None, 'nine-pinned-sprites-ui')
                console.key('\x1b', 0x1B)
                time.sleep(0.15)
            console.type('/goal\r')
            goal = 'classic console native input'
            console.type(goal + '\r')
            database = case / 'home/AppData/Local/eevee/workbench.sqlite3'

            def persisted(_):
                if not database.exists():
                    return False
                try:
                    with sqlite3.connect('file:' + database.as_posix() + '?mode=ro', uri=True) as db:
                        return db.execute('SELECT goal FROM tasks').fetchall() == [(goal,)]
                except sqlite3.OperationalError:
                    return False

            until(persisted, 'native-key-input-persisted')
            until(lambda text: 'Storage: saved' in text, 'saved-before-quit')
            result['native_input_persisted'] = True
            result['safe_quit_sent'] = True
            console.type('/quit\r')
            until(lambda text: (case / 'status.json').exists() or 'Confirm exit' in text,
                  'explicit-quit-or-review', allow_exit=True)
            if not (case / 'status.json').exists():
                console.type('y')
                result['safe_quit_confirmed'] = True
            until(lambda _: (case / 'status.json').exists(), 'explicit-safe-quit', allow_exit=True)
            result['after'] = console.state()
            result['primary_buffer_restored'] = sample().startswith(primary)
            if result['before'] != result['after']:
                raise RuntimeError('console modes/code pages/cursor were not restored')
            if not result['primary_buffer_restored']:
                raise RuntimeError('primary console buffer sentinel was not restored')
            status = json.loads((case / 'status.json').read_text(encoding='utf-8-sig'))
            if status.get('shell_error') or status.get('exit_code') is None:
                raise RuntimeError('PowerShell failed to obtain native exit status')
            result['native_exit'] = status_details(status['exit_code'])
            if result['native_exit']['unsigned'] != 0:
                raise RuntimeError('application exited nonzero after explicit quit')
            result['phase'], result['status'] = 'complete', 'passed'
        finally:
            sample()
            transcript.close()
    except BaseException as error:
        result['error'] = f'{type(error).__name__}: {error}'
        (case / 'worker-error.txt').write_text(traceback.format_exc(), encoding='utf-8')
    finally:
        # Evidence collection is secondary to publishing the primary result.
        # In particular an unresized bootstrap buffer can exceed screen()'s
        # bounds; that must not erase the original failure and hang the parent.
        cleanup_errors = []
        try:
            if (case / 'status.json').exists():
                try:
                    result['powershell_status'] = json.loads((case / 'status.json').read_text(encoding='utf-8-sig'))
                    code = result['powershell_status'].get('exit_code')
                    if code is not None:
                        result['native_exit'] = status_details(code)
                except BaseException as error:
                    cleanup_errors.append('status: ' + repr(error))
            try:
                (case / 'finish').touch()
            except BaseException as error:
                cleanup_errors.append('finish: ' + repr(error))
            if process is not None:
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    result['shell_cleanup'] = 'still running; parent ownership job will terminate tree'
                except BaseException as error:
                    cleanup_errors.append('shell wait: ' + repr(error))
                if process.returncode is not None:
                    result['powershell_exit'] = status_details(process.returncode)
            if console is not None:
                try:
                    save(case / 'final-buffer.json', console.screen(full=True))
                except BaseException as error:
                    result['final_screen_error'] = repr(error)
                    try:
                        save(case / 'final-viewport.json', console.screen())
                    except BaseException as viewport_error:
                        cleanup_errors.append('final viewport: ' + repr(viewport_error))
                try:
                    result['final_console_state'] = console.state()
                except BaseException as error:
                    cleanup_errors.append('final state: ' + repr(error))
        finally:
            try:
                if console is not None:
                    console.close()
            except BaseException as error:
                cleanup_errors.append('close console: ' + repr(error))
            finally:
                if cleanup_errors:
                    result['cleanup_errors'] = cleanup_errors
                    result['status'] = 'failed'
                result['duration_seconds'] = round(time.monotonic() - started, 3)
                pending = case / 'result.pending'
                save(pending, result)
                pending.replace(case / 'result.json')
    return 0 if result['status'] == 'passed' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--assets-dir', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--powershell', default='powershell.exe')
    parser.add_argument('--timeout', type=float, default=20, help='per-phase timeout in seconds')
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--width', type=int, default=120, help=argparse.SUPPRESS)
    parser.add_argument('--height', type=int, default=36, help=argparse.SUPPRESS)
    parser.add_argument('--images', choices=('default', 'off'), default='default', help=argparse.SUPPRESS)
    parser.add_argument('--launch', choices=('inherited', 'stderr-file'), default='inherited', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if os.name != 'nt':
        parser.error('requires native Windows; no ConPTY or POSIX substitute is accepted')
    if args.timeout <= 0:
        parser.error('--timeout must be positive')
    binary, output = args.binary.resolve(), args.output.resolve()
    if args.worker:
        return worker(output, binary, args.powershell, args.width, args.height,
                      args.images, args.launch, args.timeout)
    if not binary.is_file() or not args.assets_dir:
        parser.error('an existing --binary and pinned --assets-dir are required')
    assets = args.assets_dir.resolve()
    pinned = fixture(assets)
    output.mkdir(parents=True, exist_ok=False)
    shell = shutil.which(args.powershell)
    report = {'status': 'failed', 'binary': str(binary),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'host': platform.platform(), 'python': sys.version, 'sprites': pinned,
              'powershell': shell, 'console_backend': 'private CREATE_NEW_CONSOLE; visible ConsoleWindowClass required',
              'cases': []}
    try:
        if shell is None:
            raise RuntimeError('native Windows PowerShell is unavailable')
        for width, height in ((60, 18), (120, 36)):
            for images in ('default', 'off'):
                for launch in ('inherited', 'stderr-file'):
                    name = f'{width}x{height}-{images}-{launch}'
                    case = output / name
                    case.mkdir()
                    (case / 'project').mkdir()
                    environment = isolated_environment(case)
                    cache = case / 'home/AppData/Local/eevee/sprites'
                    cache.mkdir(parents=True)
                    for name_in_fixture in pinned['files']:
                        shutil.copy2(assets / name_in_fixture, cache / name_in_fixture)
                    (case / 'launch.ps1').write_text(powershell_script(case, binary, images, launch),
                                                    encoding='utf-8-sig')
                    argv = [sys.executable, str(Path(__file__).resolve()), '--worker',
                            '--binary', str(binary), '--output', str(case), '--powershell', shell,
                            '--width', str(width), '--height', str(height), '--images', images,
                            '--launch', launch, '--timeout', str(args.timeout)]
                    process = job = None
                    result = {'status': 'failed', 'name': name}
                    try:
                        process = subprocess.Popen(argv, env=environment,
                                                   creationflags=subprocess.CREATE_NEW_CONSOLE)
                        job = WindowsJob(process.pid)
                        (case / 'owned').touch()
                        # This is the actual worker, not a launcher whose lifetime
                        # may end before its descendant. A dead worker is immediate
                        # failure; do not wait the entire journey budget for JSON.
                        bootstrap_deadline = time.monotonic() + args.timeout + 25
                        deadline = time.monotonic() + args.timeout * 8 + 30
                        while not (case / 'result.json').exists():
                            worker_exit = process.poll()
                            if worker_exit is not None:
                                if (case / 'result.json').exists():
                                    break  # Publication won the exit/poll race.
                                result['worker_exit'] = status_details(worker_exit)
                                raise RuntimeError('classic console worker exited without publishing evidence')
                            if (not (case / 'application-launched').exists()
                                    and time.monotonic() >= bootstrap_deadline):
                                raise TimeoutError('classic console bootstrap did not release application launch')
                            if time.monotonic() >= deadline:
                                raise TimeoutError('classic console worker did not produce evidence')
                            time.sleep(0.1)
                        result.update(json.loads((case / 'result.json').read_text(encoding='utf-8')))
                        if fixture(cache) != pinned:
                            raise RuntimeError('isolated pinned sprite cache changed during launch')
                    except BaseException as error:
                        result['status'], result['driver_error'] = 'failed', f'{type(error).__name__}: {error}'
                    finally:
                        if job is not None:
                            job.close()
                        elif process is not None and process.poll() is None:
                            process.kill()
                        if process is not None:
                            process.wait(timeout=10)
                        stderr = case / 'stderr.bin'
                        if stderr.exists():
                            result['stderr_file'] = str(stderr)
                            result['stderr_bytes'] = stderr.stat().st_size
                            result['stderr_sha256'] = hashlib.sha256(stderr.read_bytes()).hexdigest()
                        elif launch == 'stderr-file':
                            result['status'] = 'failed'
                            result['stderr_error'] = 'complete diagnostic stderr file was not produced'
                        report['cases'].append(result)
                        save(output / 'results.json', report)
                    if (result['status'] != 'passed'
                            and not (case / 'application-launched').exists()):
                        raise RuntimeError('shared pre-application console bootstrap failed; remaining journeys not launched')
        report['binary_sha256_after'] = hashlib.sha256(binary.read_bytes()).hexdigest()
        if (all(case['status'] == 'passed' for case in report['cases'])
                and len(report['cases']) == 8
                and report['binary_sha256'] == report['binary_sha256_after']):
            report['status'] = 'passed'
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        save(output / 'results.json', report)
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
