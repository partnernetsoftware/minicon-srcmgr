"""Scoped Windows commit journey with independent Git/SQLite oracles.

Owned disposable repositories only. No latency, resource, clipboard or IME claim.
Every attempt gets a new ignored output directory, including failed attempts.
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
    repo, profile = root / "repo 中文", root / "profile"
    repo.mkdir()
    config = root / "global.gitconfig"
    config.write_text("[user]\nname = Fixture\nemail = fixture@example.invalid\n[core]\nautocrlf = false\n", encoding="utf-8")
    env = dict(os.environ, GIT_CONFIG_GLOBAL=str(config), GIT_CONFIG_NOSYSTEM="1")
    for name in tuple(env):
        if name.startswith(("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_", "GIT_ALTERNATE_", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT", "GIT_AUTHOR_", "GIT_COMMITTER_")):
            env.pop(name)
    def run(*arguments):
        return sp.run([git, "-C", str(repo), *arguments], env=env, check=True, capture_output=True).stdout
    run("init", "-q", "-b", "main")
    (repo / "a.txt").write_bytes(b"original\n")
    run("add", "a.txt")
    run("commit", "-qm", "initial")
    before = run("rev-parse", "HEAD")
    staged = {"a.txt": b"staged\n", "hidden.bin": bytes([0, 255, 1]), "--hidden 中文.txt": b"unicode path\n"}
    for name, content in staged.items():
        (repo / name).write_bytes(content)
    run("add", "--", *staged)
    unstaged = b"unstaged bytes must survive\n"
    (repo / "a.txt").write_bytes(unstaged)
    sp.run([exe, "--git", git, "--profile", str(profile), "trust", "grant", str(repo)], env=env, check=True, capture_output=True)
    def draft():
        with sqlite3.connect((profile / "profile.sqlite3").as_uri() + "?mode=ro", uri=True) as db:
            rows = db.execute("SELECT body FROM drafts WHERE backend='git'").fetchall()
        return rows[0][0] if rows else None
    user, gdi, sample, key = native()
    previous = user.GetForegroundWindow()
    tree = OwnedTree()
    child, hwnd = None, None
    receipt = dict(verdict="RUNNING", artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),
                   source_head=sp.check_output(["git", "rev-parse", "HEAD"], cwd=workspace, text=True).strip(),
                   os=platform.platform(), git=run("--version").decode().strip(), owned_topmost=args.owned_topmost,
                   latency_status="NOT-RUN", resource_status="NOT-RUN", visual_verdict="REVIEW-REQUIRED", cases=[],
                   harness_sha256={name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_commit.py", "probe_workspace.py", "windows_job_probe.py")})
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
    def title(value):
        wait_entry(lambda: native_title(hwnd, user) == value, "missing context: " + value)
        wait_quiet(hwnd, user, gdi)
    def launch():
        nonlocal child, hwnd
        hwnd = None
        with (root / "stderr.log").open("a", encoding="utf-8") as log:
            child = tree.spawn([exe, "--git", git, "--profile", str(profile), "gui", str(repo)], env=env, stdout=sp.DEVNULL, stderr=log)
        wait_entry(find_window, "no owned native window")
        user.ShowWindow(hwnd, 9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
            if not user.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x0043):
                raise RuntimeError("owned-window isolation refused")
        wait_entry(lambda: " | " in native_title(hwnd, user), "repository startup did not complete")
        key(hwnd, 0x43)
        title("MiniCon SrcMgr - Commit | Review ready")
    try:
        launch()
        capture_entry(hwnd, user, gdi, root, "complete-index.png")
        message = "Native review 中文"
        key(hwnd, message)
        key(hwnd, 0x0D)
        key(hwnd, "body retained through refusal")
        message += "\nbody retained through refusal"
        wait_entry(lambda: draft() == message, "continuous editor input not durably acknowledged")
        # An external writer widens the live index after the displayed review.
        staged["external.txt"] = b"external staged content\n"
        (repo / "external.txt").write_bytes(staged["external.txt"])
        run("add", "external.txt")
        key(hwnd, 0x0D, ctrl=True)
        title("MiniCon SrcMgr - Commit | Review blocked")
        assert run("rev-parse", "HEAD") == before
        assert draft() == message
        capture_entry(hwnd, user, gdi, root, "stale-index-refused.png")
        receipt["cases"].append("PASS: stale full-index submission leaves HEAD unchanged and exact multiline Unicode draft durable")
        key(hwnd, 0x74)  # F5 explicitly renews review.
        title("MiniCon SrcMgr - Commit | Review ready")
        capture_entry(hwnd, user, gdi, root, "refreshed-four-paths.png")
        # In this form the only button with a blue border is the ready Commit
        # control. The helper observes its actual rendered pixels and checks
        # foreground/ownership immediately before the native pointer click.
        click_review_button(hwnd, user, gdi, False)
        wait_entry(lambda: run("rev-parse", "HEAD") != before, "native pointer commit did not update HEAD")
        wait_entry(lambda: not native_title(hwnd, user).startswith("MiniCon SrcMgr - Commit |"), "observed commit did not close composer")
        for name, content in staged.items():
            assert run("show", "HEAD:" + name) == content, name
        assert (repo / "a.txt").read_bytes() == unstaged
        assert not run("diff", "--cached", "--name-only")
        assert run("log", "-1", "--format=%B").decode().strip() == message
        assert draft() is None
        receipt["observed_oid"] = run("rev-parse", "HEAD").decode().strip()
        capture_entry(hwnd, user, gdi, root, "commit-observed.png")
        receipt["cases"].append("PASS: explicit refreshed pointer commit has exact four-path tree/message; unstaged bytes survive; draft clears after observed success")
        # Close with unsaved text, then reopen through a new native process.
        key(hwnd, 0x43)
        title("MiniCon SrcMgr - Commit | Review blocked")  # Empty index.
        recovered = "close must preserve latest 中文"
        key(hwnd, recovered)
        capture_entry(hwnd, user, gdi, root, "before-draft-close.png")
        user.PostMessageW(hwnd, 0x10, 0, 0)
        child.wait(timeout=15)
        assert child.returncode == 0 and draft() == recovered
        receipt["first_exit_code"] = child.returncode
        # Restage an explicit fixture path so the second review is usable.
        run("add", "a.txt")
        launch()
        capture_entry(hwnd, user, gdi, root, "recovered-draft.png")
        key(hwnd, " latest")
        capture_entry(hwnd, user, gdi, root, "before-second-close.png")
        user.PostMessageW(hwnd, 0x10, 0, 0)
        child.wait(timeout=15)
        assert child.returncode == 0 and draft() == recovered + " latest"
        receipt["cases"].append("PASS: native close waits for latest draft; next process restores it and saves the next edit")
        receipt["verdict"] = "PASS"
    except Exception as error:
        receipt["verdict"], receipt["failure"] = "FAIL", str(error)
        if hwnd and child and child.poll() is None:
            try:
                capture_entry(hwnd, user, gdi, root, "failure-window.png")
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
