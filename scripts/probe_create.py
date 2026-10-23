"""Windows native Create journey; disposable fixtures, no latency claim.

Uses the existing owned-process/input/capture guards. Raw receipts stay under
ignored target/qualification and must not be published with private paths.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess as sp
import time

from probe_workspace import native, native_title, wait_entry, wait_quiet, capture_entry, click_review_button
from windows_job_probe import OwnedTree


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--git", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--owned-topmost", action="store_true", help="raise only the owned functional-test window; no timing claim")
    parser.add_argument("--settings-entry", action="store_true", help="start without Git on PATH, select it in the GUI, edit local identity and verify restart")
    args = parser.parse_args()
    workspace = Path(__file__).resolve().parents[1]
    root = Path(args.out).resolve()
    if not root.is_relative_to(workspace / "target" / "qualification"):
        parser.error("out must be a new directory below this checkout's target/qualification")
    root.mkdir(parents=True, exist_ok=False)
    exe, git = str(Path(args.exe).resolve()), str(Path(args.git).resolve())
    target = root / "created repo 中文"
    target.mkdir()
    expected = {"keep.bin": bytes([0, 255, 13, 10]), "nested/keep.txt": b"unchanged\r\n"}
    (target / "nested").mkdir()
    for name, value in expected.items():
        (target / name).write_bytes(value)
    template = root / "user-template"
    (template / "hooks").mkdir(parents=True)
    (template / "template-canary").write_bytes(b"must not be copied")
    (template / "hooks" / "post-checkout").write_text("#!/bin/sh\nexit 99\n", encoding="utf-8")
    config = root / "user.gitconfig"
    config.write_text('[init]\n\ttemplateDir = "' + template.as_posix() + '"\n\tdefaultBranch = wrong-default\n\tdefaultObjectFormat = sha256\n\tdefaultRefFormat = reftable\n', encoding="utf-8")
    environment = dict(os.environ, GIT_CONFIG_GLOBAL=str(config), GIT_CONFIG_NOSYSTEM="1", GIT_TEMPLATE_DIR=str(template))
    if args.settings_entry:
        empty_path = root / "empty-path"
        empty_path.mkdir()
        environment["PATH"] = str(empty_path)
        for name in ("GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL"):
            environment.pop(name, None)
    for name in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT"):
        environment.pop(name, None)
    global_config_before = config.read_bytes()
    user, gdi, sample, key = native()
    previous = user.GetForegroundWindow()
    tree = OwnedTree()
    receipt = {"verdict": "RUNNING", "artifact_sha256": hashlib.sha256(Path(exe).read_bytes()).hexdigest(),
               "source_head": sp.check_output(["git", "rev-parse", "HEAD"], cwd=workspace, text=True).strip(),
               "os": platform.platform(), "git": sp.check_output([git, "--version"], text=True).strip(),
               "owned_topmost": args.owned_topmost, "latency_status": "NOT-RUN", "resource_status": "NOT-RUN",
               "visual_verdict": "REVIEW-REQUIRED", "cases": [],
               "harness_sha256": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_create.py", "probe_workspace.py", "windows_job_probe.py")}}
    hwnd, child = None, None
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
    def enumerate_window():
        user.EnumWindows(callback, 0)
        if child.poll() is not None:
            raise RuntimeError("product exited before its native window appeared")
        return hwnd is not None
    def title(value, exact=False):
        wait_entry(lambda: native_title(hwnd, user) == value if exact else native_title(hwnd, user).startswith(value), "missing native context: " + value)
        wait_quiet(hwnd, user, gdi)
    allowed_git = 0
    def git_count():
        wait_entry(lambda: tree.accounting()["total_processes"] == len(tree.pids()), "process notifications did not reconcile")
        assert all(pid in tree.images for pid in tree.pids()), "unknown process before creation approval"
        return sum(name.lower() == "git.exe" for name in tree.images.values())
    def zero_git():
        assert git_count() == allowed_git, "unexpected Git before explicit approval"
    def preserved():
        for name, value in expected.items():
            assert (target / name).read_bytes() == value, "existing content changed: " + name
    try:
        with (root / "stderr.log").open("w", encoding="utf-8") as error_log:
            launch = [exe, "--profile", str(root / "profile"), "gui"] if args.settings_entry else [exe, "--git", git, "--profile", str(root / "profile"), "gui"]
            child = tree.spawn(launch, env=environment, stdout=sp.DEVNULL, stderr=error_log)
        wait_entry(enumerate_window, "no owned native window")
        user.ShowWindow(hwnd, 9)
        if args.owned_topmost:
            owner = w.DWORD()
            user.GetWindowThreadProcessId(hwnd, c.byref(owner))
            assert owner.value == child.pid
            user.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
            user.SetWindowPos.restype = w.BOOL
            if not user.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x0043):
                raise RuntimeError("owned-window isolation was refused")
        if args.settings_entry:
            time.sleep(0.2)
            wait_quiet(hwnd, user, gdi)
            key(hwnd, "G")
            title("MiniCon SrcMgr - Git settings | Ready", exact=True)
            zero_git()
            key(hwnd, git)
            capture_entry(hwnd, user, gdi, root, "git-settings-selection.png")
            click_review_button(hwnd, user, gdi, True)
            title("MiniCon SrcMgr - Git settings | Saved")
            capture_entry(hwnd, user, gdi, root, "git-settings-saved.png")
            allowed_git = git_count()
            assert allowed_git > 0, "explicit Test and save did not exercise the selected Git"
            key(hwnd, 0x1B)
            title("MiniCon SrcMgr", exact=True)
            receipt["cases"].append("PASS: no Git on PATH; settings display starts zero Git; explicit pointer Test and save selects installed Git")
        key(hwnd, "N")
        title("MiniCon SrcMgr - Create repository")
        key(hwnd, str(target))
        key(hwnd, 0x09)
        for _ in range(4):
            key(hwnd, 0x08)
        key(hwnd, "feature/start")
        wait_quiet(hwnd, user, gdi)
        capture_entry(hwnd, user, gdi, root, "create-form.png")
        click_review_button(hwnd, user, gdi, True)
        title("MiniCon SrcMgr - Create review | ")
        zero_git()
        assert not (target / ".git").exists()
        capture_entry(hwnd, user, gdi, root, "create-review.png")
        key(hwnd, 0x0D)
        time.sleep(0.15)
        assert native_title(hwnd, user).startswith("MiniCon SrcMgr - Create review | ")
        assert not (target / ".git").exists(), "opening Enter initialized without approval"
        click_review_button(hwnd, user, gdi, False)
        title("MiniCon SrcMgr - Create repository")
        zero_git()
        preserved()
        assert not (target / ".git").exists(), "Cancel initialized the target"
        receipt["cases"].append("PASS: filesystem-only preview; pointer Back preserves contents/input; Enter cannot approve")
        key(hwnd, 0x0D)
        title("MiniCon SrcMgr - Create review | ")
        click_review_button(hwnd, user, gdi, True)
        title("MiniCon SrcMgr - Trust review | ")
        preserved()
        assert (target / ".git/HEAD").read_bytes() == b"ref: refs/heads/feature/start\n"
        assert not (target / ".git/index").exists()
        assert not (target / ".git/hooks").exists()
        assert not (target / ".git/template-canary").exists()
        assert not (root / "profile/trust.v1").exists(), "init silently granted repository trust"
        capture_entry(hwnd, user, gdi, root, "create-trust.png")
        key(hwnd, 0x0D)
        time.sleep(0.15)
        assert not (root / "profile/trust.v1").exists(), "held Enter approved trust"
        click_review_button(hwnd, user, gdi, False)
        title("MiniCon SrcMgr", exact=True)
        assert not (root / "profile/trust.v1").exists()
        assert (target / ".git/HEAD").exists(), "trust cancellation must not undo authorized init"
        key(hwnd, 0x0D)
        title("MiniCon SrcMgr - Trust review | ")
        click_review_button(hwnd, user, gdi, True)
        wait_entry(lambda: " | " in native_title(hwnd, user) and "Trust review" not in native_title(hwnd, user), "created repository did not open")
        wait_quiet(hwnd, user, gdi)
        capture_entry(hwnd, user, gdi, root, "create-opened.png")
        oracle = [git, "-C", str(target)]
        assert sp.check_output(oracle + ["symbolic-ref", "HEAD"], env=environment).strip() == b"refs/heads/feature/start"
        assert sp.check_output(oracle + ["ls-files", "-z"], env=environment) == b""
        assert sp.check_output(oracle + ["rev-parse", "--show-object-format"], env=environment).strip() == b"sha1"
        assert (target / ".git/refs").is_dir() and not (target / ".git/reftable").exists()
        assert sp.run(oracle + ["rev-parse", "--verify", "HEAD"], env=environment, capture_output=True).returncode != 0
        preserved()
        receipt["cases"].append("PASS: explicit init uses selected branch and empty template; no stage/commit; separate pointer trust opens unborn repo; trust Cancel leaves created content intact")
        if args.settings_entry:
            original_config = (target / ".git/config").read_bytes()
            key(hwnd, "I")
            title("MiniCon SrcMgr - Identity settings | Ready", exact=True)
            capture_entry(hwnd, user, gdi, root, "identity-missing.png")
            assert (target / ".git/config").read_bytes() == original_config
            key(hwnd, "Native Identity 中文")
            key(hwnd, 0x09)
            key(hwnd, "native@example.invalid")
            click_review_button(hwnd, user, gdi, True)
            title("MiniCon SrcMgr - Trust review | ")
            capture_entry(hwnd, user, gdi, root, "identity-saved-review.png")
            assert sp.check_output(oracle + ["config", "--local", "--get", "user.name"], env=environment).strip() == "Native Identity 中文".encode()
            assert sp.check_output(oracle + ["config", "--local", "--get", "user.email"], env=environment).strip() == b"native@example.invalid"
            assert config.read_bytes() == global_config_before, "local identity save changed global config"
            assert not (target / ".git/config.lock").exists()
            assert not (target / ".git/index").exists()
            preserved()
            key(hwnd, 0x0D)
            time.sleep(0.15)
            assert native_title(hwnd, user).startswith("MiniCon SrcMgr - Trust review | "), "identity Save Enter approved changed config trust"
            click_review_button(hwnd, user, gdi, True)
            wait_entry(lambda: " | " in native_title(hwnd, user) and "Trust review" not in native_title(hwnd, user), "updated identity repository did not reopen")
            wait_quiet(hwnd, user, gdi)
            key(hwnd, 0x20)  # Deliberately stage only the selected keep.bin.
            wait_entry(lambda: sp.check_output(oracle + ["ls-files", "-z"], env=environment) == b"keep.bin\0", "named file did not reach index")
            key(hwnd, 0x43)
            key(hwnd, "Native identity qualification")
            key(hwnd, 0x0D, ctrl=True)
            def committed():
                return sp.run(oracle + ["rev-parse", "--verify", "HEAD"], env=environment, capture_output=True).returncode == 0
            wait_entry(committed, "first commit with configured identity did not complete")
            wait_quiet(hwnd, user, gdi)
            identities = sp.check_output(oracle + ["show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce", "HEAD"], env=environment).strip().split(b"\0")
            assert identities == ["Native Identity 中文".encode(), b"native@example.invalid"] * 2
            assert sp.check_output(oracle + ["ls-tree", "-r", "--name-only", "HEAD"], env=environment).strip() == b"keep.bin"
            preserved()
            capture_entry(hwnd, user, gdi, root, "identity-first-commit.png")
            receipt["cases"].append("PASS: explicit local identity save preserves global config/files/index; separate trust; deliberate first commit has exact author/committer and selected tree")
            user.PostMessageW(hwnd, 0x10, 0, 0)
            child.wait(timeout=10)
            assert child.returncode == 0
            receipt["first_exit_code"] = child.returncode
            hwnd = None
            with (root / "restart-stderr.log").open("w", encoding="utf-8") as error_log:
                child = tree.spawn([exe, "--profile", str(root / "profile"), "gui"], env=environment, stdout=sp.DEVNULL, stderr=error_log)
            wait_entry(enumerate_window, "no restarted native window")
            user.ShowWindow(hwnd, 9)
            if args.owned_topmost:
                owner = w.DWORD()
                user.GetWindowThreadProcessId(hwnd, c.byref(owner))
                assert owner.value == child.pid
                if not user.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x0043):
                    raise RuntimeError("restarted owned-window isolation was refused")
            time.sleep(0.2)
            key(hwnd, 0x4F, ctrl=True)
            key(hwnd, str(target))
            key(hwnd, 0x0D)
            wait_entry(lambda: " | " in native_title(hwnd, user) and "Trust review" not in native_title(hwnd, user), "saved Git preference did not work after restart without PATH Git or --git")
            wait_quiet(hwnd, user, gdi)
            capture_entry(hwnd, user, gdi, root, "settings-restarted.png")
            assert config.read_bytes() == global_config_before
            receipt["cases"].append("PASS: native restart without --git and with empty PATH reopens using the persisted selection and local identity")
        receipt["verdict"] = "PASS"
    except Exception as error:
        receipt["verdict"], receipt["failure"] = "FAIL", str(error)
        raise
    finally:
        if hwnd:
            user.PostMessageW(hwnd, 0x10, 0, 0)
        if child:
            try:
                child.wait(timeout=10)
            except sp.TimeoutExpired:
                receipt["forced_close"] = True
                child.terminate()
                child.wait(timeout=5)
            receipt["exit_code"] = child.returncode
        try:
            receipt["process_tree"] = tree.receipt(sample)
            if receipt["process_tree"]["coverage"] != "COMPLETE" or receipt["process_tree"]["accounting"]["active_processes"] != 0:
                receipt["process_evidence_status"] = "BLOCKED: missing complete terminal descendant evidence"
                if receipt["verdict"] != "FAIL":
                    receipt["verdict"], receipt["failure"] = "BLOCKED", "missing complete terminal descendant evidence"
        except Exception as error:
            receipt["process_error"] = str(error)
            if receipt["verdict"] != "FAIL":
                receipt["verdict"] = "BLOCKED"
        finally:
            tree.close()
        if child and (child.returncode != 0 or receipt.get("forced_close")):
            receipt["verdict"], receipt["failure"] = "FAIL", "abnormal product exit"
        (root / "receipt.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        if previous:
            user.SetForegroundWindow(previous)
    assert receipt["verdict"] == "PASS", receipt.get("failure", receipt.get("process_error"))
    print(json.dumps({key: value for key, value in receipt.items() if key not in ("process_tree", "harness_sha256")}, indent=2))


if __name__ == "__main__":
    main()
