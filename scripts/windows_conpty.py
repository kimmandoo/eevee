"""Native Windows ConPTY transport; output stays bytes until the caller decodes it.

The caller owns one continuously draining reader thread. Other operations belong
on the controlling thread, including close(), which must run while that reader
is still draining. Process-tree ownership remains the caller's Windows job.

API and shutdown ordering follow Microsoft's pseudoconsole session guidance:
https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session
https://learn.microsoft.com/en-us/windows/console/closepseudoconsole
"""

import ctypes
import os
import subprocess
import threading


# Windows uses LLP64, including on 64-bit ARM. Keep integral widths explicit so
# importing this module on a non-Windows test host does not change the layouts.
DWORD = ctypes.c_uint32
BOOL = ctypes.c_int32
WORD = ctypes.c_uint16
HANDLE = ctypes.c_void_p
SIZE_T = ctypes.c_size_t

ERROR_INSUFFICIENT_BUFFER = 122
ERROR_BROKEN_PIPE = 109
ERROR_NO_DATA = 232
ERROR_PIPE_NOT_CONNECTED = 233
WAIT_OBJECT_0 = 0
WAIT_TIMEOUT = 258
EXTENDED_STARTUPINFO_PRESENT = 0x00080000
CREATE_UNICODE_ENVIRONMENT = 0x00000400
STARTF_USESTDHANDLES = 0x00000100
PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE = 0x00020016


class COORD(ctypes.Structure):
    _fields_ = [("X", ctypes.c_int16), ("Y", ctypes.c_int16)]


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [
        ("cb", DWORD),
        ("lpReserved", ctypes.c_wchar_p),
        ("lpDesktop", ctypes.c_wchar_p),
        ("lpTitle", ctypes.c_wchar_p),
        ("dwX", DWORD),
        ("dwY", DWORD),
        ("dwXSize", DWORD),
        ("dwYSize", DWORD),
        ("dwXCountChars", DWORD),
        ("dwYCountChars", DWORD),
        ("dwFillAttribute", DWORD),
        ("dwFlags", DWORD),
        ("wShowWindow", WORD),
        ("cbReserved2", WORD),
        ("lpReserved2", ctypes.POINTER(ctypes.c_ubyte)),
        ("hStdInput", HANDLE),
        ("hStdOutput", HANDLE),
        ("hStdError", HANDLE),
    ]


class STARTUPINFOEXW(ctypes.Structure):
    _fields_ = [("StartupInfo", STARTUPINFOW), ("lpAttributeList", ctypes.c_void_p)]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("hProcess", HANDLE), ("hThread", HANDLE),
        ("dwProcessId", DWORD), ("dwThreadId", DWORD),
    ]


def _kernel32():
    if os.name != "nt":
        raise OSError("ConPTY requires Windows 10 version 1809 or newer")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    pointer = ctypes.c_void_p
    signatures = {
        "CreatePipe": (BOOL, [ctypes.POINTER(HANDLE), ctypes.POINTER(HANDLE), pointer, DWORD]),
        "CreatePseudoConsole": (ctypes.c_int32, [COORD, HANDLE, HANDLE, DWORD, ctypes.POINTER(HANDLE)]),
        "ResizePseudoConsole": (ctypes.c_int32, [HANDLE, COORD]),
        "ClosePseudoConsole": (None, [HANDLE]),
        "InitializeProcThreadAttributeList": (BOOL, [pointer, DWORD, DWORD, ctypes.POINTER(SIZE_T)]),
        "UpdateProcThreadAttribute": (BOOL, [pointer, DWORD, SIZE_T, pointer, SIZE_T, pointer,
                                           ctypes.POINTER(SIZE_T)]),
        "DeleteProcThreadAttributeList": (None, [pointer]),
        "CreateProcessW": (BOOL, [ctypes.c_wchar_p, ctypes.c_wchar_p, pointer, pointer, BOOL,
                                  DWORD, pointer, ctypes.c_wchar_p,
                                  ctypes.POINTER(STARTUPINFOW), ctypes.POINTER(PROCESS_INFORMATION)]),
        "ReadFile": (BOOL, [HANDLE, pointer, DWORD, ctypes.POINTER(DWORD), pointer]),
        "WriteFile": (BOOL, [HANDLE, pointer, DWORD, ctypes.POINTER(DWORD), pointer]),
        "WaitForSingleObject": (DWORD, [HANDLE, DWORD]),
        "GetExitCodeProcess": (BOOL, [HANDLE, ctypes.POINTER(DWORD)]),
        "TerminateProcess": (BOOL, [HANDLE, DWORD]),
        "CloseHandle": (BOOL, [HANDLE]),
    }
    for name, (result, arguments) in signatures.items():
        function = getattr(kernel, name)
        function.restype = result
        function.argtypes = arguments
    return kernel


def _check(success):
    if not success:
        raise ctypes.WinError(ctypes.get_last_error())


def _check_hresult(result, operation):
    if result < 0:
        raise OSError(f"{operation} failed with HRESULT 0x{result & 0xFFFFFFFF:08X}")


def _coordinates(rows, cols):
    if not (1 <= rows <= 32767 and 1 <= cols <= 32767):
        raise ValueError("ConPTY dimensions must be between 1 and 32767")
    return COORD(cols, rows)


class ConPTYProcess:
    """A direct child and its raw-byte console pipes, without a private reader.

    spawn dimensions and setwinsize use (rows, columns). read blocks until bytes
    or EOF (b''); the caller must incrementally decode UTF-8 across reads. write
    encodes UTF-8 and returns the byte count after writing the entire string.
    exitstatus is None while running and the Windows exit code after exit.
    close is idempotent and terminates a surviving direct child. Once reading
    has begun, its reader must continue to EOF concurrently with close.
    On Windows 11 24H2 and later, output can finish draining after close returns;
    the reader releases the output handle at EOF, so join that thread last.
    """

    def __init__(self, kernel):
        self._kernel = kernel
        self._input = HANDLE()
        self._output = HANDLE()
        self._console = HANDLE()
        self._process = HANDLE()
        self._exitstatus = None
        self._closed = False
        self._reader_started = False
        self._state_lock = threading.Lock()
        self.pid = None

    @classmethod
    def spawn(cls, argv, cwd=None, env=None, dimensions=(24, 80)):
        size = _coordinates(*dimensions)
        command = ctypes.create_unicode_buffer(subprocess.list2cmdline(argv))
        environment = None
        if env is not None:
            entries = (f"{key}={value}" for key, value in
                       sorted(env.items(), key=lambda item: item[0].upper()))
            # create_unicode_buffer adds the second terminating NUL, including
            # for an explicitly empty environment.
            environment = ctypes.create_unicode_buffer("\0".join(entries) + "\0")
        kernel = _kernel32()
        process = cls(kernel)
        input_read, output_write = HANDLE(), HANDLE()
        information = PROCESS_INFORMATION()
        attributes = None
        attributes_initialized = False
        try:
            _check(kernel.CreatePipe(ctypes.byref(input_read), ctypes.byref(process._input), None, 0))
            _check(kernel.CreatePipe(ctypes.byref(process._output), ctypes.byref(output_write), None, 0))
            _check_hresult(kernel.CreatePseudoConsole(size, input_read, output_write, 0,
                                                      ctypes.byref(process._console)),
                           "CreatePseudoConsole")
            required = SIZE_T()
            success = kernel.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(required))
            if success or ctypes.get_last_error() != ERROR_INSUFFICIENT_BUFFER:
                raise ctypes.WinError(ctypes.get_last_error())
            attributes = ctypes.create_string_buffer(required.value)
            _check(kernel.InitializeProcThreadAttributeList(attributes, 1, 0, ctypes.byref(required)))
            attributes_initialized = True
            # PSEUDOCONSOLE takes the HPCON value itself, not a pointer to it.
            _check(kernel.UpdateProcThreadAttribute(attributes, 0,
                                                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                                                    process._console, ctypes.sizeof(HANDLE), None, None))
            startup = STARTUPINFOEXW()
            startup.StartupInfo.cb = ctypes.sizeof(STARTUPINFOEXW)
            # Null standard handles with this flag prevent inheriting the
            # parent's redirected CI pipes; ConPTY supplies its console handles.
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES
            startup.lpAttributeList = ctypes.cast(attributes, ctypes.c_void_p)
            _check(kernel.CreateProcessW(None, command, None, None, False,
                                          EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                                          environment, os.fspath(cwd) if cwd is not None else None,
                                          ctypes.byref(startup.StartupInfo), ctypes.byref(information)))
            process._process.value = information.hProcess
            process.pid = information.dwProcessId
            return process
        except BaseException:
            if information.hProcess:
                process._process.value = information.hProcess
            # No caller reader exists yet. close() closes its read pipe before
            # shutting down ConPTY so a final frame cannot block failed spawn.
            process.close()
            raise
        finally:
            if information.hThread:
                kernel.CloseHandle(information.hThread)
            if attributes_initialized:
                kernel.DeleteProcThreadAttributeList(attributes)
            for handle in (input_read, output_write):
                if handle.value:
                    kernel.CloseHandle(handle)

    def _release(self, handle):
        if handle.value:
            self._kernel.CloseHandle(handle)
            handle.value = None

    def _finish_read(self):
        with self._state_lock:
            self._release(self._output)

    def read(self, size):
        """Return up to size raw bytes; broken/disconnected pipes are EOF."""
        if not 1 <= size <= 0xFFFFFFFF:
            raise ValueError("read size must be between 1 and 4294967295")
        with self._state_lock:
            if not self._output.value:
                return b""
            self._reader_started = True
            handle = self._output.value
        try:
            buffer = ctypes.create_string_buffer(size)
            count = DWORD()
            if not self._kernel.ReadFile(handle, buffer, size, ctypes.byref(count), None):
                error = ctypes.get_last_error()
                if error not in (ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED):
                    raise ctypes.WinError(error)
                self._finish_read()
                return b""
            if not count.value:
                self._finish_read()
                return b""
            return buffer[:count.value]
        except BaseException:
            # A failed reader cannot drain shutdown output. Closing its pipe
            # allows ClosePseudoConsole to finish even on pre-24H2 Windows.
            self._finish_read()
            raise

    def write(self, text):
        """Write all UTF-8 bytes, including after a short native write."""
        if self._closed:
            raise OSError("ConPTY is closed")
        data = text.encode("utf-8")
        address = ctypes.cast(ctypes.c_char_p(data), ctypes.c_void_p).value
        sent = 0
        while sent < len(data):
            count = DWORD()
            _check(self._kernel.WriteFile(self._input, ctypes.c_void_p(address + sent),
                                         min(len(data) - sent, 0xFFFFFFFF), ctypes.byref(count), None))
            if not count.value:
                raise OSError("ConPTY WriteFile made no progress")
            sent += count.value
        return sent

    @property
    def exitstatus(self):
        if self._exitstatus is None and self._process.value:
            result = self._kernel.WaitForSingleObject(self._process, 0)
            if result == WAIT_TIMEOUT:
                return None
            if result != WAIT_OBJECT_0:
                raise ctypes.WinError(ctypes.get_last_error())
            status = DWORD()
            _check(self._kernel.GetExitCodeProcess(self._process, ctypes.byref(status)))
            self._exitstatus = status.value
        return self._exitstatus

    def isalive(self):
        return bool(self._process.value) and self.exitstatus is None

    def setwinsize(self, rows, cols):
        if self._closed:
            raise OSError("ConPTY is closed")
        _check_hresult(self._kernel.ResizePseudoConsole(self._console, _coordinates(rows, cols)),
                       "ResizePseudoConsole")

    def close(self):
        """Stop the child and release handles while the caller drains output."""
        with self._state_lock:
            if self._closed:
                return
            self._closed = True
            reader_started = self._reader_started
            if not reader_started:
                self._release(self._output)
        try:
            if self.isalive():
                if not self._kernel.TerminateProcess(self._process, 1):
                    error = ctypes.get_last_error()
                    # The process may have exited between the poll and kill.
                    if self.isalive():
                        raise ctypes.WinError(error)
        finally:
            self._release(self._input)
            try:
                if self._console.value:
                    self._kernel.ClosePseudoConsole(self._console)
                    self._console.value = None
                if self._process.value:
                    result = self._kernel.WaitForSingleObject(self._process, 5000)
                    if result == WAIT_TIMEOUT:
                        raise TimeoutError("ConPTY process did not terminate within 5 seconds")
                    if result != WAIT_OBJECT_0:
                        raise ctypes.WinError(ctypes.get_last_error())
                    self._exitstatus = self.exitstatus
            finally:
                # Never close a handle underneath a pending synchronous read.
                # Its sole reader releases it at EOF, also on asynchronous
                # ConPTY shutdown in Windows 11 24H2 and newer.
                if not reader_started:
                    self._release(self._output)
                self._release(self._process)
