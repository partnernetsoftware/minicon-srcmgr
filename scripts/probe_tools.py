"""Windows editor/terminal handoff on owned disposable repositories.

Real Notepad content and a new Windows Terminal's cwd are external oracles.
The product-owned process tree and separately observed user-tool windows have
different lifetime boundaries. This run measures no latency/resources/AX/IME.
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
    for name in ("exe", "git", "editor", "terminal", "out"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--owned-topmost", action="store_true")
    args = parser.parse_args()
    workspace = Path(__file__).resolve().parents[1]
    root = Path(args.out).resolve()
    if not root.is_relative_to(workspace / "target/qualification"):
        parser.error("out must be a new directory below target/qualification")
    root.mkdir(parents=True, exist_ok=False)
    exe, git, editor, terminal = [str(Path(value).resolve()) for value in (args.exe, args.git, args.editor, args.terminal)]
    profile = root / "profile"
    config = root / "global.gitconfig"
    config.write_bytes(b"")
    env = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=str(config), GIT_CONFIG_NOSYSTEM="1")
    left, right = root / "editor project 中文", root / "terminal project 中文"
    filename = root.name + " 中文.txt"
    def run(repo, *arguments):
        return sp.run([git, "-C", str(repo), *arguments], env=env, check=True, capture_output=True).stdout
    def product(*arguments):
        return sp.run([exe, "--git", git, "--profile", str(profile), *arguments], env=env, check=True, capture_output=True).stdout
    for index, repo in enumerate((left, right)):
        repo.mkdir()
        run(repo, "init", "-q", "-b", "main")
        run(repo, "config", "user.name", "Fixture")
        run(repo, "config", "user.email", "fixture@example.invalid")
        (repo / filename).write_bytes(b"original\n")
        run(repo, "add", ".")
        run(repo, "commit", "-qm", "initial")
        (repo / filename).write_text(f"Owned {'editor' if index == 0 else 'terminal'} source 中文\n", encoding="utf-8")
        product("trust", "grant", str(repo))
        product("draft", str(repo), "save", "-m", f"retained draft {index} 中文")
    def state():
        return [(run(repo, "rev-parse", "HEAD").hex(), run(repo, "write-tree").hex(), hashlib.sha256((repo / filename).read_bytes()).hexdigest()) for repo in (left, right)]
    before = state()
    def rows(sql):
        with sqlite3.connect(profile / "profile.sqlite3") as db:
            return db.execute(sql).fetchall()
    drafts = rows("SELECT * FROM drafts ORDER BY repository")
    user, gdi, sample, key = native()
    previous = user.GetForegroundWindow()
    tree = OwnedTree()
    child, hwnd = None, None
    external = []
    receipt = dict(verdict="RUNNING", artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(), os=platform.platform(),
                   git=run(left, "--version").decode().strip(), owned_topmost=args.owned_topmost,
                   latency_status="NOT-RUN", resource_status="NOT-RUN", visual_verdict="REVIEW-REQUIRED", cases=[], exits=[],
                   external_tools=external, external_tree_coverage="NOT-CLAIMED; independent user tools observed by new window, PID and actual content/cwd",
                   harness_sha256={name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_tools.py", "probe_workspace.py", "windows_job_probe.py")})
    callback_type = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user.EnumWindows.argtypes = [callback_type, w.LPARAM]
    def windows():
        result = {}
        @callback_type
        def callback(window, _):
            owner = w.DWORD()
            user.GetWindowThreadProcessId(window, c.byref(owner))
            if user.IsWindowVisible(window):
                result[int(window)] = (owner.value, native_title(window, user))
            return True
        user.EnumWindows(callback, 0)
        return result
    def capture(name, window=None):
        capture_entry(window or hwnd, user, gdi, root, name + ".png")
    def topmost(window, on):
        user.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
        assert user.SetWindowPos(window, w.HWND(-1 if on else -2), 0, 0, 0, 0, 0x0043)
    def activate(window):
        # Modifier alone activates through the existing foreground guard and
        # cannot clear the source filter or dismiss a review.
        key(window, 0x10)
    def click(x, y):
        point = w.POINT(round(x), round(y))
        assert user.ClientToScreen(hwnd, c.byref(point))
        user.WindowFromPoint.argtypes, user.WindowFromPoint.restype = [w.POINT], w.HWND
        assert user.GetForegroundWindow() == hwnd and user.WindowFromPoint(point) == hwnd
        assert user.SetCursorPos(point.x, point.y)
        actual = w.POINT()
        assert user.GetCursorPos(c.byref(actual)) and (actual.x, actual.y) == (point.x, point.y)
        assert user.GetForegroundWindow() == hwnd and user.WindowFromPoint(actual) == hwnd
        user.mouse_event(2, 0, 0, 0, 0); user.mouse_event(4, 0, 0, 0, 0)
    def buttons():
        # Observe the rendered default Close/Cancel border. Four equal slots
        # share its row; no guessed footer/title coordinate supplies readiness.
        rect = w.RECT(); user.GetClientRect(hwnd, c.byref(rect))
        dc = user.GetDC(hwnd)
        found = None
        try:
            for y in range(rect.bottom - 1, rect.bottom // 2, -1):
                runs, current = [], []
                for x in range(rect.right // 2, rect.right):
                    if gdi.GetPixel(dc, x, y) == 0xD69C56: current.append(x)
                    else:
                        if 35 < len(current) < rect.right // 4: runs.append(current)
                        current = []
                if runs:
                    found = max(runs, key=lambda run: max(run)), y
                    break
        finally:
            user.ReleaseDC(hwnd, dc)
        assert found, "rendered Close/Cancel button is unavailable"
        run, y = found
        # The left content edge is observed from its pane frame, whose grey
        # bottom border shares this row with all settings buttons.
        width = len(run)
        scale = user.GetDpiForWindow(hwnd) / 96
        gap = max(2, round(6 * scale))
        return [((min(run)+max(run))/2 - (3-index)*(width+gap), y-round(3*scale)) for index in range(4)]
    def button(index):
        x,y = buttons()[index]; click(x,y)
    def tool_ready(kind):
        wait_entry(lambda: native_title(hwnd, user) == f"MiniCon SrcMgr - Tools | {kind} | Ready", "tool settings not ready: " + kind)
        wait_quiet(hwnd, user, gdi)
    def review(kind):
        button(1)
        wait_entry(lambda: native_title(hwnd, user) == f"MiniCon SrcMgr - Tools | {kind} | Launch review", "launch review not ready")
        wait_quiet(hwnd, user, gdi)
    def opened(repo):
        wait_entry(lambda: native_title(hwnd, user).startswith("MiniCon SrcMgr - " + repo.name + " | ") and str(repo).casefold() in native_title(hwnd, user).casefold(), "repository was not activated")
        wait_quiet(hwnd, user, gdi)
    def launch(repo):
        nonlocal child, hwnd
        hwnd = None
        with (root / "stderr.log").open("a", encoding="utf-8") as log:
            child = tree.spawn([exe, "--git", git, "--profile", str(profile), "gui", str(repo)], env=env, stdout=sp.DEVNULL, stderr=log)
        def find():
            nonlocal hwnd
            owned = [window for window,(pid,_) in windows().items() if pid == child.pid]
            if owned: hwnd = owned[0]
            assert child.poll() is None, "product exited early"
            return hwnd is not None
        wait_entry(find, "owned native window missing")
        user.ShowWindow(hwnd,9)
        if args.owned_topmost: topmost(hwnd,True)
        activate(hwnd); opened(repo)
    def close():
        user.PostMessageW(hwnd,0x10,0,0); child.wait(timeout=15)
        receipt["exits"].append(child.returncode); assert child.returncode == 0
    def external_window(before_windows, title):
        def find():
            found = [(window,pid,text) for window,(pid,text) in windows().items() if window not in before_windows and title in text]
            if found:
                window,pid,text = found[0]
                external.append(dict(window=window,pid=pid,title=text,closed=False))
                return window
            return None
        found = None
        def observe():
            nonlocal found
            found = find(); return found is not None
        wait_entry(observe,"launched tool has no identifiable new window: " + title)
        return found
    try:
        launch(left)
        key(hwnd,0x32); key(hwnd,"/"); key(hwnd,"中文"); key(hwnd,0x0D)
        wait_quiet(hwnd,user,gdi); capture("context-before-tools")
        key(hwnd,"T"); tool_ready("Editor"); capture("empty-tool-settings")
        key(hwnd,editor); button(0)
        wait_entry(lambda: bool(rows("SELECT value FROM settings WHERE key='tools.editor.v1'")),"editor preference not saved")
        tool_ready("Editor"); capture("saved-editor-settings")
        before_windows = windows()
        review("Editor"); capture("editor-launch-review")
        key(hwnd,0x0D)
        assert not any(filename in title for window,(_,title) in windows().items() if window not in before_windows), "Enter launched the editor"
        button(3); tool_ready("Editor"); capture("editor-review-cancelled")
        assert not any(filename in title for window,(_,title) in windows().items() if window not in before_windows)
        review("Editor"); button(0)
        edit_window = external_window(before_windows, filename)
        if args.owned_topmost: topmost(hwnd,False)
        activate(edit_window)
        if args.owned_topmost: topmost(edit_window,True)
        user.FindWindowExW.argtypes, user.FindWindowExW.restype = [w.HWND,w.HWND,w.LPCWSTR,w.LPCWSTR], w.HWND
        edit = user.FindWindowExW(edit_window,None,"Edit",None)
        assert edit, "classic Notepad text control unavailable"
        user.SendMessageW.argtypes = [w.HWND,w.UINT,w.WPARAM,w.LPARAM]
        length = user.SendMessageW(edit,0x0E,0,0)
        text = c.create_unicode_buffer(length+1)
        user.SendMessageW(edit,0x0D,length+1,c.cast(text,c.c_void_p).value)
        assert text.value.replace("\r\n","\n") == (left/filename).read_text(encoding="utf-8")
        capture("actual-editor-target", edit_window)
        user.PostMessageW(edit_window,0x10,0,0)
        wait_entry(lambda: edit_window not in windows(),"owned editor did not close normally")
        external[-1]["closed"] = True
        activate(hwnd)
        if args.owned_topmost: topmost(hwnd,True)
        opened(left); capture("context-after-editor")
        key(hwnd,0x4F,True); key(hwnd,str(right)); key(hwnd,0x0D); opened(right)
        key(hwnd,"T"); tool_ready("Editor")
        # The two rendered mode buttons occupy the first content row. Locate
        # the blue selected Editor border, then click its adjacent Terminal.
        rect = w.RECT(); user.GetClientRect(hwnd,c.byref(rect)); scale=user.GetDpiForWindow(hwnd)/96
        dc=user.GetDC(hwnd)
        try:
            hit=None
            for y in range(round(60*scale),rect.bottom//3):
                xs=[x for x in range(rect.right*38//100,rect.right*75//100) if gdi.GetPixel(dc,x,y)==0xD69C56]
                if rect.right//6 < len(xs) < rect.right//3: hit=(min(xs),max(xs),y); break
        finally: user.ReleaseDC(hwnd,dc)
        assert hit,"selected Editor mode frame missing"
        left_x,right_x,y=hit
        click((left_x+right_x)/2 + (right_x-left_x)+max(2,round(6*scale)), y+round(8*scale))
        tool_ready("Terminal")
        key(hwnd,terminal); key(hwnd,0x09)
        title="srcmgr-owned-"+root.name
        arguments=["-w","new","new-tab","-d","{repo}","--title",title,"--suppressApplicationTitle",str(Path(os.environ["SystemRoot"])/"System32/cmd.exe"),"/d","/u"]
        for index,value in enumerate(arguments):
            if index: key(hwnd,0x0D)
            key(hwnd,value)
        button(0)
        wait_entry(lambda: bool(rows("SELECT value FROM settings WHERE key='tools.terminal.v1'")),"terminal preference not saved")
        tool_ready("Terminal"); capture("saved-terminal-settings")
        before_windows=windows()
        review("Terminal"); capture("terminal-launch-review"); button(0)
        terminal_window=external_window(before_windows,title)
        if args.owned_topmost: topmost(hwnd,False)
        activate(terminal_window)
        if args.owned_topmost: topmost(terminal_window,True)
        cwd_report=root/"terminal-cwd.txt"
        # This constant diagnostic command is typed only into the identified
        # new owned terminal. It writes one owned report outside both repos.
        key(terminal_window,'cd > "'+str(cwd_report)+'"'); key(terminal_window,0x0D)
        wait_entry(cwd_report.exists,"terminal did not report its actual cwd")
        assert cwd_report.read_text(encoding="utf-16-le").strip().casefold()==str(right).casefold()
        capture("actual-terminal-cwd",terminal_window)
        key(terminal_window,"exit"); key(terminal_window,0x0D)
        wait_entry(lambda: terminal_window not in windows(),"owned new terminal did not close normally")
        external[-1]["closed"]=True
        activate(hwnd)
        if args.owned_topmost: topmost(hwnd,True)
        opened(right); key(hwnd,0x09,True); opened(left); capture("context-after-terminal-return")
        close()
        assert state()==before and rows("SELECT * FROM drafts ORDER BY repository")==drafts
        query_hex="中文".encode("utf-8").hex()
        assert sum(value.split("\n")[11]==query_hex for _,value in rows("SELECT key,value FROM settings WHERE key LIKE 'git.navigation.v1.%'"))==1
        launch(left)
        key(hwnd,"/")
        wait_entry(lambda:native_title(hwnd,user)=="MiniCon SrcMgr - Filter | Files | 1 matches","restored Files filter not ready")
        capture("query-after-process-restart"); key(hwnd,0x1B)
        key(hwnd,"T"); tool_ready("Editor"); capture("editor-after-process-restart")
        button(3); close()
        assert state()==before and rows("SELECT * FROM drafts ORDER BY repository")==drafts
        assert sum(value.split("\n")[11]==query_hex for _,value in rows("SELECT key,value FROM settings WHERE key LIKE 'git.navigation.v1.%'"))==1
        receipt["cases"]=["PASS: save is passive; Enter and pointer Cancel do not launch; explicit pointer Launch opens the exact Unicode working-tree file in Notepad", "PASS: saved terminal definition launches a new identifiable Windows Terminal at the other repository's actual cwd; both owned tool windows close normally; repository context, drafts and Git/source state survive return and product restart"]
        receipt["verdict"]="PASS"
    except Exception as error:
        receipt["verdict"],receipt["failure"]="FAIL",str(error)
        if hwnd and child and child.poll() is None:
            try:
                activate(hwnd); capture("failure-window")
            except Exception as capture_error: receipt["capture_error"]=str(capture_error)
        raise
    finally:
        for tool in external:
            if not tool["closed"] and tool["window"] in windows(): user.PostMessageW(tool["window"],0x10,0,0)
        if child and child.poll() is None:
            user.PostMessageW(hwnd,0x10,0,0)
            try: child.wait(timeout=15)
            except sp.TimeoutExpired:
                receipt["forced_close"]=True; child.terminate(); child.wait(timeout=5)
        receipt["exit_code"]=child.returncode if child else None
        try:
            receipt["process_tree"]=tree.receipt(sample)
            evidence=receipt["process_tree"]
            if evidence["coverage"]!="COMPLETE" or evidence["accounting"]["active_processes"]:
                receipt["process_evidence_status"]="BLOCKED"
                if receipt["verdict"]!="FAIL": receipt["verdict"]="BLOCKED"
        except Exception as error:
            receipt["process_error"]=str(error)
            if receipt["verdict"]!="FAIL": receipt["verdict"]="BLOCKED"
        tree.close()
        if receipt.get("forced_close") or child and child.returncode!=0: receipt["verdict"]="FAIL"
        (root/"receipt.json").write_text(json.dumps(receipt,indent=2),encoding="utf-8")
        if previous: user.SetForegroundWindow(previous)
    assert receipt["verdict"]=="PASS",receipt.get("failure",receipt.get("process_error"))
    print(json.dumps({name:value for name,value in receipt.items() if name not in ("process_tree","harness_sha256")}))


if __name__=="__main__":
    main()
