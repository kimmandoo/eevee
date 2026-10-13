"""Real Windows ConPTY byte-transport regressions (no terminal emulator required)."""
import codecs
import hashlib
import json
import os
import queue
import re
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

if sys.platform == 'win32':
    from windows_conpty import ConPTYProcess


UNICODE_TEXT = '┌─┬┐│└┴┘ 汉字日本語한글 🦀😀𐐷 e\u0301o\u0308'
ARGUMENT = 'argument with spaces, "quotes", and 漢字'
ENVIRONMENT_VALUE = 'environment with spaces = 漢字'
TITLE = re.compile(r'\x1b\](?:0|2);(.*?)(?:\x07|\x1b\\)', re.DOTALL)
TIMEOUT = 15.0

# Titles preserve combining characters and astral code points without depending
# on ConPTY's screen-cell redraws, cursor moves, wrapping, or newline conversion.
# Each title has a unique prefix, and the child waits for input between titles
# so ConPTY cannot coalesce two title updates before the parent observes them.
# Windows documents OSC 0/2 and split WriteFile sequences:
# https://learn.microsoft.com/en-us/windows/console/console-virtual-terminal-sequences
CHILD = f'PAYLOAD = {UNICODE_TEXT!r}\n' + r'''
import ctypes
from ctypes import wintypes
import hashlib
import json
import msvcrt
import os
import sys
import time

kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
kernel32.GetStdHandle.argtypes = [wintypes.DWORD]
kernel32.GetStdHandle.restype = wintypes.HANDLE
kernel32.GetConsoleMode.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
kernel32.GetConsoleMode.restype = wintypes.BOOL
kernel32.SetConsoleMode.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel32.SetConsoleMode.restype = wintypes.BOOL
kernel32.SetConsoleOutputCP.argtypes = [wintypes.UINT]
kernel32.SetConsoleOutputCP.restype = wintypes.BOOL

def checked(result):
    if not result:
        raise ctypes.WinError(ctypes.get_last_error())

output = kernel32.GetStdHandle(-11)
mode = wintypes.DWORD()
checked(kernel32.GetConsoleMode(output, ctypes.byref(mode)))
checked(kernel32.SetConsoleMode(output, mode.value | 0x0004))
checked(kernel32.SetConsoleOutputCP(65001))
input_handle = kernel32.GetStdHandle(-10)
checked(kernel32.GetConsoleMode(input_handle, ctypes.byref(mode)))
checked(kernel32.SetConsoleMode(input_handle, mode.value & ~0x0004))
msvcrt.setmode(sys.stdout.fileno(), os.O_BINARY)

def title(value, split=False):
    encoded = ('\x1b]0;' + value + '\x07').encode('utf-8')
    if split:
        for byte in encoded:
            os.write(sys.stdout.fileno(), bytes((byte,)))
            time.sleep(0.003)
    else:
        os.write(sys.stdout.fileno(), encoded)

def size():
    dimensions = os.get_terminal_size(sys.stdout.fileno())
    return [dimensions.lines, dimensions.columns]

def command(expected):
    received = sys.stdin.readline().strip()
    if received != expected:
        raise RuntimeError(f'expected {expected!r}, received {received!r}')

# Keep this title below the documented 255-character limit even when the
# runner's temporary directory is long; verify its full path by digest.
title('EEVEE_READY:' + json.dumps({
    'cwd_sha256': hashlib.sha256(os.path.normcase(os.getcwd()).encode('utf-8')).hexdigest(),
    'environment': os.environ['EEVEE_CONPTY_TEST'],
    'argument': sys.argv[1],
    'size': size(),
}, ensure_ascii=False, separators=(',', ':')))
command('emit')
title('EEVEE_UNICODE:' + PAYLOAD, split=True)
command('size')
title('EEVEE_SIZE:' + json.dumps(size()))
command('exit')
title('EEVEE_DONE:23')
sys.exit(23)
'''


@unittest.skipUnless(sys.platform == 'win32', 'requires native Windows ConPTY')
class ConPTYTests(unittest.TestCase):
    def spawn_child(self, directory):
        process = ConPTYProcess.spawn(
            [sys.executable, '-u', '-c', CHILD, ARGUMENT],
            cwd=str(directory),
            env={**os.environ, 'EEVEE_CONPTY_TEST': ENVIRONMENT_VALUE},
            dimensions=(20, 160),
        )
        chunks = queue.Queue()

        def drain():
            try:
                while True:
                    chunk = process.read(1)
                    if not isinstance(chunk, bytes):
                        raise TypeError(f'read(1) returned {type(chunk).__name__}, not bytes')
                    if len(chunk) > 1:
                        raise ValueError(f'read(1) returned {len(chunk)} bytes')
                    chunks.put(chunk)
                    if not chunk:
                        return
            except Exception as error:
                chunks.put(error)

        # Synchronous native reads must never prevent a deadline from expiring.
        # Continue draining during close: ClosePseudoConsole can flush output.
        reader = threading.Thread(target=drain, name='conpty-regression-reader', daemon=True)
        reader.start()

        def cleanup():
            try:
                process.close()
            finally:
                reader.join(timeout=TIMEOUT)
            self.assertFalse(reader.is_alive(), 'ConPTY read remained blocked after close')
            self.assertFalse(process.isalive(), 'ConPTY child remained alive after close')

        self.addCleanup(cleanup)
        return process, chunks

    def titles(self, chunks):
        decoder = codecs.getincrementaldecoder('utf-8')('strict')
        pending = ''
        while True:
            deadline = time.monotonic() + TIMEOUT
            while True:
                match = TITLE.search(pending)
                if match is not None:
                    pending = pending[match.end():]
                    yield match.group(1)
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self.fail(f'timed out waiting for ConPTY title; pending={pending!r}')
                try:
                    chunk = chunks.get(timeout=remaining)
                except queue.Empty:
                    self.fail(f'timed out waiting for ConPTY bytes; pending={pending!r}')
                if isinstance(chunk, Exception):
                    raise chunk
                if not chunk:
                    pending += decoder.decode(b'', final=True)
                    self.fail(f'ConPTY reached EOF before expected title; pending={pending!r}')
                pending += decoder.decode(chunk)

    def expect_title(self, titles, prefix):
        # Ignore inherited/startup titles; only the unique child-owned markers
        # participate in the assertion. Both OSC 0 and OSC 2 are valid titles.
        deadline = time.monotonic() + TIMEOUT
        for title in titles:
            if title.startswith(prefix):
                return title[len(prefix):]
            if time.monotonic() >= deadline:
                self.fail(f'timed out waiting for {prefix!r}; last title={title!r}')
        self.fail(f'ConPTY ended before title {prefix!r}')

    def test_split_utf8_and_process_contract(self):
        temporary = tempfile.TemporaryDirectory(prefix='eevee conpty ')
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name) / 'working directory 漢字'
        directory.mkdir()
        process, chunks = self.spawn_child(directory)
        titles = self.titles(chunks)
        ready = json.loads(self.expect_title(titles, 'EEVEE_READY:'))
        expected_cwd = hashlib.sha256(os.path.normcase(str(directory)).encode('utf-8')).hexdigest()
        self.assertEqual(ready['cwd_sha256'], expected_cwd)
        self.assertEqual(ready['environment'], ENVIRONMENT_VALUE)
        self.assertEqual(ready['argument'], ARGUMENT)
        self.assertEqual(ready['size'], [20, 160])
        self.assertTrue(process.isalive())
        self.assertIsNone(process.exitstatus)

        process.write('emit\r')
        self.assertEqual(self.expect_title(titles, 'EEVEE_UNICODE:'), UNICODE_TEXT)

        process.setwinsize(27, 132)
        process.write('size\r')
        self.assertEqual(json.loads(self.expect_title(titles, 'EEVEE_SIZE:')), [27, 132])

        process.write('exit\r')
        self.assertEqual(self.expect_title(titles, 'EEVEE_DONE:'), '23')
        deadline = time.monotonic() + TIMEOUT
        while process.isalive() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertFalse(process.isalive(), 'child did not exit before deadline')
        self.assertEqual(process.exitstatus, 23)
        process.close()
        self.assertEqual(process.exitstatus, 23)

    def test_close_terminates_waiting_child(self):
        temporary = tempfile.TemporaryDirectory(prefix='eevee conpty close ')
        self.addCleanup(temporary.cleanup)
        process, chunks = self.spawn_child(Path(temporary.name))
        self.expect_title(self.titles(chunks), 'EEVEE_READY:')
        self.assertTrue(process.isalive())
        process.close()
        self.assertFalse(process.isalive())
        self.assertIsNotNone(process.exitstatus)
        # Cleanup closes it again and also proves that its blocking reader exits.


if __name__ == '__main__':
    unittest.main()
