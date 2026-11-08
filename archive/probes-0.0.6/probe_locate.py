"""Windows missing-recent Locate/Remove and subsequent explicit trusted Open.

Owned fixtures, actual Git/profile oracles and retained native captures only.
No resource, response, accessibility, IME or clipboard qualification follows.
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
import time
import traceback

from probe_workspace import native, native_title, wait_entry, wait_quiet, capture_entry
from windows_job_probe import OwnedTree


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("exe","git","out"): parser.add_argument("--"+name,required=True)
    parser.add_argument("--owned-topmost",action="store_true")
    args=parser.parse_args()
    workspace=Path(__file__).resolve().parents[1]
    root=Path(args.out).resolve()
    if not root.is_relative_to(workspace/"target/qualification"): parser.error("out must be a new owned qualification directory")
    root.mkdir(parents=True,exist_ok=False)
    (root/"harness-source.py").write_bytes(Path(__file__).read_bytes())
    exe,git=str(Path(args.exe).resolve()),str(Path(args.git).resolve())
    profile=root/"profile"; old=root/"old project 中文"; new=root/"located project 中文"
    config=root/"global.gitconfig";config.write_bytes(b"")
    env={name:value for name,value in os.environ.items() if not name.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=str(config),GIT_CONFIG_NOSYSTEM="1")
    def run(repo,*arguments): return sp.run([git,"-C",str(repo),*arguments],env=env,check=True,capture_output=True).stdout
    def product(*arguments): return sp.run([exe,"--git",git,"--profile",str(profile),*arguments],env=env,check=True,capture_output=True).stdout
    old.mkdir();run(old,"init","-q","-b","main");run(old,"config","user.name","Fixture");run(old,"config","user.email","fixture@example.invalid")
    (old/"a.txt").write_bytes(b"original\n");(old/"binary.dat").write_bytes(b"\0\x01unchanged\xff")
    run(old,"add",".");run(old,"commit","-qm","initial")
    (old/"a.txt").write_text("retained work 中文\n",encoding="utf-8");(old/"untracked.txt").write_bytes(b"retained untracked\n")
    product("trust","grant",str(old));product("draft",str(old),"save","-m","old path draft retained 中文")
    def rows(sql):
        with sqlite3.connect(profile/"profile.sqlite3") as db: return db.execute(sql).fetchall()
    def recent(): return rows("SELECT value FROM settings WHERE key='workspace.recents.v1'")[0][0]
    def drafts(): return rows("SELECT * FROM drafts ORDER BY backend,repository")
    def source_state(repo):
        return dict(head=run(repo,"rev-parse","HEAD").hex(),index_tree=run(repo,"write-tree").hex(),files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in repo.iterdir() if p.is_file()})
    before=source_state(old)
    user,gdi,sample,key=native();previous=user.GetForegroundWindow();tree=OwnedTree()
    child,hwnd=None,None
    receipt=dict(verdict="RUNNING",artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),os=platform.platform(),git=run(old,"--version").decode().strip(),
                 owned_topmost=args.owned_topmost,latency_status="NOT-RUN",resource_status="NOT-RUN",visual_verdict="REVIEW-REQUIRED",cases=[],exits=[],
                 harness_sha256={name:hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ("probe_locate.py","probe_workspace.py","windows_job_probe.py")})
    callback_type=c.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM);user.EnumWindows.argtypes=[callback_type,w.LPARAM]
    @callback_type
    def callback(window,_):
        nonlocal hwnd
        pid=w.DWORD();user.GetWindowThreadProcessId(window,c.byref(pid))
        if pid.value==child.pid and user.IsWindowVisible(window): hwnd=window
        return True
    def found():
        user.EnumWindows(callback,0);assert child.poll() is None,"product exited before native entry";return hwnd is not None
    def activate(): key(hwnd,0x10)
    def capture(name): capture_entry(hwnd,user,gdi,root,name+".png")
    def title(expected):
        wait_entry(lambda:native_title(hwnd,user)==expected,"native state not ready: "+expected);wait_quiet(hwnd,user,gdi)
    def opened(repo):
        wait_entry(lambda:native_title(hwnd,user).startswith("MiniCon SrcMgr - "+repo.name+" | ") and str(repo).casefold() in native_title(hwnd,user).casefold(),"repository did not activate")
        wait_quiet(hwnd,user,gdi)
    def launch(repo=None):
        nonlocal child,hwnd
        hwnd=None
        arguments=[exe,"--git",git,"--profile",str(profile),"gui"]+([str(repo)] if repo else [])
        with (root/"stderr.log").open("a",encoding="utf-8") as log: child=tree.spawn(arguments,env=env,stdout=sp.DEVNULL,stderr=log)
        wait_entry(found,"owned native window missing");user.ShowWindow(hwnd,9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT]
            assert user.SetWindowPos(hwnd,w.HWND(-1),0,0,0,0,0x0043)
        activate()
        if repo: opened(repo)
        else: title("MiniCon SrcMgr")
    def close():
        wait_quiet(hwnd,user,gdi);user.PostMessageW(hwnd,0x10,0,0);child.wait(timeout=15)
        receipt["exits"].append(child.returncode);assert child.returncode==0
    def controls(count):
        rect=w.RECT();user.GetClientRect(hwnd,c.byref(rect));dc=user.GetDC(hwnd)
        try:
            for y in range(rect.bottom-1,rect.bottom//2,-1):
                runs=[];start=None;previous_color=None
                for x in range(rect.right*38//100,rect.right+1):
                    color=gdi.GetPixel(dc,x,y) if x<rect.right else -1
                    if color!=previous_color:
                        if start is not None and 40<x-start<rect.right//3:runs.append((start,x-1,y))
                        start=x if color in (0xD69C56,0x3C3C3C) else None
                    previous_color=color
                if len(runs)==count:return runs
        finally:user.ReleaseDC(hwnd,dc)
        return None
    def button(index,count):
        wait_entry(lambda:controls(count) is not None,"rendered entry buttons unavailable")
        left,right,y=controls(count)[index];scale=user.GetDpiForWindow(hwnd)/96
        point=w.POINT((left+right)//2,y-max(3,round(3*scale)))
        assert user.ClientToScreen(hwnd,c.byref(point))
        user.WindowFromPoint.argtypes,user.WindowFromPoint.restype=[w.POINT],w.HWND
        assert user.GetForegroundWindow()==hwnd and user.WindowFromPoint(point)==hwnd
        assert user.SetCursorPos(point.x,point.y)
        actual=w.POINT();assert user.GetCursorPos(c.byref(actual)) and (actual.x,actual.y)==(point.x,point.y)
        assert user.GetForegroundWindow()==hwnd and user.WindowFromPoint(actual)==hwnd
        user.mouse_event(2,0,0,0,0);user.mouse_event(4,0,0,0,0)
    try:
        launch(old);capture("old-project-bootstrap");close()
        original_recent=recent();old_drafts=drafts();old_views=dict(rows("SELECT key,value FROM settings WHERE key LIKE 'git.navigation.v1.%'"))
        trust_before=(profile/"trust.v1").read_bytes()
        assert old.resolve().parent==root and new.parent==root and not new.exists()
        old.rename(new)
        assert source_state(new)==before
        untrusted_start=time.time_ns()//100+116444736000000000
        launch();wait_entry(lambda:controls(3) is not None,"recent list not presented");capture("missing-recent")
        button(0,3);title("MiniCon SrcMgr - Open repository | Ready");capture("missing-open-failure")
        assert recent()==original_recent and (profile/"trust.v1").read_bytes()==trust_before
        button(1,3);title("MiniCon SrcMgr - Locate recent | Editing")
        key(hwnd,str(new));capture("locate-input")
        button(0,2);title("MiniCon SrcMgr - Locate recent | Review");capture("locate-review")
        key(hwnd,0x0D)
        assert recent()==original_recent and (profile/"trust.v1").read_bytes()==trust_before
        button(1,2);title("MiniCon SrcMgr - Locate recent | Editing");capture("locate-cancelled")
        assert recent()==original_recent
        button(0,2);title("MiniCon SrcMgr - Locate recent | Review");button(0,2)
        title("MiniCon SrcMgr");wait_entry(lambda:recent()!=original_recent,"location save not durable");capture("location-saved-welcome")
        assert drafts()==old_drafts and (profile/"trust.v1").read_bytes()==trust_before and source_state(new)==before
        assert dict(rows("SELECT key,value FROM settings WHERE key LIKE 'git.navigation.v1.%'"))==old_views
        button(0,3)
        wait_entry(lambda:native_title(hwnd,user).startswith("MiniCon SrcMgr - Trust review | "),"new location inherited old trust")
        capture("separate-trust-review");key(hwnd,0x0D)
        assert (profile/"trust.v1").read_bytes()==trust_before
        button(1,2);title("MiniCon SrcMgr - Open repository | Ready");capture("trust-cancelled")
        assert (profile/"trust.v1").read_bytes()==trust_before
        button(0,3)
        wait_entry(lambda:native_title(hwnd,user).startswith("MiniCon SrcMgr - Trust review | "),"fresh trust review unavailable")
        capture("fresh-trust-review")
        explicit_git_authority=time.time_ns()//100+116444736000000000
        button(0,2);opened(new);capture("explicit-open-new-location")
        trust_after=(profile/"trust.v1").read_bytes();assert trust_after!=trust_before
        assert drafts()==old_drafts and source_state(new)==before
        key(hwnd,0x57,True);title("MiniCon SrcMgr");capture("recent-after-tab-close")
        button(2,3);wait_entry(lambda:recent()=="","Remove did not remove only the recent record");capture("recent-removed")
        assert drafts()==old_drafts and source_state(new)==before and (profile/"trust.v1").read_bytes()==trust_after
        saved_views=dict(rows("SELECT key,value FROM settings WHERE key LIKE 'git.navigation.v1.%'"))
        assert all(saved_views.get(key)==value for key,value in old_views.items())
        close();launch();capture("empty-recents-after-restart");close()
        assert recent()=="" and drafts()==old_drafts and source_state(new)==before and (profile/"trust.v1").read_bytes()==trust_after
        receipt["process_tree"]=tree.receipt(sample)
        git_events=[p for p in receipt["process_tree"]["processes"] if p.get("image","").casefold()=="git.exe"]
        unauthorized=[p for p in git_events if untrusted_start<=p["created_100ns"]<explicit_git_authority]
        assert not unauthorized,"Git executed while browsing/locating/removing or before explicit new-location trust"
        assert any(p["created_100ns"]>=explicit_git_authority for p in git_events),"collector has no positive control for new-location Git"
        receipt["git_before_new_trust"]=0
        receipt["cases"]=["PASS: missing Open retains the source; native Locate text/review/Enter refusal/Back preserve the original record; explicit Save changes only the recent location with no Git or inherited trust", "PASS: explicit new-location Open requires fresh trust; Enter/Cancel do not grant it; pointer approval opens current state; Remove and restart preserve source, index/HEAD, old drafts/views and independent trust metadata"]
        receipt["verdict"]="PASS"
    except Exception as error:
        receipt["verdict"],receipt["failure"]="FAIL",repr(error);receipt["failure_traceback"]=traceback.format_exc()
        if hwnd and child and child.poll() is None:
            try:activate();capture("failure-window")
            except Exception as capture_error:receipt["capture_error"]=repr(capture_error)
        raise
    finally:
        if child and child.poll() is None:
            for _ in range(2):key(hwnd,0x1B)
            user.PostMessageW(hwnd,0x10,0,0)
            try:child.wait(timeout=15)
            except sp.TimeoutExpired:receipt["forced_close"]=True;child.terminate();child.wait(timeout=5)
        receipt["exit_code"]=child.returncode if child else None
        try:
            if "process_tree" not in receipt:receipt["process_tree"]=tree.receipt(sample)
            evidence=receipt["process_tree"]
            if evidence["coverage"]!="COMPLETE" or evidence["accounting"]["active_processes"]:
                receipt["process_evidence_status"]="BLOCKED"
                if receipt["verdict"]!="FAIL":receipt["verdict"]="BLOCKED"
        except Exception as error:
            receipt["process_error"]=repr(error)
            if receipt["verdict"]!="FAIL":receipt["verdict"]="BLOCKED"
        tree.close()
        if receipt.get("forced_close") or child and child.returncode!=0:receipt["verdict"]="FAIL"
        (root/"receipt.json").write_text(json.dumps(receipt,indent=2),encoding="utf-8")
        if previous:user.SetForegroundWindow(previous)
    assert receipt["verdict"]=="PASS",receipt.get("failure",receipt.get("process_error"))
    print(json.dumps({name:value for name,value in receipt.items() if name not in ("process_tree","harness_sha256")}))


if __name__=="__main__":main()
