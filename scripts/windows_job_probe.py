"""Owned-process accounting for the opt-in Windows workspace qualification probe.

Associates a completion port before starting the suspended product. Retained
handles preserve exited helpers' OS high-water/lifetime counters. Missing job
notifications/handles make coverage incomplete rather than a passing estimate.
"""
import ctypes as c
from ctypes import wintypes as w
import subprocess as sp
import threading
import time


class Accounting(c.Structure):
    _fields_ = [(name, c.c_longlong) for name in ("user", "kernel", "period_user", "period_kernel")] + [(name, w.DWORD) for name in ("faults", "total", "active", "terminated")]


class Association(c.Structure):
    _fields_ = [("key", c.c_void_p), ("port", w.HANDLE)]


class ThreadEntry(c.Structure):
    _fields_ = [(name, w.DWORD) for name in ("size", "usage", "tid", "pid")] + [("priority", w.LONG), ("delta", w.LONG), ("flags", w.DWORD)]


class IoCounters(c.Structure):
    _fields_ = [(name, c.c_ulonglong) for name in ("read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]


def command_metadata(kernel, handle):
    """Read an owned x64 process's PEB fields; fail closed on unavailable data.

    Diagnostic only: mirrors Microsoft's vscode-windows-process-tree reader.
    No injection/debugger, global process enumeration or product instrumentation.
    """
    if c.sizeof(c.c_void_p) != 8:
        raise RuntimeError("command capture requires the qualified x64 probe host")
    class Basic(c.Structure):
        _fields_ = [(name, c.c_void_p) for name in ("reserved", "peb", "r2", "r3", "pid", "parent")]
    class Unicode(c.Structure):
        _fields_ = [("length", w.WORD), ("maximum", w.WORD), ("buffer", c.c_void_p)]
    class Peb(c.Structure):
        _fields_ = [("reserved", c.c_byte * 32), ("parameters", c.c_void_p)]
    class Parameters(c.Structure):
        _fields_ = [("reserved", c.c_byte * 96), ("image", Unicode), ("command", Unicode)]
    query = c.WinDLL("ntdll").NtQueryInformationProcess
    query.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.ULONG, c.c_void_p]
    query.restype = c.c_long
    kernel.ReadProcessMemory.argtypes = [w.HANDLE, c.c_void_p, c.c_void_p, c.c_size_t, c.POINTER(c.c_size_t)]
    def read(address, value):
        count = c.c_size_t()
        if not address or not kernel.ReadProcessMemory(handle, address, c.byref(value), c.sizeof(value), c.byref(count)) or count.value != c.sizeof(value):
            raise RuntimeError("owned command parameters unavailable")
        return value
    info = Basic()
    if query(handle, 0, c.byref(info), c.sizeof(info), None) < 0:
        raise RuntimeError("owned process basic information unavailable")
    peb = read(info.peb, Peb())
    parameters = read(peb.parameters, Parameters())
    command = parameters.command
    if not command.length or command.length % 2 or command.length > command.maximum or command.length > 65534:
        raise RuntimeError("owned command line has invalid bounds")
    buffer = read(command.buffer, (c.c_byte * command.length)())
    text = bytes(buffer).decode("utf-16-le", errors="strict")
    shell = c.WinDLL("shell32")
    shell.CommandLineToArgvW.argtypes = [w.LPCWSTR, c.POINTER(c.c_int)]
    shell.CommandLineToArgvW.restype = c.POINTER(w.LPWSTR)
    kernel.LocalFree.argtypes = [c.c_void_p]
    count = c.c_int()
    parsed = shell.CommandLineToArgvW(text, c.byref(count))
    if not parsed:
        raise RuntimeError("owned command line could not be parsed")
    try:
        return {"parent_pid": info.parent, "argv": [parsed[index] for index in range(count.value)]}
    finally:
        kernel.LocalFree(parsed)


class OwnedTree:
    def __init__(self, capture_commands=False):
        self.capture_commands = capture_commands
        self.kernel = k = c.WinDLL("kernel32", use_last_error=True)
        k.CreateJobObjectW.argtypes, k.CreateJobObjectW.restype = [c.c_void_p, w.LPCWSTR], w.HANDLE
        k.CreateIoCompletionPort.argtypes, k.CreateIoCompletionPort.restype = [w.HANDLE, w.HANDLE, c.c_size_t, w.DWORD], w.HANDLE
        k.SetInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD]
        k.QueryInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.c_void_p]
        k.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
        k.GetQueuedCompletionStatus.argtypes = [w.HANDLE, c.POINTER(w.DWORD), c.POINTER(c.c_size_t), c.POINTER(c.c_void_p), w.DWORD]
        k.OpenProcess.argtypes, k.OpenProcess.restype = [w.DWORD, w.BOOL, w.DWORD], w.HANDLE
        k.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
        k.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
        k.GetProcessIoCounters.argtypes = [w.HANDLE, c.POINTER(IoCounters)]
        k.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
        k.TerminateJobObject.argtypes = [w.HANDLE, w.UINT]
        k.CloseHandle.argtypes = [w.HANDLE]
        k.CreateToolhelp32Snapshot.argtypes, k.CreateToolhelp32Snapshot.restype = [w.DWORD, w.DWORD], w.HANDLE
        k.Thread32First.argtypes = k.Thread32Next.argtypes = [w.HANDLE, c.POINTER(ThreadEntry)]
        k.OpenThread.argtypes, k.OpenThread.restype = [w.DWORD, w.BOOL, w.DWORD], w.HANDLE
        k.ResumeThread.argtypes, k.ResumeThread.restype = [w.HANDLE], w.DWORD
        self.job = k.CreateJobObjectW(None, None)
        self.port = k.CreateIoCompletionPort(w.HANDLE(-1), None, 0, 1)
        if not self.job or not self.port:
            raise c.WinError(c.get_last_error())
        association = Association(1, self.port)
        if not k.SetInformationJobObject(self.job, 7, c.byref(association), c.sizeof(association)):
            raise c.WinError(c.get_last_error())
        self.handles = {}
        self.images = {}
        self.commands = {}
        self.active = set()
        self.missing = []
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._observe, daemon=True)
        self.thread.start()

    def _observe(self):
        k = self.kernel
        while not self.stop.is_set():
            code, key, value = w.DWORD(), c.c_size_t(), c.c_void_p()
            if not k.GetQueuedCompletionStatus(self.port, c.byref(code), c.byref(key), c.byref(value), 100):
                continue
            if code.value in (7, 8):
                with self.lock:
                    self.active.discard(value.value)
            elif code.value == 6:
                pid = value.value
                handle = k.OpenProcess(0x410, False, pid)
                name = None
                command = None
                if handle:
                    image = c.create_unicode_buffer(32768)
                    length = w.DWORD(len(image))
                    if k.QueryFullProcessImageNameW(handle, 0, image, c.byref(length)):
                        name = image.value.rsplit("\\", 1)[-1]
                    if self.capture_commands and name and name.lower() == "git.exe":
                        try:
                            command = command_metadata(k, handle)
                        except Exception as error:
                            command = {"error": str(error)}
                with self.lock:
                    if handle:
                        if pid in self.handles:
                            k.CloseHandle(handle)
                        else:
                            self.handles[pid] = handle
                            self.active.add(pid)
                            if name:
                                self.images[pid] = name
                            if command is not None:
                                self.commands[pid] = command
                    else:
                        self.missing.append(pid)

    def spawn(self, command, **kwargs):
        child = sp.Popen(command, creationflags=sp.CREATE_NO_WINDOW | 4, **kwargs)  # CREATE_SUSPENDED
        try:
            if not self.kernel.AssignProcessToJobObject(self.job, w.HANDLE(int(child._handle))):
                raise c.WinError(c.get_last_error())
            snapshot = self.kernel.CreateToolhelp32Snapshot(4, 0)
            if snapshot == w.HANDLE(-1).value:
                raise c.WinError(c.get_last_error())
            try:
                entry = ThreadEntry()
                entry.size = c.sizeof(entry)
                found = self.kernel.Thread32First(snapshot, c.byref(entry))
                while found and entry.pid != child.pid:
                    found = self.kernel.Thread32Next(snapshot, c.byref(entry))
                if not found:
                    raise RuntimeError("suspended product's primary thread was not found")
                thread = self.kernel.OpenThread(2, False, entry.tid)
                if not thread:
                    raise c.WinError(c.get_last_error())
                try:
                    if self.kernel.ResumeThread(thread) == 0xFFFFFFFF:
                        raise c.WinError(c.get_last_error())
                finally:
                    self.kernel.CloseHandle(thread)
            finally:
                self.kernel.CloseHandle(snapshot)
            return child
        except BaseException:
            child.terminate()
            child.wait(timeout=10)
            raise

    def accounting(self):
        result = Accounting()
        if not self.kernel.QueryInformationJobObject(self.job, 1, c.byref(result), c.sizeof(result), None):
            raise c.WinError(c.get_last_error())
        return {"cpu_s": (result.user + result.kernel) / 1e7, "total_processes": result.total, "active_processes": result.active, "limit_terminations": result.terminated}

    def pids(self, active_only=False):
        with self.lock:
            if not active_only:
                return list(self.handles)
            result = []
            for pid in list(self.active):
                handle = self.handles[pid]
                code = w.DWORD()
                if self.kernel.GetExitCodeProcess(handle, c.byref(code)) and code.value == 259:
                    result.append(pid)
                else:
                    self.active.discard(pid)
            return result

    def receipt(self, sample):
        # Drain completion notifications before checking their count against
        # the independently maintained OS job accounting total.
        until = time.monotonic() + 2
        while (len(self.pids()) < self.accounting()["total_processes"] or self.accounting()["active_processes"]) and time.monotonic() < until:
            time.sleep(0.05)
        rows, errors = [], []
        with self.lock:
            handles = list(self.handles.items())
            images = dict(self.images)
            commands = dict(self.commands)
            missing = list(self.missing)
        for pid, handle in handles:
            try:
                metric = sample(pid)
                times = [w.FILETIME() for _ in range(4)]
                if not self.kernel.GetProcessTimes(handle, *(c.byref(t) for t in times)):
                    raise c.WinError(c.get_last_error())
                io = IoCounters()
                if not self.kernel.GetProcessIoCounters(handle, c.byref(io)):
                    raise c.WinError(c.get_last_error())
                ticks = lambda t: (t.dwHighDateTime << 32) | t.dwLowDateTime
                row = {"pid": pid, "image": images.get(pid, "unavailable at notification"), "created_100ns": ticks(times[0]), "exited_100ns": ticks(times[1]), "read_bytes": io.read_bytes, "write_bytes": io.write_bytes, **metric}
                if pid in commands:
                    row["command_capture"] = commands[pid]
                rows.append(row)
            except Exception as error:
                errors.append({"pid": pid, "error": str(error)})
        accounting = self.accounting()
        complete = not missing and not errors and len(rows) == accounting["total_processes"]
        # A ceiling bound, never a measured simultaneous RSS peak. Only sum
        # high-waters of processes whose kernel lifetime intervals overlap.
        events = []
        for row in rows:
            events.append((row["created_100ns"], row["peak_rss"]))
            if row["exited_100ns"]:
                events.append((row["exited_100ns"], -row["peak_rss"]))
        total = bound = 0
        for _, amount in sorted(events, key=lambda item: (item[0], -item[1])):
            total += amount
            bound = max(bound, total)
        return {"accounting": accounting, "processes": rows, "coverage": "COMPLETE" if complete else "BLOCKED", "missing_notifications": missing, "counter_errors": errors, "overlapping_lifetime_rss_upper_bound": bound if complete else None, "method": "OS high-waters retained through process handles; interval-overlap sum is a conservative ceiling bound, not a simultaneous sampled peak; notification count reconciled against job TotalProcesses"}

    def close(self):
        if self.accounting()["active_processes"]:
            # This unnamed job contains only the probe-launched product/tree.
            self.kernel.TerminateJobObject(self.job, 1)
        self.stop.set()
        self.thread.join(timeout=2)
        with self.lock:
            for handle in self.handles.values():
                self.kernel.CloseHandle(handle)
            self.handles.clear()
        self.kernel.CloseHandle(self.job)
        self.kernel.CloseHandle(self.port)
