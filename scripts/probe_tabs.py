"""Scoped Windows repository-tab journey; owned fixtures and real Git/SQLite.

Functional evidence only. Always use a new ignored qualification directory.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess as sp

from probe_workspace import native, native_title, wait_entry, wait_quiet, capture_entry
from windows_job_probe import OwnedTree


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("exe", "git", "out"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--owned-topmost", action="store_true")
    args = parser.parse_args()
    workspace = Path(__file__).resolve().parents[1]
    root = Path(args.out).resolve()
    if not root.is_relative_to(workspace / "target/qualification"):
        parser.error("out must be a new directory below target/qualification")
    root.mkdir(parents=True, exist_ok=False)
    exe, git = str(Path(args.exe).resolve()), str(Path(args.git).resolve())
    profile = root / "profile"
    config = root / "global.gitconfig"
    config.write_bytes(b"")
    env = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=str(config), GIT_CONFIG_NOSYSTEM="1")
    def run(repo, *arguments):
        return sp.run([git, "-C", str(repo), *arguments], env=env, check=True, capture_output=True).stdout
    original = b"".join(f"original {i}\n".encode() for i in range(100))
    changed = b"".join(f"changed {i}\n".encode() for i in range(100))
    left, right = root / "left", root / "right 中文"
    for repo in (left, right):
        repo.mkdir()
        run(repo, "init", "-q", "-b", "main")
        run(repo, "config", "user.name", "Fixture")
        run(repo, "config", "user.email", "fixture@example.invalid")
        for name in ("a.txt", "b.txt"):
            (repo / name).write_bytes(original)
        run(repo, "add", ".")
        run(repo, "commit", "-qm", "fixture")
        for name in ("a.txt", "b.txt"):
            (repo / name).write_bytes(changed)
        sp.run([exe, "--git", git, "--profile", str(profile), "trust", "grant", str(repo)], env=env, check=True, capture_output=True)
    heads = [run(p, "rev-parse", "HEAD") for p in (left, right)]
    def records(table, where="1"):
        with sqlite3.connect((profile / "profile.sqlite3").as_uri() + "?mode=ro", uri=True) as db:
            return db.execute(f"SELECT * FROM {table} WHERE {where}").fetchall()
    user, gdi, sample, key = native()
    previous = user.GetForegroundWindow()
    tree = OwnedTree()
    child, hwnd = None, None
    receipt = dict(verdict="RUNNING", artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),
                   os=platform.platform(), git=run(left, "--version").decode().strip(), owned_topmost=args.owned_topmost,
                   latency_status="NOT-RUN", resource_status="NOT-RUN", visual_verdict="REVIEW-REQUIRED", cases=[],
                   harness_sha256={name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_tabs.py", "probe_workspace.py", "windows_job_probe.py")})
    callback_type = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    def enum(window, _):
        nonlocal hwnd
        owner = w.DWORD()
        user.GetWindowThreadProcessId(window, c.byref(owner))
        if owner.value == child.pid and user.IsWindowVisible(window):
            hwnd = window
        return True
    callback = callback_type(enum)
    user.EnumWindows.argtypes = [callback_type, w.LPARAM]
    def find_window():
        user.EnumWindows(callback, 0)
        if child.poll() is not None:
            raise RuntimeError("product exited before native entry")
        return hwnd is not None
    def opened(repo):
        wait_entry(lambda: str(repo).casefold() in native_title(hwnd, user).casefold(), "repository did not activate")
        wait_quiet(hwnd, user, gdi)
    def title(value):
        wait_entry(lambda: native_title(hwnd, user) == value, "missing native context: " + value)
        wait_quiet(hwnd, user, gdi)
    def capture(name):
        capture_entry(hwnd, user, gdi, root, name + ".png")
    def first_tab_click():
        # Observe the first rendered tab's grey border, above the content
        # panels. Never infer a successful switch from these pixels alone.
        rect = w.RECT()
        user.GetClientRect(hwnd, c.byref(rect))
        dc = user.GetDC(hwnd)
        found = None
        try:
            scale = user.GetDpiForWindow(hwnd) / 96
            for y in range(round(25 * scale), round(65 * scale)):
                start = None
                for x in range(10, rect.right // 2):
                    grey = gdi.GetPixel(dc, x, y) == 0x3C3C3C
                    if grey and start is None:
                        start = x
                    if not grey and start is not None:
                        if x - start > 100:
                            found = (start + x) // 2, y + round(5 * scale)
                            break
                        start = None
                if found:
                    break
        finally:
            user.ReleaseDC(hwnd, dc)
        assert found, "no rendered first-tab border"
        point = w.POINT(*found)
        assert user.ClientToScreen(hwnd, c.byref(point))
        user.WindowFromPoint.argtypes, user.WindowFromPoint.restype = [w.POINT], w.HWND
        assert user.GetForegroundWindow() == hwnd and user.WindowFromPoint(point) == hwnd, "tab input ownership refused"
        user.SetCursorPos(point.x, point.y)
        actual = w.POINT()
        assert user.GetCursorPos(c.byref(actual)) and (actual.x, actual.y) == (point.x, point.y)
        assert user.GetForegroundWindow() == hwnd and user.WindowFromPoint(actual) == hwnd
        user.mouse_event(2, 0, 0, 0, 0)
        user.mouse_event(4, 0, 0, 0, 0)
    try:
        with (root / "stderr.log").open("w", encoding="utf-8") as log:
            child = tree.spawn([exe, "--git", git, "--profile", str(profile), "gui", str(left)], env=env, stdout=sp.DEVNULL, stderr=log)
        wait_entry(find_window, "no owned native window")
        user.ShowWindow(hwnd, 9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
            assert user.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x0043)
        opened(left)
        key(hwnd, 0x28)  # Second raw path, b.txt.
        capture("left-selected")
        wait_quiet(hwnd, user, gdi)
        key(hwnd, 0x0D)
        for _ in range(4):
            key(hwnd, 0x22)
        capture("left-scrolled")
        key(hwnd, 0x43)
        title("MiniCon SrcMgr - Commit | Review blocked")
        key(hwnd, "left draft 中文")
        capture("left-draft")
        key(hwnd, 0x1B)
        opened(left)
        key(hwnd, 0x4F, ctrl=True)
        key(hwnd, str(right))
        key(hwnd, 0x0D)
        opened(right)
        key(hwnd, 0x33)
        key(hwnd, 0x43)
        title("MiniCon SrcMgr - Commit | Review blocked")
        key(hwnd, "right draft 中文")
        capture("two-tabs-right-draft")
        first_tab_click()
        opened(left)
        capture("left-restored")
        nav = records("settings", "key LIKE 'git.navigation.v1.%'")
        assert len(nav) == 2
        positions = [tuple(map(int, value.split("\n")[8:10])) for _, value in nav]
        assert any(cursor >= 40 and scroll > 0 for cursor, scroll in positions), "scrolled context was not durable"
        assert {row[2] for row in records("drafts")} == {"left draft 中文", "right draft 中文"}
        receipt["cases"].append("PASS: native pointer tab switch waits for draft; independent multiline-path contexts and scrolled selection are durable")
        key(hwnd, 0x32)
        wait_quiet(hwnd, user, gdi)
        key(hwnd, 0x20)
        wait_entry(lambda: run(left, "show", ":b.txt") == changed, "selected left b.txt was not staged")
        assert run(right, "show", ":b.txt") == original
        assert run(left, "show", ":a.txt") == original
        capture("left-staged")
        wait_quiet(hwnd, user, gdi)
        key(hwnd, 0x57, ctrl=True)
        opened(right)
        capture("left-closed-right-restored")
        key(hwnd, 0x57, ctrl=True)
        title("MiniCon SrcMgr")
        capture("last-tab-closed")
        key(hwnd, 0x4F, ctrl=True)
        key(hwnd, str(left))
        key(hwnd, 0x0D)
        opened(left)
        capture("reopened-left")
        assert [run(p, "rev-parse", "HEAD") for p in (left, right)] == heads
        for repo in (left, right):
            assert (repo / "a.txt").read_bytes() == changed and (repo / "b.txt").read_bytes() == changed
        receipt["cases"].append("PASS: restored selection stages only left b.txt; Ctrl+W closes each active tab; last close returns Welcome; deliberate reopen retains draft/context")
        user.PostMessageW(hwnd, 0x10, 0, 0)
        child.wait(timeout=15)
        assert child.returncode == 0
        receipt["verdict"] = "PASS"
    except Exception as error:
        receipt["verdict"], receipt["failure"] = "FAIL", str(error)
        if hwnd and child and child.poll() is None:
            try:
                capture("failure-window")
            except Exception as capture_error:
                receipt["capture_error"] = str(capture_error)
        raise
    finally:
        if child and child.poll() is None:
            user.PostMessageW(hwnd, 0x10, 0, 0)
            try:
                child.wait(timeout=15)
            except sp.TimeoutExpired:
                receipt["forced_close"] = True
                child.terminate()
                child.wait(timeout=5)
        receipt["exit_code"] = child.returncode if child else None
        try:
            receipt["process_tree"] = tree.receipt(sample)
            evidence = receipt["process_tree"]
            if evidence["coverage"] != "COMPLETE" or evidence["accounting"]["active_processes"]:
                receipt["process_evidence_status"] = "BLOCKED"
                if receipt["verdict"] != "FAIL":
                    receipt["verdict"] = "BLOCKED"
        except Exception as error:
            receipt["process_error"] = str(error)
            if receipt["verdict"] != "FAIL":
                receipt["verdict"] = "BLOCKED"
        tree.close()
        if receipt.get("forced_close") or child and child.returncode != 0:
            receipt["verdict"] = "FAIL"
        (root / "receipt.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        if previous:
            user.SetForegroundWindow(previous)
    assert receipt["verdict"] == "PASS", receipt.get("failure", receipt.get("process_error"))
    print(json.dumps({name: value for name, value in receipt.items() if name not in ("process_tree", "harness_sha256")}))


if __name__ == "__main__":
    main()
