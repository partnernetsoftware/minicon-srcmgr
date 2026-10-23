"""Opt-in Windows native resource/input probe; creates only disposable fixtures.

Reports desktop-observed focus feedback separately from client-surface feedback.
Raw receipts and captures belong under ignored target/, never in public docs.
"""

import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess as sp
import threading
import time


def fixture(git, root, files, commits):
    repo = root / "repo"
    repo.mkdir()
    run = lambda *args, **kw: sp.run([git, "-C", str(repo), *args], check=True, capture_output=True, **kw)
    run("init", "-q", "-b", "main")
    run("config", "user.name", "Fixture")
    run("config", "user.email", "fixture@example.invalid")
    run("config", "core.autocrlf", "false")
    stream = sp.Popen([git, "-C", str(repo), "fast-import", "--quiet"], stdin=sp.PIPE, stdout=sp.DEVNULL, stderr=sp.PIPE)
    content = b"one\ntwo\nthree\n"
    stream.stdin.write(b"blob\nmark :1\ndata %d\n" % len(content) + content + b"\n")
    for index in range(commits):
        stream.stdin.write(f"commit refs/heads/main\ncommitter Fixture <fixture@example.invalid> {1700000000 + index} +0000\ndata 9\nfixture\n\n".encode())
        if index == 0:
            for number in range(files):
                stream.stdin.write(f"M 100644 :1 f{number:05}.txt\n".encode())
        stream.stdin.write(b"\n")
    stream.stdin.close()
    error = stream.stderr.read()
    if stream.wait():
        raise RuntimeError(error.decode(errors="replace"))
    run("reset", "--hard", "-q")
    run("update-ref", "--stdin", input="".join(f"update refs/heads/ref-{i:03} HEAD\n" for i in range(199)).encode())
    for number in range(min(files, 200)):
        (repo / f"f{number:05}.txt").write_text("ONE\ntwo\nthree\n", encoding="utf-8")
    return repo


def native():
    user, gdi, kernel, psapi = (c.WinDLL(name, use_last_error=True) for name in ("user32", "gdi32", "kernel32", "psapi"))
    user.GetForegroundWindow.restype = w.HWND
    user.GetDC.argtypes, user.GetDC.restype = [w.HWND], w.HDC
    user.ReleaseDC.argtypes = [w.HWND, w.HDC]
    user.SetForegroundWindow.argtypes = [w.HWND]
    user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
    user.ShowWindow.argtypes = [w.HWND, c.c_int]
    user.GetDpiForWindow.argtypes, user.GetDpiForWindow.restype = [w.HWND], w.UINT
    user.GetClientRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
    user.ClientToScreen.argtypes = [w.HWND, c.POINTER(w.POINT)]
    user.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
    user.AttachThreadInput.argtypes = [w.DWORD, w.DWORD, w.BOOL]
    user.IsWindowVisible.argtypes = [w.HWND]
    gdi.GetPixel.argtypes, gdi.GetPixel.restype = [w.HDC, c.c_int, c.c_int], w.DWORD
    kernel.OpenProcess.argtypes, kernel.OpenProcess.restype = [w.DWORD, w.BOOL, w.DWORD], w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]

    class Memory(c.Structure):
        _fields_ = [("cb", w.DWORD), ("faults", w.DWORD)] + [(x, c.c_size_t) for x in ("peak_rss", "rss", "peak_paged", "paged", "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile", "private")]

    psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]
    kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
    def sample(pid):
        handle = kernel.OpenProcess(0x410, False, pid)
        if not handle:
            raise c.WinError(c.get_last_error())
        try:
            memory = Memory()
            memory.cb = c.sizeof(memory)
            if not psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb):
                raise c.WinError(c.get_last_error())
            times = [w.FILETIME() for _ in range(4)]
            if not kernel.GetProcessTimes(handle, *(c.byref(t) for t in times)):
                raise c.WinError(c.get_last_error())
            ticks = lambda t: (t.dwHighDateTime << 32) | t.dwLowDateTime
            return {"rss": memory.rss, "peak_rss": memory.peak_rss, "private": memory.private, "peak_private_commit": memory.peak_pagefile, "cpu_s": sum(ticks(t) for t in times[2:]) / 1e7}
        finally:
            kernel.CloseHandle(handle)

    class Keyboard(c.Structure):
        _fields_ = [("vk", w.WORD), ("scan", w.WORD), ("flags", w.DWORD), ("time", w.DWORD), ("extra", c.c_size_t)]
    class Payload(c.Union):
        _fields_ = [("keyboard", Keyboard), ("padding", c.c_byte * 32)]
    class Input(c.Structure):
        _fields_ = [("kind", w.DWORD), ("payload", Payload)]
    user.SendInput.argtypes, user.SendInput.restype = [w.UINT, c.POINTER(Input), c.c_int], w.UINT
    def key(hwnd, vk, ctrl=False):
        if user.GetForegroundWindow() != hwnd:
            # Temporarily attach the probe's input queue; never send to the
            # unrelated foreground window if activation is refused.
            thread = kernel.GetCurrentThreadId()
            foreground_thread = user.GetWindowThreadProcessId(user.GetForegroundWindow(), None)
            foreground_before = user.GetForegroundWindow()
            attached = user.AttachThreadInput(thread, foreground_thread, True)
            try:
                activated = user.SetForegroundWindow(hwnd)
            finally:
                if attached:
                    user.AttachThreadInput(thread, foreground_thread, False)
            if user.GetForegroundWindow() != hwnd:
                raise RuntimeError(f"activation refused: no input sent; attached={bool(attached)}, set_return={bool(activated)}, foreground_before={foreground_before}, foreground_after={user.GetForegroundWindow()}, target={hwnd}, target_visible={bool(user.IsWindowVisible(hwnd))}")
        if isinstance(vk, str):
            raw = vk.encode("utf-16-le", errors="surrogatepass")
            sequence = [(0, int.from_bytes(raw[i:i + 2], "little"), flag) for i in range(0, len(raw), 2) for flag in (4, 6)]
        elif not ctrl and (0x30 <= vk <= 0x39 or 0x41 <= vk <= 0x5A):
            # Send the intended literal character without changing the target
            # thread's keyboard layout or relying on an IME/CapsLock state.
            scan = ord(chr(vk).lower())
            sequence = [(0, scan, 4), (0, scan, 6)]
        else:
            sequence = [(0x11, 0, 0), (vk, 0, 0), (vk, 0, 2), (0x11, 0, 2)] if ctrl else [(vk, 0, 0), (vk, 0, 2)]
        items = (Input * len(sequence))()
        for index, (code, scan, flags) in enumerate(sequence):
            items[index].kind = 1
            items[index].payload.keyboard = Keyboard(code, scan, flags, 0, 0)
        if user.SendInput(len(items), items, c.sizeof(Input)) != len(items):
            raise c.WinError(c.get_last_error())
    return user, gdi, sample, key


def wait_quiet(hwnd, user, gdi, timeout=30):
    rect = w.RECT()
    user.GetClientRect(hwnd, c.byref(rect))
    scale = user.GetDpiForWindow(hwnd) / 96
    dc = user.GetDC(hwnd)
    x, y = rect.right - round(12 * scale) - 1, round(12 * scale) + 1
    try:
        until = time.monotonic() + timeout
        while gdi.GetPixel(dc, x, y) == 0xD69C56:
            if time.monotonic() >= until:
                raise RuntimeError("visible Git working marker did not clear")
            time.sleep(0.05)
    finally:
        user.ReleaseDC(hwnd, dc)


def desktop_focus(hwnd, user, gdi, key, points, receipt, busy_tree=None, selection_regions=None):
    """Pair an acquired desktop pixel with its OS update QPC, not readback time."""
    from windows_desktop_probe import DesktopSampler
    origin = w.POINT()
    if not user.ClientToScreen(hwnd, c.byref(origin)):
        raise c.WinError(c.get_last_error())
    sampler = DesktopSampler(origin.x, origin.y)
    client = user.GetDC(hwnd)
    rect = w.RECT()
    user.GetClientRect(hwnd, c.byref(rect))
    scale = user.GetDpiForWindow(hwnd) / 96
    result = receipt["latency_ms"]
    progress = receipt["desktop_progress"] = {"phase": "initialized", "completed_trials": 0}
    left_right = None
    try:
        for name, (x, y, vk) in points.items():
            expected = 0xD69C56
            reset_key = 0x31
            if selection_regions:
                # Measure an actual selected row/cursor, not only pane focus.
                if name == "Main":
                    key(hwnd, 0x32)
                    time.sleep(0.08)
                    wait_quiet(hwnd, user, gdi)
                    key(hwnd, 0x0D)
                else:
                    key(hwnd, vk)
                key(hwnd, 0x24)
                time.sleep(0.08)
                wait_quiet(hwnd, user, gdi)
                if left_right is None:
                    margin = round(12 * scale)
                    candidates = [column for column in range(margin + 2, rect.right - margin) if gdi.GetPixel(client, column, y) == 0xD69C56]
                    if not candidates:
                        raise RuntimeError("cannot locate the focused list's right border")
                    left_right = candidates[0]
                x = (rect.right - round(12 * scale) - 1 if name == "Main" else left_right) - round(8 * scale)
                top, bottom = selection_regions[name]
                selected = [row for row in range(top + 1, bottom - 1) if gdi.GetPixel(client, x, row) == 0x784F26]
                if not selected:
                    raise RuntimeError(f"no first-row selection visible in {name}")
                first = selected[0]
                height = 1
                while first + height in selected:
                    height += 1
                y = first + height + height // 2
                if y >= bottom - 1:
                    raise RuntimeError(f"no second visible row in {name}")
                expected, reset_key, vk = 0x784F26, 0x24, 0x28
            samples, completion, accumulated, epochs, active_git = [], [], [], [], []
            for trial in range(35):
                progress.update(panel=name, trial=trial, phase="reset activation")
                key(hwnd, reset_key)
                if selection_regions:
                    time.sleep(0.08)
                    wait_quiet(hwnd, user, gdi)
                reset = time.monotonic() + 3
                while True:
                    frame = sampler.read(origin.x + x, origin.y + y)
                    if frame and not frame["protected"] and frame["pixel"] != expected:
                        break
                    if time.monotonic() >= reset:
                        raise RuntimeError("desktop frame did not establish the focus reset")
                git_pids = []
                if busy_tree:
                    # Start a fresh read after verified quiescence. Sampling a
                    # previous read's last helper can race its view replacement
                    # and invalidate the selected diff row before presentation.
                    deadline = time.monotonic() + 15
                    quiet_since = None
                    while True:
                        live = [pid for pid in busy_tree.pids(active_only=True) if busy_tree.images.get(pid, "").lower() == "git.exe"]
                        working = gdi.GetPixel(client, rect.right - round(12 * scale) - 1, round(12 * scale) + 1) == 0xD69C56
                        if not live and not working:
                            if quiet_since is None:
                                quiet_since = time.monotonic()
                            if time.monotonic() - quiet_since >= 0.1:
                                break
                        else:
                            quiet_since = None
                        if time.monotonic() >= deadline:
                            raise RuntimeError("previous Git read did not settle before the busy-input trial")
                        time.sleep(0.01)
                    key(hwnd, 0x52)  # Actual read refresh; no hook/sleep simulation.
                    deadline = time.monotonic() + 5
                    while not git_pids:
                        git_pids = [pid for pid in busy_tree.pids(active_only=True) if busy_tree.images.get(pid, "").lower() == "git.exe"]
                        if time.monotonic() >= deadline:
                            raise RuntimeError("no owned live Git process for the busy-input trial")
                        if not git_pids:
                            time.sleep(0.002)
                before = time.perf_counter()
                input_epoch = time.time_ns() // 1000
                progress["phase"] = "target activation"
                key(hwnd, vk)
                deadline = time.monotonic() + 3
                while True:
                    frame = sampler.read(origin.x + x, origin.y + y)
                    if frame and frame["protected"]:
                        raise RuntimeError("desktop capture masks protected content")
                    if frame and frame["pixel"] == expected and frame["present_seconds"] >= before:
                        after = time.perf_counter()
                        if frame["present_seconds"] > after + 0.001:
                            raise RuntimeError("desktop frame/probe clocks do not agree")
                        if trial >= 5:
                            samples.append(round((frame["present_seconds"] - before) * 1000, 3))
                            completion.append(round((after - before) * 1000, 3))
                            accumulated.append(frame["accumulated"])
                            epochs.append(input_epoch)
                            active_git.append(git_pids)
                        progress["completed_trials"] += 1
                        break
                    if time.monotonic() >= deadline:
                        raise RuntimeError(f"desktop frame did not show input feedback for {name}")
                time.sleep(0.04)
            result[name] = {"samples": samples, "p95": sorted(samples)[math.ceil(len(samples) * .95) - 1],
                            "capture_completion_ms": completion, "accumulated_frames": accumulated,
                            "qpc_frequency": sampler.frequency.value,
                            "input_epoch_us": epochs, "active_git_pids": active_git,
                            "feedback": "selected row / diff cursor" if selection_regions else "pane focus border",
                            "method": "Actual acquired DXGI desktop feedback pixel and OS LastPresentTime QPC after verified reset; five warmups, 30 trials. Coalesced last-update time is an upper bound on first matching desktop update. Readback completion is separate; no subtraction. No physical scanout/photodiode claim."}
        return result
    finally:
        user.ReleaseDC(hwnd, client)
        sampler.close()


def sustained(git, repo, root, prefix, hwnd, user, gdi, key, seconds, cycles, child, exercise_faults, postcommit_fault=False):
    """Repeat actual inspect/edit/stage/commit/switch paths with idle intervals."""
    def press(vk, ctrl=False):
        key(hwnd, vk, ctrl)
        time.sleep(0.08)
        wait_quiet(hwnd, user, gdi)

    def run(*args):
        return sp.run([git, "-C", str(repo), *args], check=True, capture_output=True).stdout

    def expect(predicate, description):
        until = time.monotonic() + 20
        while time.monotonic() < until:
            if predicate(): return
            time.sleep(0.05)
        raise RuntimeError(description)

    begin = time.monotonic()
    completed = commits = 0
    failures = []

    def recoverable_fault(kind):
        target = repo / "f00000.txt"
        original = target.read_bytes()
        head = run("rev-parse", "HEAD")
        index = run("write-tree")
        if kind == "draft-save-io":
            press(0x43)
            for code in (0x46, 0x41, 0x49, 0x4C, 0x44, 0x52, 0x41, 0x46, 0x54):
                press(code)
            lock = root / "profile" / "profile.lock"
            with lock.open("xb") as output:
                output.write(b"owned qualification profile lock\n")
            try:
                press(0x1B)
                assert lock.read_bytes() == b"owned qualification profile lock\n"
            finally:
                lock.unlink()
            press(0x1B)  # Retry the retained text after storage is available.
            draft = sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip()
            if draft.lower() != b"faildraft":
                raise RuntimeError("failed draft save lost the original message before retry")
            press(0x43)
            for _ in range(len(draft)):
                press(0x08)
            press(0x1B)
            assert sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip() == b"no draft", "blank save must produce the CLI's documented empty-draft state"
        elif kind == "repository-disappearance":
            away = repo.with_name("repo-away")
            assert repo.resolve().parent == root and away.parent == root and not away.exists()
            repo.rename(away)
            try:
                press(0x52)
            finally:
                away.rename(repo)
        else:
            target.write_bytes(b"failure-probe\ntwo\nthree\n")
            press(0x52)
            press(0x24)
            try:
                if kind == "git-index-lock":
                    lock = repo / ".git" / "index.lock"
                    with lock.open("xb") as output:
                        output.write(b"owned qualification lock\n")
                    try:
                        press(0x20)
                        assert lock.read_bytes() == b"owned qualification lock\n", "product removed a foreign Git lock"
                    finally:
                        lock.unlink()
                elif kind == "profile-io":
                    recovery = root / "profile" / "recovery"
                    saved = root / "profile" / "recovery-saved"
                    assert recovery.parent.resolve().is_relative_to(root) and not saved.exists()
                    existed = recovery.exists()
                    if existed:
                        recovery.rename(saved)
                    try:
                        recovery.write_bytes(b"owned storage obstruction\n")
                        press(0x44)
                        press(0x59)
                    finally:
                        recovery.unlink()
                        if existed:
                            saved.rename(recovery)
                elif kind == "denied-file-access":
                    env = dict(os.environ, SRCMGR_PROBE_FILE=str(target))
                    def powershell(code, **extra):
                        shell = shutil.which("pwsh.exe") or shutil.which("powershell.exe")
                        return sp.run([shell, "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; " + code], env=dict(env, **extra), check=True, capture_output=True, creationflags=sp.CREATE_NO_WINDOW).stdout
                    descriptor = powershell("(Get-Acl -LiteralPath $env:SRCMGR_PROBE_FILE).Sddl").decode().strip()
                    try:
                        powershell("$acl=Get-Acl -LiteralPath $env:SRCMGR_PROBE_FILE; $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User; $rule=[System.Security.AccessControl.FileSystemAccessRule]::new($sid,[System.Security.AccessControl.FileSystemRights]::ReadData,[System.Security.AccessControl.AccessControlType]::Deny); $acl.AddAccessRule($rule); Set-Acl -LiteralPath $env:SRCMGR_PROBE_FILE -AclObject $acl")
                        try:
                            target.read_bytes()
                        except PermissionError:
                            pass
                        else:
                            raise RuntimeError("ACL injection did not deny a real file read")
                        press(0x44)
                        press(0x59)
                    finally:
                        powershell("$acl=Get-Acl -LiteralPath $env:SRCMGR_PROBE_FILE; $acl.SetSecurityDescriptorSddlForm($env:SRCMGR_PROBE_SDDL); Set-Acl -LiteralPath $env:SRCMGR_PROBE_FILE -AclObject $acl", SRCMGR_PROBE_SDDL=descriptor)
                elif kind == "confirmation-cancel":
                    press(0x44)
                    press(0x4E)
                assert target.read_bytes() == b"failure-probe\ntwo\nthree\n", f"{kind} lost worktree edits"
            finally:
                target.write_bytes(original)
        press(0x52)
        assert run("rev-parse", "HEAD") == head and run("write-tree") == index
        assert target.read_bytes() == original
        failures.append({"case": kind, "verdict": "PASS: intended refusal/cancel preserves HEAD, index and worktree; restored fixture and refreshed"})
        (root / "journey-progress.json").write_text(json.dumps({"completed_cycles": completed, "commits": commits, "recoverable_failures": failures}, indent=2), encoding="utf-8")
    while completed < cycles or time.monotonic() - begin < seconds:
        if child.poll() is not None:
            raise RuntimeError("product exited during sustained journey")
        press(0x32)  # Files
        if completed % 5 == 0:
            previous = run("rev-parse", "HEAD")
            content = f"soak-{completed}\ntwo\nthree\n".encode()
            (repo / "f00000.txt").write_bytes(content)
            press(0x52)  # Refresh actual external edit.
            press(0x41)  # Stage all; initial fixture also has 199 other changes.
            expect(lambda: bool(run("diff", "--cached", "--name-only")), "stage-all did not produce observed index changes")
            press(0x43)
            for code in (0x53, 0x4F, 0x41, 0x4B):
                press(code)
            if postcommit_fault:
                press(0x1B)
                assert sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip() == b"soak"
                press(0x43)
            press(0x0D, True)
            expect(lambda: run("rev-parse", "HEAD") != previous, "sustained GUI commit did not advance HEAD")
            wait_quiet(hwnd, user, gdi)
            if postcommit_fault:
                lock = root / "profile" / "profile.lock"
                assert lock.read_bytes() == b"owned post-write lock\n", "post-commit lock was not preserved"
                if user.GetForegroundWindow() != hwnd:
                    raise RuntimeError("cannot capture completed-write guidance outside the owned foreground window")
                from PIL import ImageGrab
                bounds = w.RECT()
                user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
                user.GetWindowRect(hwnd, c.byref(bounds))
                ImageGrab.grab(bbox=(bounds.left, bounds.top, bounds.right, bounds.bottom)).save(root / "postcommit-completed.png")
                lock.unlink()  # Only the verified owned injection is removed.
                press(0x52)
                assert sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip() == b"soak", "cleanup failure lost the durable draft"
                press(0x43)
                for _ in range(4):
                    press(0x08)
                press(0x1B)
                assert sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip() == b"no draft"
                failures.append({"case": "post-commit-profile-io", "verdict": "PASS: Git created the commit; foreign lock and durable draft preserved; refresh recovered after removing the owned injection", "capture": "postcommit-completed.png"})
            if run("show", "HEAD:f00000.txt") != content or run("diff", "--cached", "--name-only"):
                raise RuntimeError("sustained GUI commit/index integrity mismatch")
            commits += 1
        press(0x34)  # Commit -> files -> diff -> scroll -> back twice.
        press(0x24)
        press(0x0D)
        press(0x0D)
        press(0x22)
        press(0x1B)
        press(0x1B)
        press(0x33)  # Switch away and back with a clean worktree.
        press(0x24)
        press(0x28)
        press(0x20)
        if run("branch", "--show-current").strip() == b"main":
            raise RuntimeError("sustained GUI branch switch did not occur")
        press(0x24)
        press(0x20)
        if run("branch", "--show-current").strip() != b"main":
            raise RuntimeError("sustained GUI return to main failed")
        press(0x32)
        completed += 1
        if exercise_faults and completed in (2, 4, 6, 8, 10, 12):
            recoverable_fault({2: "repository-disappearance", 4: "git-index-lock", 6: "profile-io", 8: "denied-file-access", 10: "confirmation-cancel", 12: "draft-save-io"}[completed])
        if completed % 10 == 0:
            print(f"sustained cycles={completed}, commits={commits}, elapsed={time.monotonic() - begin:.1f}s", flush=True)
        # Idle is part of this journey, not another stream of refresh jobs.
        deadline = begin + seconds * min(completed / max(1, cycles), 1)
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError("product exited during sustained idle")
            time.sleep(max(0, min(0.1, deadline - time.monotonic())))
    integrity = run("fsck", "--no-reflogs", "--no-dangling").decode(errors="replace")
    press(0x43)
    for code in (0x44, 0x52, 0x41, 0x46, 0x54):
        press(code)
    press(0x1B)
    draft = sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip()
    if draft.lower() != b"draft":
        raise RuntimeError("acknowledged GUI draft was not stored")
    scope = "Delivered native local paths with synthetic fixtures; controlled post-commit hook injects only the owned profile lock" if postcommit_fault else "Delivered native local paths with synthetic fixtures and no custom hooks"
    return {"seconds": time.monotonic() - begin, "cycles": completed, "commits": commits, "recoverable_failures": failures, "acknowledged_draft": draft.decode(), "integrity": "PASS: committed bytes/index, branch return each cycle; final git fsck", "fsck_output": integrity, "scope": scope + "; no power-loss or cross-platform claim"}


def workflow(git, repo, root, hwnd, user, key):
    """One real changed-path smoke, with Git oracles and reviewable captures."""
    from PIL import ImageGrab

    def run(*args):
        return sp.run([git, "-C", str(repo), *args], capture_output=True, check=True).stdout

    def press(vk):
        key(hwnd, vk)
        time.sleep(0.1)
        quiet()

    def wait_for(predicate, description):
        until = time.monotonic() + 15
        while time.monotonic() < until:
            if predicate():
                return
            time.sleep(0.1)
        raise RuntimeError(f"workflow timeout: {description}")

    captures = []
    def quiet():
        # The persistent header busy marker remains visible even when footer
        # hints are truncated. Wait for completion, never guess from Git state.
        rect = w.RECT()
        user.GetClientRect(hwnd, c.byref(rect))
        scale = user.GetDpiForWindow(hwnd) / 96
        dc = user.GetDC(hwnd)
        gdi = c.WinDLL("gdi32", use_last_error=True)
        gdi.GetPixel.argtypes, gdi.GetPixel.restype = [w.HDC, c.c_int, c.c_int], w.DWORD
        x, y = rect.right - round(12 * scale) - 1, round(12 * scale) + 1
        try:
            wait_for(lambda: gdi.GetPixel(dc, x, y) != 0xD69C56, "visible Git working marker to clear")
        finally:
            user.ReleaseDC(hwnd, dc)
        time.sleep(0.1)

    def capture(name):
        rect = w.RECT()
        user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
        if not user.GetWindowRect(hwnd, c.byref(rect)):
            raise c.WinError(c.get_last_error())
        path = root / f"workflow-{name}.png"
        ImageGrab.grab(bbox=(rect.left, rect.top, rect.right, rect.bottom)).save(path)
        captures.append(path.name)

    target = repo / "f00000.txt"
    quiet()
    before = target.read_bytes()
    press(0x32)  # Files
    press(0x24)  # Home
    press(0x20)  # Stage
    wait_for(lambda: b"f00000.txt" in run("diff", "--cached", "--name-only"), "stage index")
    time.sleep(1)
    capture("stage-result")
    assert target.read_bytes() == before, "stage wrote worktree"
    press(0x20)
    wait_for(lambda: not run("diff", "--cached", "--name-only"), "unstage index")
    press(0x44)  # Discard prompt
    capture("discard-prompt")
    press(0x4E)  # Cancel
    assert target.read_bytes() == before
    press(0x44)
    press(0x59)
    wait_for(lambda: target.read_bytes() == b"one\ntwo\nthree\n", "confirmed discard")
    capture("discard-result")
    copies = list((root / "profile" / "recovery").glob("*/worktree.bin"))
    assert copies, "missing recovery copy"
    env = dict(os.environ, SRCMGR_PROBE_DIRECTORY=str(copies[0].parent))
    shell = shutil.which("pwsh.exe") or shutil.which("powershell.exe")
    code = "$ErrorActionPreference='Stop'; $acl=Get-Acl -LiteralPath $env:SRCMGR_PROBE_DIRECTORY; $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $other=@($acl.Access | Where-Object { $_.AccessControlType -eq 'Allow' -and $_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value -ne $sid }); [pscustomobject]@{protected=$acl.AreAccessRulesProtected;otherAllowEntries=$other.Count} | ConvertTo-Json -Compress"
    acl = json.loads(sp.run([shell, "-NoProfile", "-NonInteractive", "-Command", code], env=env, capture_output=True, check=True, creationflags=sp.CREATE_NO_WINDOW).stdout)
    assert acl["protected"] and acl["otherAllowEntries"] == 0, "recovery payload directory is not private to the current user"
    press(0x35)  # Stashes
    press(0x23)  # End: older of two
    capture("stash-list")
    ids = run("stash", "list", "--format=%H").splitlines()
    assert len(ids) == 2
    press(0x20)
    capture("stash-apply-prompt")
    press(0x59)
    wait_for(lambda: target.read_bytes() == before, "selected stash apply")
    assert run("stash", "list", "--format=%H").splitlines() == ids
    capture("stash-applied")
    press(0x44)  # Preview older drop, then an external writer changes the list.
    run("stash", "push", "-qm", "external", "--", "f00000.txt")
    changed_ids = run("stash", "list", "--format=%H").splitlines()
    press(0x59)
    time.sleep(0.6)
    assert run("stash", "list", "--format=%H").splitlines() == changed_ids, "stale drop mutated stash list"
    capture("stash-stale-refusal")
    press(0x50)  # Pop the still-selected older full identity.
    capture("stash-pop-prompt")
    press(0x59)
    wait_for(lambda: target.read_bytes() == before, "selected stash pop")
    after_pop = run("stash", "list", "--format=%H").splitlines()
    assert after_pop == [oid for oid in changed_ids if oid != ids[1]], "pop removed another source"
    capture("stash-popped")
    press(0x44)  # Drop the remaining selected newer entry, preserving worktree.
    capture("stash-drop-prompt")
    press(0x59)
    after_drop = run("stash", "list", "--format=%H").splitlines()
    assert after_drop == [changed_ids[0]], "drop removed another entry"
    assert target.read_bytes() == before, "drop wrote worktree"
    capture("stash-dropped")
    press(0x34)  # Commits
    press(0x23)  # End: root (smoke uses <= 200 commits)
    press(0x0D)
    time.sleep(0.6)
    capture("commit-files")
    press(0x0D)
    time.sleep(0.6)
    capture("commit-file-diff")
    press(0x1B)
    capture("commit-return")
    press(0x1B)
    return {"git_oracles": "PASS: stage/index/worktree; discard cancel/copy; selected apply retains source; stale drop preserves list; exact pop/drop preserve other entries/worktree", "recovery_acl": acl, "captures": captures, "visual_verdict": "REVIEW-REQUIRED: inspect captured labels/focus/return; Git oracles do not prove rendering"}


def native_title(hwnd, user):
    user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
    title = c.create_unicode_buffer(32768)
    user.GetWindowTextW(hwnd, title, len(title))
    return title.value


def wait_entry(predicate, label):
    until = time.monotonic() + 15
    while time.monotonic() < until:
        if predicate(): return
        time.sleep(0.05)
    raise RuntimeError(label)


def capture_entry(hwnd, user, gdi, root, name):
    from PIL import ImageGrab
    wait_quiet(hwnd, user, gdi)
    time.sleep(0.15)
    if user.GetForegroundWindow() != hwnd:
        raise RuntimeError("cannot capture entry outside the owned foreground window")
    bounds = w.RECT()
    user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
    if not user.GetWindowRect(hwnd, c.byref(bounds)):
        raise c.WinError(c.get_last_error())
    user.WindowFromPoint.argtypes, user.WindowFromPoint.restype = [w.POINT], w.HWND
    owner = w.DWORD()
    user.GetWindowThreadProcessId(hwnd, c.byref(owner))
    for x, y in ((bounds.left + 20, bounds.top + 40), (bounds.right - 20, bounds.top + 40), ((bounds.left + bounds.right) // 2, (bounds.top + bounds.bottom) // 2), (bounds.left + 20, bounds.bottom - 20), (bounds.right - 20, bounds.bottom - 20)):
        visible = user.WindowFromPoint(w.POINT(x, y))
        process = w.DWORD()
        user.GetWindowThreadProcessId(visible, c.byref(process))
        if process.value != owner.value:
            raise RuntimeError("owned capture is occluded by another process")
    ImageGrab.grab(bbox=(bounds.left, bounds.top, bounds.right, bounds.bottom)).save(root / name)


def click_review_button(hwnd, user, gdi, allow):
    # Locate the actual default-Cancel blue border, then the adjacent grey
    # approval border. This observes native pixels, not a hidden product hook.
    if user.GetForegroundWindow() != hwnd:
        raise RuntimeError("foreground guard refused a trust-button click")
    rect = w.RECT()
    user.GetClientRect(hwnd, c.byref(rect))
    dc = user.GetDC(hwnd)
    try:
        found = None
        for y in range(rect.bottom - 1, rect.bottom // 2, -1):
            runs, current = [], []
            for x in range(rect.right * 40 // 100, rect.right):
                if gdi.GetPixel(dc, x, y) == 0xD69C56:
                    current.append(x)
                else:
                    if 40 < len(current) < rect.right // 3: runs.append(current)
                    current = []
            if 40 < len(current) < rect.right // 3: runs.append(current)
            if runs:
                # A form's surrounding focus frame may be below its buttons.
                # Match a button-width contiguous run, not that entire frame.
                found = max(runs, key=lambda run: max(run)), y
                break
        if found is None:
            raise RuntimeError("cannot locate rendered Cancel control")
        columns, y = found
        if allow:
            runs, current = [], []
            for x in range(rect.right * 38 // 100, min(columns)):
                if gdi.GetPixel(dc, x, y) == 0x3C3C3C:
                    current.append(x)
                else:
                    if len(current) > 40: runs.append(current)
                    current = []
            if len(current) > 40: runs.append(current)
            if not runs: raise RuntimeError("cannot locate rendered Trust and open control")
            columns = max(runs, key=len)
        x = (min(columns) + max(columns)) // 2
        y -= max(3, round(3 * user.GetDpiForWindow(hwnd) / 96))
    finally:
        user.ReleaseDC(hwnd, dc)
    origin = w.POINT(x, y)
    if not user.ClientToScreen(hwnd, c.byref(origin)):
        raise c.WinError(c.get_last_error())
    user.WindowFromPoint.argtypes, user.WindowFromPoint.restype = [w.POINT], w.HWND
    if user.GetForegroundWindow() != hwnd or user.WindowFromPoint(origin) != hwnd:
        raise RuntimeError("owned-window guard refused trust-button click")
    user.SetCursorPos.argtypes, user.SetCursorPos.restype = [c.c_int, c.c_int], w.BOOL
    if not user.SetCursorPos(origin.x, origin.y):
        raise RuntimeError("native pointer positioning was refused")
    actual = w.POINT()
    if not user.GetCursorPos(c.byref(actual)) or (actual.x, actual.y) != (origin.x, origin.y) or user.GetForegroundWindow() != hwnd or user.WindowFromPoint(actual) != hwnd:
        raise RuntimeError("pointer/foreground changed before trust-button click")
    user.mouse_event(0x0002, 0, 0, 0, 0)
    user.mouse_event(0x0004, 0, 0, 0, 0)


def welcome_entry_start(repo, root, hwnd, user, gdi, key, tree):
    wait_quiet(hwnd, user, gdi)
    if native_title(hwnd, user) != "MiniCon SrcMgr":
        raise RuntimeError("expected repository-independent Welcome")
    capture_entry(hwnd, user, gdi, root, "welcome-empty.png")

    def zero_git():
        wait_entry(lambda: tree.accounting()["total_processes"] == len(tree.pids()), "entry process notifications did not reconcile")
        images = dict(tree.images)
        assert all(pid in images for pid in tree.pids()), "unknown process image before trust approval"
        count = sum(name.lower() == "git.exe" for name in images.values())
        assert count == 0, "Git ran before explicit trust approval"
        return count

    before = zero_git()
    key(hwnd, 0x4F, ctrl=True)
    key(hwnd, str(repo))
    key(hwnd, 0x0D)
    wait_entry(lambda: native_title(hwnd, user).startswith("MiniCon SrcMgr - Trust review | "), "trust review did not open")
    capture_entry(hwnd, user, gdi, root, "welcome-trust-review.png")
    zero_git()
    assert not (root / "profile" / "trust.v1").exists(), "review saved authority before approval"
    click_review_button(hwnd, user, gdi, False)
    wait_entry(lambda: native_title(hwnd, user) == "MiniCon SrcMgr", "Cancel did not restore Welcome context")
    zero_git()
    assert not (root / "profile" / "trust.v1").exists(), "Cancel created a trust grant"
    key(hwnd, 0x0D)  # Retry the preserved path; opening Enter cannot approve it.
    wait_entry(lambda: native_title(hwnd, user).startswith("MiniCon SrcMgr - Trust review | "), "retry did not prepare a new review")
    wait_quiet(hwnd, user, gdi)
    time.sleep(0.15)
    zero_git()
    click_review_button(hwnd, user, gdi, True)
    wait_entry(lambda: " | " in native_title(hwnd, user) and "Trust review" not in native_title(hwnd, user), "explicit pointer approval did not open the repository")
    wait_quiet(hwnd, user, gdi)
    capture_entry(hwnd, user, gdi, root, "welcome-approved.png")
    return {"before_approval_git_processes": before, "cancel": "PASS: pointer Cancel preserved no-grant/no-Git state and original path", "approval": "PASS: pointer Trust and open acknowledged the selected repository", "captures": ["welcome-empty.png", "welcome-trust-review.png", "welcome-approved.png"], "visual_verdict": "REVIEW-REQUIRED", "scope": "Fresh-profile Welcome and explicit trust only; no init, settings or latency qualification"}


def entry_smoke(git, repo, other, root, prefix, hwnd, user, gdi, key, approve_other=False):
    """Actual Open/input, index and draft routing, in two owned fixtures."""
    import sqlite3
    from PIL import ImageGrab

    def run(path, *args):
        return sp.run([git, "-C", str(path), *args], check=True, capture_output=True).stdout

    def normalize(path):
        return os.path.normcase(str(path).removeprefix("\\\\?\\"))

    def draft(path):
        with sqlite3.connect(root / "profile" / "profile.sqlite3") as db:
            rows = db.execute("SELECT repository, body FROM drafts WHERE backend='git'").fetchall()
        return next((body.encode() for location, body in rows if normalize(location) == normalize(path / ".git")), None)

    def wait(predicate, label):
        until = time.monotonic() + 15
        while time.monotonic() < until:
            if predicate(): return
            time.sleep(0.05)
        raise RuntimeError(label)

    def recent_is(path):
        with sqlite3.connect(root / "profile" / "profile.sqlite3") as db:
            value = db.execute("SELECT value FROM settings WHERE key='workspace.recents.v1'").fetchone()
        return value and normalize(bytes.fromhex(value[0].splitlines()[0].split("\t")[2]).decode("utf-16-le")) == normalize(path)

    def open_repo(path):
        wait_quiet(hwnd, user, gdi)
        key(hwnd, 0x4F, ctrl=True)
        key(hwnd, str(path))
        key(hwnd, 0x0D)
        if approve_other and path == other:
            wait_entry(lambda: native_title(hwnd, user).startswith("MiniCon SrcMgr - Trust review | "), "other repository review did not open")
            wait_quiet(hwnd, user, gdi)
            key(hwnd, 0x59)
        wait(lambda: recent_is(path), "Open did not acknowledge the intended recent worktree")
        user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
        def title_matches():
            title = c.create_unicode_buffer(32768)
            user.GetWindowTextW(hwnd, title, len(title))
            return normalize(title.value.removeprefix("MiniCon SrcMgr - ").split(" | ")[-1]) == normalize(path)
        wait(title_matches, "native window title did not identify the opened worktree")
        wait_quiet(hwnd, user, gdi)
        time.sleep(0.2)  # Functional settling only; never reported as latency.

    def capture(name):
        wait_quiet(hwnd, user, gdi)
        time.sleep(0.15)
        if user.GetForegroundWindow() != hwnd:
            raise RuntimeError("cannot capture entry outside the owned foreground window")
        bounds = w.RECT()
        user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
        if not user.GetWindowRect(hwnd, c.byref(bounds)):
            raise c.WinError(c.get_last_error())
        ImageGrab.grab(bbox=(bounds.left, bounds.top, bounds.right, bounds.bottom)).save(root / name)

    source_head, source_index = run(repo, "rev-parse", "HEAD"), run(repo, "write-tree")
    source_bytes = (repo / "f00000.txt").read_bytes()
    target_bytes = (other / "f00000.txt").read_bytes()
    open_repo(other)
    key(hwnd, 0x20)
    wait(lambda: run(other, "show", ":f00000.txt") == target_bytes, "stage did not target the opened worktree")
    key(hwnd, 0x43)
    key(hwnd, " edited")
    key(hwnd, 0x1B)
    wait(lambda: draft(other) == b"target draft edited", "target GUI draft not acknowledged")
    capture("entry-target.png")
    assert draft(repo) == b"source draft", "source draft was overwritten"
    assert run(repo, "write-tree") == source_index, "source index changed after Open"
    open_repo(repo)
    key(hwnd, 0x43)
    key(hwnd, " resumed")
    key(hwnd, 0x1B)
    wait(lambda: draft(repo) == b"source draft resumed", "return did not restore the source draft")
    assert draft(other) == b"target draft edited", "return overwrote target draft"
    assert run(repo, "rev-parse", "HEAD") == source_head and run(repo, "write-tree") == source_index
    assert (repo / "f00000.txt").read_bytes() == source_bytes
    capture("entry-return.png")
    return {"git_state": "PASS: selected stage touched only target index; source HEAD/index/worktree preserved", "drafts": "PASS: independent acknowledged drafts after Open and return", "captures": ["entry-target.png", "entry-return.png"], "visual_verdict": "REVIEW-REQUIRED", "scope": "Two trusted owned fixtures; no tabs, first-run trust, init or latency claim"}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--exe", required=True)
    p.add_argument("--git", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--files", type=int, default=20000)
    p.add_argument("--commits", type=int, default=100000)
    p.add_argument("--idle-seconds", type=int, default=60)
    p.add_argument("--settle-seconds", type=int, default=30)
    p.add_argument("--tree", action="store_true", help="account for product and helpers in an owned Windows job")
    p.add_argument("--measure-ui", action="store_true", help="opt into bounded local application timing receipts inside the isolated profile")
    p.add_argument("--desktop-frames", action="store_true", help="observe DXGI desktop pixels paired with OS update timestamps; fail if unavailable")
    p.add_argument("--busy-latency", action="store_true", help="require owned live Git at each desktop-input trial and verify kernel lifetime overlap")
    p.add_argument("--selection-latency", action="store_true", help="measure selected rows and the diff cursor instead of only focus borders")
    p.add_argument("--skip-latency", action="store_true", help="resource/workflow-only probe; records latency as NOT-RUN")
    p.add_argument("--postcommit-profile-fault", action="store_true", help="one-cycle owned post-commit storage failure and completion-guidance capture")
    p.add_argument("--cycles", type=int, default=0)
    p.add_argument("--soak-seconds", type=int, default=0)
    p.add_argument("--exercise-faults", action="store_true", help="inject supported failure/cancel cases only in the owned sustained fixture")
    p.add_argument("--workflow", action="store_true", help="run changed-path smoke and capture screens; requires Pillow and <= 200 commits")
    p.add_argument("--entry-smoke", action="store_true", help="two-owned-repository Open/index/draft journey; requires small fixture and skip-latency")
    p.add_argument("--welcome-entry", action="store_true", help="start without a repository; prove no Git before pointer trust approval; requires entry-smoke and tree")
    p.add_argument("--owned-topmost", action="store_true", help="temporarily raise only this probe window on a shared desktop; requires skip-latency")
    args = p.parse_args()
    if os.name != "nt":
        p.error("this native probe requires Windows")
    if min(args.idle_seconds, args.files, args.commits) < 1 or min(args.settle_seconds, args.cycles, args.soak_seconds) < 0:
        p.error("files, commits and idle duration must be positive; other durations/counts cannot be negative")
    if args.soak_seconds and not args.cycles:
        args.cycles = 100
    if args.workflow and (args.commits > 200 or args.files < 1):
        p.error("workflow smoke requires 1..200 commits and at least one file")
    if args.skip_latency and args.desktop_frames:
        p.error("skip-latency and desktop-frames cannot be combined")
    if args.owned_topmost and not args.skip_latency:
        p.error("owned-topmost is functional isolation, not a presentation benchmark")
    if args.welcome_entry and not (args.entry_smoke and args.tree):
        p.error("welcome-entry requires entry-smoke and tree")
    if args.entry_smoke and (max(args.files, args.commits) > 200 or not args.skip_latency or args.cycles or args.soak_seconds or args.workflow or args.exercise_faults or args.postcommit_profile_fault):
        p.error("entry-smoke requires <=200 files/commits, skip-latency and no other workflow/fault/soak mode")
    if args.busy_latency and not (args.tree and args.desktop_frames):
        p.error("busy-latency requires tree accounting and desktop-frames")
    if args.selection_latency and (not args.desktop_frames or min(args.files, args.commits) < 2):
        p.error("selection-latency requires desktop-frames and at least two files/commits")
    if args.postcommit_profile_fault and (args.cycles != 1 or args.soak_seconds or args.exercise_faults or args.workflow):
        p.error("postcommit-profile-fault requires one cycle without other workflow/fault/soak modes")
    root = Path(args.out).resolve()
    root.mkdir(parents=True, exist_ok=False)
    exe = str(Path(args.exe).resolve())
    repo = fixture(args.git, root, args.files, args.commits)
    if args.postcommit_profile_fault:
        import shlex
        lock = root / "profile" / "profile.lock"
        (repo / ".git" / "hooks" / "post-commit").write_bytes(("#!/bin/sh\nprintf 'owned post-write lock\\n' > " + shlex.quote(lock.as_posix()) + "\n").encode("utf-8"))
    if args.workflow:
        for label, content in (("older", None), ("newer", "NEWER\ntwo\nthree\n")):
            if content:
                (repo / "f00000.txt").write_text(content, encoding="utf-8")
            sp.run([args.git, "-C", str(repo), "stash", "push", "-qm", label, "--", "f00000.txt"], check=True, capture_output=True)
        (repo / "f00000.txt").write_text("ONE\ntwo\nthree\n", encoding="utf-8")
    prefix = [exe, "--git", args.git, "--profile", str(root / "profile")]
    if not args.welcome_entry:
        sp.run(prefix + ["trust", "grant", str(repo)], check=True, capture_output=True)
    sp.run(prefix + ["setting", "set", "history.limit", "200"], check=True, capture_output=True)
    other = None
    if args.entry_smoke:
        other_root = root / "entry-other"
        other_root.mkdir()
        other = fixture(args.git, other_root, 5, 2)
        target = other_root / ("target repo 中文" if args.welcome_entry else "target-repo")
        other.rename(target)
        other = target.resolve()
        sp.run([args.git, "-C", str(other), "branch", "-m", "entry-target"], check=True, capture_output=True)
        if not args.welcome_entry:
            sp.run(prefix + ["trust", "grant", str(other)], check=True, capture_output=True)
        for path, text in ((repo, "source draft"), (other, "target draft")):
            sp.run(prefix + ["draft", str(path), "save", "-m", text], check=True, capture_output=True)
    user, gdi, sample, key = native()
    previous = user.GetForegroundWindow()
    from windows_job_probe import OwnedTree
    tree = OwnedTree() if args.tree else None
    kwargs = {"stdout": sp.DEVNULL, "stderr": (root / "stderr.log").open("w")}
    if args.measure_ui:
        kwargs["env"] = dict(os.environ, SRCMGR_MEASURE_UI="1")
    launch = prefix + (["gui"] if args.welcome_entry else ["gui", str(repo)])
    child = tree.spawn(launch, **kwargs) if tree else sp.Popen(launch, creationflags=sp.CREATE_NO_WINDOW, **kwargs)
    harness_first = sample(os.getpid())
    receipt = {"artifact_sha256": hashlib.sha256(Path(exe).read_bytes()).hexdigest(), "os": platform.platform(), "fixture": {"files": args.files, "commits": args.commits, "refs": 200, "changed": min(args.files, 200)}, "git": sp.check_output([args.git, "--version"], text=True).strip()}
    receipt["desktop_pixels"] = [user.GetSystemMetrics(0), user.GetSystemMetrics(1)]
    receipt["logical_cpus"] = os.cpu_count()
    receipt["product_pid"] = child.pid
    receipt["owned_topmost"] = args.owned_topmost
    receipt["character_input"] = "Native Unicode SendInput for literal ASCII commands/text; virtual keys for named/control chords; keyboard layout unchanged"
    receipt["history_limit"] = 200
    receipt["harness_sha256"] = {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_workspace.py", "windows_job_probe.py")}
    if args.desktop_frames:
        receipt["harness_sha256"]["windows_desktop_probe.py"] = hashlib.sha256(Path(__file__).with_name("windows_desktop_probe.py").read_bytes()).hexdigest()
    monitoring_done = threading.Event()
    trace_summary = {"samples": 0, "app_sampled_peak_rss": 0, "tree_sampled_peak_rss": 0, "max_sample_gap_ms": 0, "errors": []}
    def monitor():
        previous_sample = time.monotonic()
        with (root / "resources.jsonl").open("w", encoding="utf-8") as trace:
            while not monitoring_done.is_set():
                now = time.monotonic()
                trace_summary["max_sample_gap_ms"] = max(trace_summary["max_sample_gap_ms"], (now - previous_sample) * 1000)
                previous_sample = now
                try:
                    app = sample(child.pid)
                    pids = tree.pids(active_only=True) if tree else [child.pid]
                    rss = sum(sample(pid)["rss"] for pid in pids)
                    trace.write(json.dumps({"monotonic_s": now, "app": app, "tree_sampled_rss": rss, "live_pids": pids}) + "\n")
                    trace_summary["samples"] += 1
                    trace_summary["app_sampled_peak_rss"] = max(trace_summary["app_sampled_peak_rss"], app["rss"])
                    trace_summary["tree_sampled_peak_rss"] = max(trace_summary["tree_sampled_peak_rss"], rss)
                except Exception as error:
                    trace_summary["errors"].append(str(error))
                monitoring_done.wait(max(0, 0.025 - (time.monotonic() - now)))
    monitoring = threading.Thread(target=monitor, daemon=True)
    monitoring.start()
    hwnd = None
    callback_type = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    def enum(window, _):
        nonlocal hwnd
        pid = w.DWORD()
        user.GetWindowThreadProcessId(window, c.byref(pid))
        if pid.value == child.pid and user.IsWindowVisible(window):
            hwnd = window
        return True
    callback = callback_type(enum)
    user.EnumWindows.argtypes = [callback_type, w.LPARAM]
    try:
        deadline = time.monotonic() + 30
        while not hwnd and time.monotonic() < deadline:
            user.EnumWindows(callback, 0)
            if child.poll() is not None:
                raise RuntimeError("window exited before becoming visible")
            time.sleep(0.05)
        if not hwnd:
            raise RuntimeError("no native window after 30 seconds")
        time.sleep(1)
        user.ShowWindow(hwnd, 9)
        if args.owned_topmost:
            owner = w.DWORD()
            user.GetWindowThreadProcessId(hwnd, c.byref(owner))
            if owner.value != child.pid:
                raise RuntimeError("cannot raise a foreign window")
            user.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
            user.SetWindowPos.restype = w.BOOL
            if not user.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x0043):
                raise RuntimeError("owned-window functional isolation was refused")
        user.SetForegroundWindow(hwnd)
        time.sleep(0.2)
        scale = user.GetDpiForWindow(hwnd) / 96
        rect = w.RECT()
        user.GetClientRect(hwnd, c.byref(rect))
        receipt["client_pixels"], receipt["scale"] = [rect.right, rect.bottom], scale
        if args.welcome_entry:
            receipt["welcome"] = welcome_entry_start(repo, root, hwnd, user, gdi, key, tree)
        else:
            wait_entry(lambda: native_title(hwnd, user).startswith("MiniCon SrcMgr - ") and "Trust review" not in native_title(hwnd, user), "initial repository did not open")
            wait_quiet(hwnd, user, gdi)
        dc = user.GetDC(hwnd)
        screen_dc = user.GetDC(None)
        try:
            margin = round(12 * scale)
            pixel = lambda x, y: gdi.GetPixel(dc, x, y)
            segments = []
            start = None
            for y in range(rect.bottom):
                border = pixel(margin, y) in (0x3C3C3C, 0xD69C56)
                if border and start is None:
                    start = y
                if not border and start is not None:
                    if y - start > 10:
                        segments.append((start, y))
                    start = None
            if len(segments) != 4:
                raise RuntimeError(f"expected 4 panel borders, observed {segments}")
            points = {"Files": (margin, sum(segments[1]) // 2, 0x32), "Commits": (margin, sum(segments[3]) // 2, 0x34), "Main": (rect.right - margin - 1, sum(segments[1]) // 2, 0x0D)}
            receipt["latency_ms"] = {}
            if args.skip_latency:
                receipt["latency_status"] = "NOT-RUN: explicitly disabled for resource/workflow measurement"
            for name, (x, y, vk) in ([] if args.desktop_frames or args.skip_latency else points.items()):
                trials, surface_trials = [], []
                for trial in range(35):
                    key(hwnd, 0x31)
                    origin = w.POINT()
                    user.ClientToScreen(hwnd, c.byref(origin))
                    visible_pixel = lambda: gdi.GetPixel(screen_dc, origin.x + x, origin.y + y)
                    deadline = time.monotonic() + 2
                    while (pixel(x, y) == 0xD69C56 or visible_pixel() == 0xD69C56) and time.monotonic() < deadline:
                        time.sleep(0.001)
                    if pixel(x, y) == 0xD69C56 or visible_pixel() == 0xD69C56:
                        raise RuntimeError("focus reset did not present")
                    before = time.perf_counter()
                    key(hwnd, vk)
                    deadline = time.monotonic() + 2
                    surface_ms = visible_ms = None
                    while (surface_ms is None or visible_ms is None) and time.monotonic() < deadline:
                        if surface_ms is None and pixel(x, y) == 0xD69C56:
                            surface_ms = (time.perf_counter() - before) * 1000
                        if visible_ms is None and visible_pixel() == 0xD69C56:
                            visible_ms = (time.perf_counter() - before) * 1000
                        time.sleep(0.001)
                    if visible_ms is None or surface_ms is None:
                        raise RuntimeError(f"no visible focus feedback for {name}")
                    if trial >= 5:
                        trials.append(round(visible_ms, 3))
                        surface_trials.append(round(surface_ms, 3))
                    time.sleep(0.04)
                receipt["latency_ms"][name] = {"samples": trials, "p95": sorted(trials)[math.ceil(len(trials) * .95) - 1], "surface_samples": surface_trials, "surface_p95": sorted(surface_trials)[math.ceil(len(surface_trials) * .95) - 1], "method": "Desktop DC pixel at ClientToScreen coordinates; client DC sampled separately; both reset before input; polling/GDI overhead included"}
        finally:
            user.ReleaseDC(hwnd, dc)
            user.ReleaseDC(None, screen_dc)
        if args.desktop_frames:
            regions = {"Files": segments[1], "Commits": segments[3], "Main": (round(12 * scale), rect.bottom)} if args.selection_latency else None
            receipt["latency_ms"] = desktop_focus(hwnd, user, gdi, key, points, receipt, tree if args.busy_latency else None, regions)
        if args.workflow:
            receipt["workflow"] = workflow(args.git, repo, root, hwnd, user, key)
        if args.entry_smoke:
            receipt["entry"] = entry_smoke(args.git, repo, other, root, prefix, hwnd, user, gdi, key, args.welcome_entry)
        wait_quiet(hwnd, user, gdi)
        time.sleep(args.settle_seconds)
        initial_settled = sample(child.pid)
        if args.cycles or args.soak_seconds:
            receipt["sustained"] = sustained(args.git, repo, root, prefix, hwnd, user, gdi, key, args.soak_seconds, args.cycles, child, args.exercise_faults, args.postcommit_profile_fault)
            wait_quiet(hwnd, user, gdi)
            time.sleep(args.settle_seconds)
            final_settled = sample(child.pid)
            receipt["repeated_use"] = {"settle_seconds": args.settle_seconds, "initial": initial_settled, "final": final_settled, "rss_growth": final_settled["rss"] - initial_settled["rss"]}
            old_pid = child.pid
            user.PostMessageW(hwnd, 0x10, 0, 0)
            child.wait(timeout=10)
            if child.returncode != 0:
                raise RuntimeError("controlled close failed before draft restart check")
            hwnd = None
            child = tree.spawn(prefix + ["gui", str(repo)], **kwargs) if tree else sp.Popen(prefix + ["gui", str(repo)], creationflags=sp.CREATE_NO_WINDOW, **kwargs)
            until = time.monotonic() + 30
            while not hwnd and time.monotonic() < until:
                user.EnumWindows(callback, 0)
                if child.poll() is not None:
                    raise RuntimeError("restarted GUI exited before becoming visible")
                time.sleep(0.05)
            if not hwnd:
                raise RuntimeError("restarted GUI did not become visible")
            key(hwnd, 0x32)
            time.sleep(0.1)
            wait_quiet(hwnd, user, gdi)
            draft = sp.run(prefix + ["draft", str(repo), "show"], check=True, capture_output=True).stdout.strip().decode()
            if draft != receipt["sustained"]["acknowledged_draft"]:
                raise RuntimeError("controlled restart lost acknowledged draft")
            receipt["controlled_restart"] = {"first_pid": old_pid, "restarted_pid": child.pid, "draft": "PASS: acknowledged GUI draft preserved across controlled close/relaunch", "first_exit_code": 0}
            time.sleep(args.settle_seconds)
        first = sample(child.pid)
        tree_first = tree.accounting() if tree else None
        idle_epoch_us = time.time_ns() // 1000
        begin = time.monotonic()
        samples = []
        while time.monotonic() - begin < args.idle_seconds:
            samples.append(sample(child.pid))
            time.sleep(0.1)
        last = sample(child.pid)
        receipt["idle"] = {"epoch_us": idle_epoch_us, "end_epoch_us": time.time_ns() // 1000, "seconds": time.monotonic() - begin, "first": first, "last": last, "max_sampled_rss": max(t["rss"] for t in samples), "one_core_cpu_percent": (last["cpu_s"] - first["cpu_s"]) / (time.monotonic() - begin) * 100}
        if tree:
            tree_last = tree.accounting()
            receipt["idle"]["tree_one_core_cpu_percent"] = (tree_last["cpu_s"] - tree_first["cpu_s"]) / receipt["idle"]["seconds"] * 100
            user.ShowWindow(hwnd, 6)
            time.sleep(args.settle_seconds)
            minimized_first = tree.accounting()
            minimized_epoch_us = time.time_ns() // 1000
            minimized_begin = time.monotonic()
            while time.monotonic() - minimized_begin < args.idle_seconds:
                time.sleep(0.1)
            minimized_seconds = time.monotonic() - minimized_begin
            minimized_last = tree.accounting()
            receipt["minimized_idle"] = {"epoch_us": minimized_epoch_us, "end_epoch_us": time.time_ns() // 1000, "seconds": minimized_seconds, "tree_one_core_cpu_percent": (minimized_last["cpu_s"] - minimized_first["cpu_s"]) / minimized_seconds * 100}
            user.ShowWindow(hwnd, 9)
        receipt["scope"] = "Native SendInput to desktop feedback; per-panel metadata identifies selected row/diff cursor or focus border and timestamped DXGI frames or GDI readback completion. Feedback timing is not Git action-completion latency. Resource tree claims require complete OS job/handle coverage and the declared fixture/environment."
    except Exception as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
        if hwnd and user.GetForegroundWindow() == hwnd:
            try:
                from PIL import ImageGrab
                bounds = w.RECT()
                user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
                user.GetWindowRect(hwnd, c.byref(bounds))
                ImageGrab.grab(bbox=(bounds.left, bounds.top, bounds.right, bounds.bottom)).save(root / "failure-window.png")
                receipt["failure_capture"] = "failure-window.png"
            except Exception as capture_error:
                receipt["failure_capture_error"] = str(capture_error)
        raise
    finally:
        if hwnd and "error" in receipt:
            try:
                wait_quiet(hwnd, user, gdi, timeout=10)
            except Exception as settle_error:
                receipt["failure_settle_error"] = str(settle_error)
        if hwnd:
            user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
            user.PostMessageW(hwnd, 0x10, 0, 0)
        try:
            child.wait(timeout=10)
        except sp.TimeoutExpired:
            receipt["forced_close"] = True
            child.terminate()
            child.wait(timeout=5)
        monitoring_done.set()
        monitoring.join(timeout=3)
        receipt["resource_trace"] = trace_summary
        receipt["exit_code"] = child.returncode
        harness_last = sample(os.getpid())
        receipt["harness_overhead"] = {"process_cpu_s": harness_last["cpu_s"] - harness_first["cpu_s"], "process_peak_rss": harness_last["peak_rss"], "scope": "Probe process only, including monitoring/input/validation; independent oracle subprocesses are outside this counter and outside the product job"}
        if tree:
            try:
                receipt["process_tree"] = tree.receipt(sample)
                if args.busy_latency:
                    rows = {row["pid"]: row for row in receipt["process_tree"]["processes"]}
                    coverage = {}
                    for name, data in receipt.get("latency_ms", {}).items():
                        overlaps = []
                        for epoch, pids in zip(data["input_epoch_us"], data["active_git_pids"], strict=True):
                            tick = epoch * 10 + 116444736000000000
                            overlaps.append(any(pid in rows and rows[pid]["created_100ns"] <= tick < rows[pid]["exited_100ns"] for pid in pids))
                        coverage[name] = {"samples": len(overlaps), "verified_overlap": sum(overlaps), "all_overlaps": overlaps}
                    complete = receipt["process_tree"]["coverage"] == "COMPLETE" and len(coverage) == 3 and all(data["samples"] == 30 and data["verified_overlap"] == 30 for data in coverage.values())
                    receipt["busy_latency"] = {"coverage": "COMPLETE" if complete else "BLOCKED", "panels": coverage, "method": "Every retained input must overlap a recorded owned Git kernel lifetime; all trials retained, no slow-trial exclusion or automatic retry"}
            finally:
                tree.close()
        if args.measure_ui:
            paths = sorted((root / "profile").glob("timing-*.json"))
            receipt["ui_timing"] = [{"file": path.name, **json.loads(path.read_text(encoding="utf-8"))} for path in paths]
            if not paths:
                receipt["ui_timing_error"] = "BLOCKED: no application timing receipt was exported"
        (root / "receipt.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        if previous:
            user.SetForegroundWindow(previous)
    print(json.dumps({k: v for k, v in receipt.items() if k not in ("latency_ms", "process_tree", "ui_timing")}, indent=2))
    if "ui_timing" in receipt:
        print("UI callback timing:", [{"file": report["file"], "groups": report["groups"]} for report in receipt["ui_timing"]])
    if tree:
        print("process tree:", {k: v for k, v in receipt["process_tree"].items() if k not in ("processes", "method")})
    print("latency p95 ms:", {name: data["p95"] for name, data in receipt.get("latency_ms", {}).items()})


if __name__ == "__main__":
    main()
