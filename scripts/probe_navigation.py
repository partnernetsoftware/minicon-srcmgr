"""Windows history-return and saved-view recovery journey on owned fixtures.

Actual Git/SQLite and retained native captures are independent oracles. No
latency/resource/IME/clipboard qualification follows from this functional run.
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

from probe_workspace import native, native_title, wait_entry, wait_quiet, capture_entry, click_review_button
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
    left, right = root / "history 中文", root / "other repo"
    def run(repo, *arguments):
        return sp.run([git, "-C", str(repo), *arguments], env=env, check=True, capture_output=True).stdout
    for repo in (left, right):
        repo.mkdir()
        run(repo, "init", "-q", "-b", "main")
        run(repo, "config", "user.name", "Fixture")
        run(repo, "config", "user.email", "fixture@example.invalid")
        (repo / "a.txt").write_bytes(b"initial\n")
        run(repo, "add", ".")
        run(repo, "commit", "-qm", "initial")
    for i in range(30):
        name = f"f{i:02}.txt" if i != 29 else "f29 中文.txt"
        (left / name).write_bytes(b"".join(f"history line {n}\n".encode() for n in range(100)))
    run(left, "add", ".")
    run(left, "commit", "-qm", "navigation target\n\n" + "".join(f"message line {n}\n" for n in range(80)))
    (left / "a.txt").write_bytes(b"uncommitted bytes survive\n")
    for repo in (left, right):
        sp.run([exe, "--git", git, "--profile", str(profile), "trust", "grant", str(repo)], env=env, check=True, capture_output=True)
    oid = run(left, "rev-parse", "HEAD").decode().strip()
    before_refs = [(run(p, "rev-parse", "HEAD"), run(p, "write-tree")) for p in (left, right)]
    before_files = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for repo in (left, right) for p in repo.iterdir() if p.is_file()}
    def rows(sql, parameters=()):
        with sqlite3.connect((profile / "profile.sqlite3").as_uri() + "?mode=ro", uri=True) as db:
            return db.execute(sql, parameters).fetchall()
    def write(sql, parameters):
        with sqlite3.connect(profile / "profile.sqlite3") as db:
            db.execute(sql, parameters)
    user, gdi, sample, key = native()
    previous = user.GetForegroundWindow()
    tree = OwnedTree()
    child, hwnd = None, None
    receipt = dict(verdict="RUNNING", artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),
                   os=platform.platform(), git=run(left, "--version").decode().strip(), owned_topmost=args.owned_topmost,
                   latency_status="NOT-RUN", resource_status="NOT-RUN", visual_verdict="REVIEW-REQUIRED", cases=[], exits=[],
                   harness_sha256={name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_navigation.py", "probe_workspace.py", "windows_job_probe.py")})
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
    def capture(name):
        capture_entry(hwnd, user, gdi, root, name + ".png")
    def opened(repo):
        prefix = f"MiniCon SrcMgr - {repo.name} | "
        wait_entry(lambda: native_title(hwnd, user).startswith(prefix) and str(repo).casefold() in native_title(hwnd, user).casefold(), "repository did not activate")
        wait_quiet(hwnd, user, gdi)
    def history(kind):
        expected = f"MiniCon SrcMgr - History | {kind} | Ready | {oid[:12]} | "
        wait_entry(lambda: native_title(hwnd, user).startswith(expected), "history view was not ready: " + kind)
        wait_quiet(hwnd, user, gdi)
    def recovery():
        wait_entry(lambda: native_title(hwnd, user).startswith("MiniCon SrcMgr - Saved view recovery | Ready | "), "saved view recovery did not appear")
        wait_quiet(hwnd, user, gdi)
    def launch(repo):
        nonlocal child, hwnd
        hwnd = None
        with (root / "stderr.log").open("a", encoding="utf-8") as log:
            child = tree.spawn([exe, "--git", git, "--profile", str(profile), "gui", str(repo)], env=env, stdout=sp.DEVNULL, stderr=log)
        wait_entry(find_window, "no owned native window")
        user.ShowWindow(hwnd, 9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
            assert user.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x0043), "owned-window isolation refused"
    def close():
        wait_quiet(hwnd, user, gdi)
        user.PostMessageW(hwnd, 0x10, 0, 0)
        child.wait(timeout=15)
        receipt["exits"].append(child.returncode)
        assert child.returncode == 0
    def viewport(name):
        from PIL import Image
        image = Image.open(root / (name + ".png")).convert("RGB")
        # Analyze retained pixels only; no derived/edited capture replaces them.
        bounds = (round(image.width * .42), round(image.height * .27), image.width - 25, round(image.height * .86))
        return hashlib.sha256(image.crop(bounds).tobytes()).hexdigest()
    def back_click():
        rect = w.RECT()
        user.GetClientRect(hwnd, c.byref(rect))
        scale = user.GetDpiForWindow(hwnd) / 96
        dc = user.GetDC(hwnd)
        try:
            x = round(rect.right * .6)
            ys = [y for y in range(round(60 * scale), rect.bottom // 4) if all(gdi.GetPixel(dc, x + dx, y) == 0xD69C56 for dx in range(-15, 16))]
        finally:
            user.ReleaseDC(hwnd, dc)
        assert ys, "rendered history frame is unavailable"
        point = w.POINT(rect.right - round(50 * scale), ys[0] + round(10 * scale))
        assert user.ClientToScreen(hwnd, c.byref(point))
        user.WindowFromPoint.argtypes, user.WindowFromPoint.restype = [w.POINT], w.HWND
        assert user.GetForegroundWindow() == hwnd and user.WindowFromPoint(point) == hwnd
        user.SetCursorPos(point.x, point.y)
        actual = w.POINT()
        assert user.GetCursorPos(c.byref(actual)) and (actual.x, actual.y) == (point.x, point.y)
        assert user.GetForegroundWindow() == hwnd and user.WindowFromPoint(actual) == hwnd
        user.mouse_event(2, 0, 0, 0, 0)
        user.mouse_event(4, 0, 0, 0, 0)
    try:
        # Bootstrap one genuine saved record and draft before corrupting only
        # that owned record, rather than synthesizing native-path key spelling.
        launch(right)
        opened(right)
        key(hwnd, 0x43)
        wait_entry(lambda: native_title(hwnd, user) == "MiniCon SrcMgr - Commit | Review blocked", "empty-index composer not ready")
        key(hwnd, "right draft retained 中文")
        capture("right-draft")
        key(hwnd, 0x1B)
        opened(right)
        capture("right-bootstrap")
        close()
        nav_rows = rows("SELECT key,value FROM settings WHERE key LIKE 'git.navigation.v1.%'")
        assert len(nav_rows) == 1
        right_key = nav_rows[0][0]
        invalid = "future navigation version; keep this exact value"
        write("UPDATE settings SET value=?1 WHERE key=?2", (invalid, right_key))
        launch(left)
        opened(left)
        key(hwnd, 0x34)
        key(hwnd, "/")
        key(hwnd, "navigation target")
        wait_entry(lambda: native_title(hwnd, user) == "MiniCon SrcMgr - Filter | Commits | 1 matches", "history query not applied")
        key(hwnd, 0x0D)
        history("Commit")
        key(hwnd, 0x0D)
        history("Files")
        key(hwnd, 0x23)
        capture("files-before-diff")
        key(hwnd, 0x0D)
        history("File Diff")
        for _ in range(4):
            key(hwnd, 0x22)
        capture("diff-before-departure")
        key(hwnd, 0x4F, ctrl=True)
        key(hwnd, str(right))
        key(hwnd, 0x0D)
        recovery()
        capture("saved-view-recovery")
        key(hwnd, 0x0D)  # Enter must not authorize a reset.
        recovery()
        assert rows("SELECT value FROM settings WHERE key=?1", (right_key,)) == [(invalid,)]
        click_review_button(hwnd, user, gdi, False)
        capture("recovery-cancelled")
        assert rows("SELECT value FROM settings WHERE key=?1", (right_key,)) == [(invalid,)]
        key(hwnd, 0x0D)  # Retry the retained Open input, for a fresh review.
        recovery()
        click_review_button(hwnd, user, gdi, True)
        opened(right)
        capture("default-view-opened")
        assert rows("SELECT value FROM settings WHERE key LIKE 'git.navigation.archive.v1.%'") == [(invalid,)]
        assert rows("SELECT body FROM drafts WHERE backend='git'") == [("right draft retained 中文",)]
        receipt["cases"].append("PASS: Enter/Cancel preserve the invalid view; deliberate pointer archive opens defaults, retains the exact archive and draft, and does not mutate Git")
        key(hwnd, 0x09, ctrl=True)
        history("File Diff")
        capture("diff-after-tab-return")
        assert viewport("diff-before-departure") == viewport("diff-after-tab-return"), "tab return did not restore the visible Diff viewport"
        close()
        launch(left)
        history("File Diff")
        capture("diff-after-process-restart")
        assert viewport("diff-before-departure") == viewport("diff-after-process-restart"), "process restart did not restore the visible Diff viewport"
        back_click()
        history("Files")
        capture("files-after-back")
        assert viewport("files-before-diff") == viewport("files-after-back"), "Back did not restore the selected committed file and viewport"
        key(hwnd, 0x1B)
        history("Commit")
        capture("commit-after-back")
        key(hwnd, "/")
        wait_entry(lambda: native_title(hwnd, user) == "MiniCon SrcMgr - Filter | Commits | 1 matches", "Back cleared the history query")
        capture("query-after-back")
        key(hwnd, 0x1B)
        capture("back-query-retained")
        close()
        assert [(run(p, "rev-parse", "HEAD"), run(p, "write-tree")) for p in (left, right)] == before_refs
        after_files = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for repo in (left, right) for p in repo.iterdir() if p.is_file()}
        assert after_files == before_files
        receipt["cases"].append("PASS: tab return and new native process restore the same committed Diff viewport; pointer/keyboard Back reread both prior levels and retain the query; refs/index/source bytes remain exact")
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
