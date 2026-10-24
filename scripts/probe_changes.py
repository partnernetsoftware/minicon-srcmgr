"""Owned Windows Changes buttons, stale partial review and literal ignore journey.

Real input, rendered controls and independent Git/file/profile oracles. Functional
isolation does not qualify response, resources, accessibility, IME or clipboard.
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
    for name in ('exe','git','out'):parser.add_argument('--'+name,required=True)
    parser.add_argument('--owned-topmost',action='store_true')
    args=parser.parse_args();workspace=Path(__file__).resolve().parents[1]
    root=Path(args.out).resolve()
    if not root.is_relative_to(workspace/'target/qualification'):parser.error('out must be a new owned qualification directory')
    root.mkdir(parents=True,exist_ok=False);(root/'harness-source.py').write_bytes(Path(__file__).read_bytes())
    exe,git=str(Path(args.exe).resolve()),str(Path(args.git).resolve())
    repo=root/'project 中文';repo.mkdir();profile=root/'profile';config=root/'global.gitconfig';config.write_bytes(b'')
    env={k:v for k,v in os.environ.items() if not k.startswith('GIT_')};env.update(GIT_CONFIG_GLOBAL=str(config),GIT_CONFIG_NOSYSTEM='1')
    def run(*argv):return sp.run([git,'-C',str(repo),*argv],env=env,check=True,capture_output=True).stdout
    def product(*argv):return sp.run([exe,'--git',git,'--profile',str(profile),*argv],env=env,check=True,capture_output=True).stdout
    run('init','-q','-b','main');run('config','user.name','Fixture');run('config','user.email','fixture@example.invalid')
    middle=b'l\n'*10;base=b'top\n'+middle+b'bottom\n';changed=b'TOP\n'+middle+b'BOTTOM\n'
    source=repo/'a.txt';rules=repo/'.gitignore';source.write_bytes(base);rules.write_bytes(b'# retained\r\n')
    run('add','.');run('commit','-qm','initial');source.write_bytes(changed);rules.write_bytes(b'# retained\r\n# dirty\r\n')
    literal='cache[1] 中文.tmp';sibling='cache1 中文.tmp'
    for name in (literal,sibling,'local.cache'):(repo/name).write_bytes(b'preserved untracked\n')
    product('trust','grant',str(repo));product('draft',str(repo),'save','-m','retained Changes draft 中文')
    head=run('rev-parse','HEAD');index=run('write-tree');rules_before=rules.read_bytes()
    def draft():
        with sqlite3.connect(profile/'profile.sqlite3') as db:return db.execute('SELECT * FROM drafts ORDER BY backend,repository').fetchall()
    drafts_before=draft();user,gdi,sample,key=native();previous=user.GetForegroundWindow();tree=OwnedTree();child=None;hwnd=None
    receipt=dict(verdict='RUNNING',artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),os=platform.platform(),git=run('--version').decode().strip(),owned_topmost=args.owned_topmost,
        resource_status='NOT-RUN',latency_status='NOT-RUN',visual_verdict='REVIEW-REQUIRED',cases=[],controls=[],
        harness_sha256={name:hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ('probe_changes.py','probe_workspace.py','windows_job_probe.py')})
    callback_type=c.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM);user.EnumWindows.argtypes=[callback_type,w.LPARAM]
    @callback_type
    def callback(window,_):
        nonlocal hwnd
        pid=w.DWORD();user.GetWindowThreadProcessId(window,c.byref(pid))
        if pid.value==child.pid and user.IsWindowVisible(window):hwnd=window
        return True
    def found():
        user.EnumWindows(callback,0);assert child.poll() is None,'product exited before entry';return hwnd is not None
    def capture(name):capture_entry(hwnd,user,gdi,root,name+'.png')
    def ready(side='Unstaged'):
        # A previously rendered Ready Diff survives until the mutation reply.
        # Observe actual owned work settling before using that title again.
        until=time.monotonic()+15;quiet_since=None
        while time.monotonic()<until:
            live=[pid for pid in tree.pids(active_only=True) if tree.images.get(pid,'').casefold()=='git.exe']
            if live:quiet_since=None
            elif quiet_since is None:quiet_since=time.monotonic()
            elif time.monotonic()-quiet_since>=0.25:break
            time.sleep(0.05)
        else:raise RuntimeError('owned Git work did not settle')
        wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - '+repo.name+' | ') and native_title(hwnd,user).endswith(' | '+side+' Ready'),'current selected content not ready')
        wait_quiet(hwnd,user,gdi)
    def review(local=False):
        expected='MiniCon SrcMgr - Review | '+('Local ignore' if local else 'Project ignore')+' | Ready'
        wait_entry(lambda:native_title(hwnd,user)==expected,'ignore review not ready');wait_quiet(hwnd,user,gdi)
    def click(x,y):
        point=w.POINT(x,y);assert user.ClientToScreen(hwnd,c.byref(point))
        user.WindowFromPoint.argtypes,user.WindowFromPoint.restype=[w.POINT],w.HWND
        assert user.GetForegroundWindow()==hwnd and user.WindowFromPoint(point)==hwnd
        assert user.SetCursorPos(point.x,point.y);actual=w.POINT();assert user.GetCursorPos(c.byref(actual))
        assert (actual.x,actual.y)==(point.x,point.y) and user.WindowFromPoint(actual)==hwnd
        user.mouse_event(2,0,0,0,0);user.mouse_event(4,0,0,0,0)
    def controls(count,left=False,top=False,above=None):
        rect=w.RECT();user.GetClientRect(hwnd,c.byref(rect));dc=user.GetDC(hwnd);scale=user.GetDpiForWindow(hwnd)/96
        start,end=(0,rect.right*38//100) if left else (rect.right*38//100,rect.right)
        bottom=(rect.bottom*65//100 if left else rect.bottom-1) if above is None else above
        # Scope controls are inside the content area, below repository tabs.
        rows=range(round(70*scale),bottom) if top else range(bottom,round(45*scale),-1)
        try:
            for y in rows:
                runs=[];begin=None;previous_color=None
                for x in range(start,end+1):
                    color=gdi.GetPixel(dc,x,y) if x<end else -1
                    if color!=previous_color:
                        if begin is not None and 45*scale<x-begin<rect.right//2:runs.append((begin,x-1,y))
                        begin=x if color in (0xD69C56,0x3C3C3C,0x808080) else None
                    previous_color=color
                if len(runs)==count:
                    result=[]
                    for a,b,row in runs:
                        edge=gdi.GetPixel(dc,a,row)
                        interior=row-round(3*scale) if gdi.GetPixel(dc,a,row-round(3*scale))==edge else row+round(3*scale)
                        result.append((a,b,row,interior))
                    return result
        finally:user.ReleaseDC(hwnd,dc)
        return None
    def button(index,count,left=False,top=False,upper=False):
        found_controls=controls(count,left,top)
        if upper:
            assert found_controls is not None
            found_controls=controls(count,above=found_controls[0][2]-round(35*user.GetDpiForWindow(hwnd)/96))
        assert found_controls is not None,'rendered controls missing'
        a,b,y,inside=found_controls[index];receipt['controls'].append(dict(index=index,count=count,left=left,top=top,upper=upper,rect_line=[a,b,y],title=native_title(hwnd,user)))
        click((a+b)//2,inside)
    def select(query):
        key(hwnd,0x32);key(hwnd,'/');key(hwnd,0x41,True);key(hwnd,query);key(hwnd,0x0D);ready()
    try:
        with (root/'stderr.log').open('w',encoding='utf-8') as log:child=tree.spawn([exe,'--git',git,'--profile',str(profile),'gui',str(repo)],env=env,stdout=sp.DEVNULL,stderr=log)
        wait_entry(found,'owned native window missing');user.ShowWindow(hwnd,9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT];assert user.SetWindowPos(hwnd,w.HWND(-1),0,0,0,0,0x0043)
        key(hwnd,0x10);ready();select('a.txt');capture('changes-buttons')
        button(0,4,left=True);wait_entry(lambda:run('show',':a.txt')==changed,'Stage did not write selected file');ready('Staged')
        button(1,4,left=True);wait_entry(lambda:run('write-tree')==index,'Unstage did not restore original index');ready()
        # The selected first removed line is staged through the visible Lines button.
        button(0,3);wait_entry(lambda:run('show',':a.txt')==middle+b'bottom\n','Stage lines did not stage just the selected deletion');ready()
        button(1,3,upper=True);ready('Staged');capture('staged-side')
        button(2,3,upper=True);wait_entry(lambda:run('write-tree')==index,'Unstage hunk did not restore actual index');ready()
        button(2,3,upper=True);wait_entry(lambda:run('show',':a.txt')==b'TOP\n'+middle+b'bottom\n','Stage hunk widened scope');ready()
        button(1,4,left=True);wait_entry(lambda:run('write-tree')==index,'file unstage failed');ready()
        foreign=b'FOREIGN\n'+middle+b'BOTTOM\n';source.write_bytes(foreign)
        button(0,3);ready();capture('stale-lines-refused')
        assert run('write-tree')==index and source.read_bytes()==foreign,'old Diff selected foreign bytes'
        receipt['cases'].append('PASS: pointer file stage/unstage, explicit staged side, line/hunk scope and external-worktree freshness use actual index oracles')
        # Existing recovery preview supplies the same pointer Confirm/Cancel path.
        button(2,4,left=True)
        wait_entry(lambda:native_title(hwnd,user)=='MiniCon SrcMgr - Review | Repository action | Ready','discard review missing');wait_quiet(hwnd,user,gdi)
        key(hwnd,0x0D);assert source.read_bytes()==foreign;button(1,2);ready();assert source.read_bytes()==foreign
        button(2,4,left=True);wait_entry(lambda:native_title(hwnd,user)=='MiniCon SrcMgr - Review | Repository action | Ready','fresh discard review missing');wait_quiet(hwnd,user,gdi)
        capture('discard-review');button(0,2);wait_entry(lambda:source.read_bytes()==base,'confirmed discard did not restore file')
        wait_quiet(hwnd,user,gdi)
        copies=list((profile/'recovery').glob('*/worktree.bin'));assert any(p.read_bytes()==foreign for p in copies),'discard lost the reviewed recovery bytes'
        select(literal);button(3,4,left=True);review();capture('project-ignore-review');key(hwnd,0x0D);assert rules.read_bytes()==rules_before
        button(1,2);ready();assert rules.read_bytes()==rules_before
        button(3,4,left=True);review();external=rules_before+b'# external comment\r\n';rules.write_bytes(external);button(0,2);ready();capture('stale-ignore-refused')
        assert rules.read_bytes()==external and run('write-tree')==index,'stale Ignore overwrote rules or index'
        button(3,4,left=True);review();button(0,2);wait_entry(lambda:rules.read_bytes()!=external,'Save rule did not publish')
        matched=sp.run([git,'-C',str(repo),'check-ignore','-z','--stdin'],input=literal.encode()+b'\0',env=env,capture_output=True)
        assert matched.returncode==0 and matched.stdout==literal.encode()+b'\0','Git did not exclude the exact raw selected path'
        sibling_result=sp.run([git,'-C',str(repo),'check-ignore','-q','--',sibling],env=env,capture_output=True);assert sibling_result.returncode==1,'literal brackets widened into glob'
        assert rules.read_bytes().startswith(external);project_saved=rules.read_bytes()
        wait_entry(lambda:not native_title(hwnd,user).startswith('MiniCon SrcMgr - Review'),'saved project review did not close');wait_quiet(hwnd,user,gdi)
        select('local.cache');button(3,4,left=True);review();button(1,2,top=True);review(True);capture('local-ignore-review');button(0,2)
        wait_entry(lambda:(repo/'.git/info/exclude').exists() and b'/local.cache' in (repo/'.git/info/exclude').read_bytes(),'local exclude not saved')
        assert run('check-ignore','--','local.cache').strip()==b'local.cache' and rules.read_bytes()==project_saved
        receipt['cases'].append('PASS: discard Enter/Cancel preserve bytes; explicit pointer approval retains recovery copy; project/local Ignore, Enter/Cancel and stale destination preserve unrelated rules and literal siblings')
        key(hwnd,0x74);wait_quiet(hwnd,user,gdi);capture('changes-final')
        wait_entry(lambda:not native_title(hwnd,user).startswith('MiniCon SrcMgr - Review'),'review did not close')
        user.PostMessageW(hwnd,0x10,0,0);child.wait(timeout=15);assert child.returncode==0
        assert run('rev-parse','HEAD')==head and run('write-tree')==index and source.read_bytes()==base and draft()==drafts_before
        for name in (literal,sibling,'local.cache'):assert (repo/name).read_bytes()==b'preserved untracked\n'
        receipt['process_tree']=tree.receipt(sample);assert receipt['process_tree']['coverage']=='COMPLETE' and not receipt['process_tree']['accounting']['active_processes']
        receipt['verdict']='PASS'
    except Exception as error:
        receipt.update(verdict='FAIL',failure=repr(error),failure_traceback=traceback.format_exc())
        if hwnd and child and child.poll() is None:
            try:key(hwnd,0x10);capture('failure-window')
            except Exception as capture_error:receipt['capture_error']=repr(capture_error)
        raise
    finally:
        if child and child.poll() is None:
            key(hwnd,0x1B)
            try:ready()
            except Exception as error:receipt['cleanup_readiness_error']=repr(error)
            user.PostMessageW(hwnd,0x10,0,0)
            try:child.wait(timeout=15)
            except sp.TimeoutExpired:receipt['forced_close']=True;child.terminate();child.wait(timeout=5)
        receipt['exit_code']=child.returncode if child else None
        try:
            if 'process_tree' not in receipt:receipt['process_tree']=tree.receipt(sample)
            if receipt['process_tree']['coverage']!='COMPLETE' or receipt['process_tree']['accounting']['active_processes']:
                receipt['process_evidence_status']='BLOCKED'
                if receipt['verdict']!='FAIL':receipt['verdict']='BLOCKED'
        except Exception as error:
            receipt['process_error']=repr(error)
            if receipt['verdict']!='FAIL':receipt['verdict']='BLOCKED'
        tree.close()
        if receipt.get('forced_close') or child and child.returncode!=0:receipt['verdict']='FAIL'
        (root/'receipt.json').write_text(json.dumps(receipt,indent=2),encoding='utf-8')
        if previous:user.SetForegroundWindow(previous)
    assert receipt['verdict']=='PASS',receipt.get('failure')
    print(json.dumps({k:v for k,v in receipt.items() if k not in ('process_tree','controls','harness_sha256')},ensure_ascii=True))


if __name__=='__main__':main()
