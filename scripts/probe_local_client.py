"""Windows fresh-profile local-client integration using real input and Git/SQLite.

Entry/trust -> selected stage -> full-index commit -> independent drafts/tabs ->
close/reopen/restart. Reuses separately qualified fault cases; no resource,
latency, native text/accessibility or supported-reference-host claim.
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

from probe_workspace import native, native_title, wait_entry, wait_quiet, capture_entry, click_review_button
from windows_job_probe import OwnedTree


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('exe','git','out'):parser.add_argument('--'+name,required=True)
    parser.add_argument('--owned-topmost',action='store_true')
    args=parser.parse_args();workspace=Path(__file__).resolve().parents[1];root=Path(args.out).resolve()
    if not root.is_relative_to(workspace/'target/qualification'):parser.error('out must be a new owned qualification directory')
    root.mkdir(parents=True,exist_ok=False);(root/'harness-source.py').write_bytes(Path(__file__).read_bytes())
    exe,git=str(Path(args.exe).resolve()),str(Path(args.git).resolve());profile=root/'profile'
    config=root/'global.gitconfig';config.write_bytes(b'[user]\nname = Fixture\nemail = fixture@example.invalid\n[core]\nautocrlf = false\n')
    env={k:v for k,v in os.environ.items() if not k.startswith('GIT_')};env.update(GIT_CONFIG_GLOBAL=str(config),GIT_CONFIG_NOSYSTEM='1')
    def run(repo,*args):return sp.run([git,'-C',str(repo),*args],env=env,check=True,capture_output=True).stdout
    left,right=root/'left project 中文',root/'right project 中文'
    for repo in (left,right):
        repo.mkdir();run(repo,'init','-q','-b','main');(repo/'a.txt').write_bytes(b'original a\n')
        (repo/'b.txt').write_bytes(b''.join(f'original {i}\n'.encode() for i in range(60)))
        run(repo,'add','.');run(repo,'commit','-qm','initial')
        (repo/'a.txt').write_bytes((repo.name+' changed a\n').encode());(repo/'b.txt').write_bytes(b''.join(f'changed {i}\n'.encode() for i in range(60)))
        (repo/'notes[1].tmp').write_bytes(b'untracked must survive\n')
    def files(repo):return {p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in repo.iterdir() if p.is_file()}
    heads={str(repo):run(repo,'rev-parse','HEAD') for repo in (left,right)};indices={str(repo):run(repo,'write-tree') for repo in (left,right)};sources={str(repo):files(repo) for repo in (left,right)}
    def records(table):
        if not (profile/'profile.sqlite3').exists():return []
        with sqlite3.connect((profile/'profile.sqlite3').as_uri()+'?mode=ro',uri=True) as db:return db.execute('SELECT * FROM '+table).fetchall()
    def bodies():return {row[2] for row in records('drafts')}
    def navigation():return {key:value for key,value in records('settings') if key.startswith('git.navigation.v1.')}
    user,gdi,sample,key=native();previous=user.GetForegroundWindow();tree=OwnedTree();child=None;hwnd=None
    receipt=dict(verdict='RUNNING',artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),os=platform.platform(),git=run(left,'--version').decode().strip(),
        owned_topmost=args.owned_topmost,resource_status='NOT-RUN',latency_status='NOT-RUN',visual_verdict='REVIEW-REQUIRED',cases=[],exits=[],
        harness_sha256={name:hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in ('probe_local_client.py','probe_workspace.py','windows_job_probe.py')})
    callback_type=c.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM);user.EnumWindows.argtypes=[callback_type,w.LPARAM]
    @callback_type
    def callback(window,_):
        nonlocal hwnd
        pid=w.DWORD();user.GetWindowThreadProcessId(window,c.byref(pid))
        if pid.value==child.pid and user.IsWindowVisible(window):hwnd=window
        return True
    def found():
        user.EnumWindows(callback,0);assert child.poll() is None,'product exited before entry';return hwnd is not None
    def settle():
        until=time.monotonic()+15;quiet_since=None
        while time.monotonic()<until:
            live=[pid for pid in tree.pids(active_only=True) if tree.images.get(pid,'').casefold()=='git.exe']
            if live:quiet_since=None
            elif quiet_since is None:quiet_since=time.monotonic()
            elif time.monotonic()-quiet_since>=0.25:break
            time.sleep(0.05)
        else:raise RuntimeError('owned Git work did not settle')
        wait_quiet(hwnd,user,gdi)
    def title(value):wait_entry(lambda:native_title(hwnd,user)==value,'native state missing: '+value);settle()
    def opened(repo):
        wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - '+repo.name+' | ') and native_title(hwnd,user).endswith(' Ready'),'current repository content did not activate');settle()
    def capture(name):capture_entry(hwnd,user,gdi,root,name+'.png')
    def launch():
        nonlocal child,hwnd
        hwnd=None
        with (root/'stderr.log').open('a',encoding='utf-8') as log:child=tree.spawn([exe,'--git',git,'--profile',str(profile),'gui'],env=env,stdout=sp.DEVNULL,stderr=log)
        wait_entry(found,'owned native window missing');user.ShowWindow(hwnd,9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT];assert user.SetWindowPos(hwnd,w.HWND(-1),0,0,0,0,0x0043)
        key(hwnd,0x10);title('MiniCon SrcMgr')
    def open_repo(repo,trust=False):
        key(hwnd,0x4F,True);title('MiniCon SrcMgr - Open repository | Ready');key(hwnd,str(repo));key(hwnd,0x0D)
        if trust:
            wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - Trust review | '),'fresh repository trust review missing');settle();capture(repo.name.split()[0]+'-trust-review')
            click_review_button(hwnd,user,gdi,True)
        opened(repo)
    def close():
        settle();user.PostMessageW(hwnd,0x10,0,0);child.wait(timeout=15);receipt['exits'].append(child.returncode);assert child.returncode==0
    def draft(text):
        key(hwnd,0x43);title('MiniCon SrcMgr - Commit | Review blocked');key(hwnd,text)
        wait_entry(lambda:text in bodies(),'draft not durably acknowledged');capture('draft-'+text.split()[0])
    def assert_sources():
        for repo in (left,right):assert files(repo)==sources[str(repo)],'local task unexpectedly rewrote source files'
    try:
        assert not profile.exists(),'profile must be genuinely new; no preseeded trust, drafts or navigation'
        launch();capture('fresh-welcome');open_repo(left,True);capture('left-entered')
        key(hwnd,0x20);expected_a=(left/'a.txt').read_bytes();wait_entry(lambda:run(left,'show',':a.txt')==expected_a,'selected stage did not update index');opened(left)
        staged_tree=run(left,'write-tree');assert run(right,'write-tree')==indices[str(right)] and run(left,'show',':b.txt')==run(left,'show','HEAD:b.txt')
        capture('left-staged');key(hwnd,0x49);title('MiniCon SrcMgr - Review | Project ignore | Ready');capture('fresh-review-footer')
        click_review_button(hwnd,user,gdi,False);opened(left);assert not (left/'.gitignore').exists() and run(left,'write-tree')==staged_tree
        key(hwnd,0x43);title('MiniCon SrcMgr - Commit | Review ready');message='P5 local client 中文\nOnly selected a.txt'
        key(hwnd,'P5 local client 中文');key(hwnd,0x0D);key(hwnd,'Only selected a.txt');wait_entry(lambda:message in bodies(),'multiline commit message not durable');capture('complete-index-review')
        click_review_button(hwnd,user,gdi,False);wait_entry(lambda:run(left,'rev-parse','HEAD')!=heads[str(left)],'pointer commit did not create an object');opened(left)
        observed=run(left,'rev-parse','HEAD');assert run(left,'rev-parse','HEAD^{tree}')==staged_tree and run(left,'show','HEAD:a.txt')==expected_a
        assert run(left,'log','-1','--format=%B').decode().strip()==message and not run(left,'diff','--cached','--name-only')
        wait_entry(lambda:message not in bodies(),'completed commit did not clear its draft');assert_sources();capture('commit-observed')
        receipt['commit_oid']=observed.decode().strip();receipt['cases'].append('PASS: new profile Welcome/Open/fresh trust, selected stage and pointer full-index commit produce the exact reviewed tree/message without rewriting source or the other index; fresh rule review hides the prior result and Cancel writes nothing')
        key(hwnd,0x32);key(hwnd,'/');key(hwnd,'b.txt');title('MiniCon SrcMgr - Filter | Files | 1 matches');key(hwnd,0x0D);opened(left)
        key(hwnd,0x0D);key(hwnd,0x22);settle();capture('left-filtered-diff')
        left_draft='left retained draft 中文';right_draft='right independent draft 中文'
        draft(left_draft);key(hwnd,0x1B);opened(left);open_repo(right,True);draft(right_draft)
        key(hwnd,0x09,True);opened(left);capture('left-returned')
        assert bodies()=={left_draft,right_draft} and len(navigation())==2
        assert any(value.split('\n')[11]==b'b.txt'.hex() for value in navigation().values()),'Files query not bound to saved worktree context'
        key(hwnd,0x57,True);opened(right);capture('right-after-left-close');key(hwnd,0x57,True);title('MiniCon SrcMgr');capture('all-tabs-closed')
        open_repo(left);capture('left-reopened');assert bodies()=={left_draft,right_draft};saved=navigation();close()
        launch();open_repo(left);capture('left-after-process-restart')
        key(hwnd,0x43);title('MiniCon SrcMgr - Commit | Review blocked');capture('restored-left-draft')
        assert bodies()=={left_draft,right_draft} and all(navigation().get(k)==v for k,v in saved.items())
        key(hwnd,' latest');wait_entry(lambda:left_draft+' latest' in bodies(),'restored composer did not preserve draft before editing');key(hwnd,0x1B);opened(left);close()
        assert bodies()=={left_draft+' latest',right_draft};assert_sources()
        assert run(left,'rev-parse','HEAD')==observed and run(right,'rev-parse','HEAD')==heads[str(right)]
        assert run(left,'write-tree')==staged_tree and run(right,'write-tree')==indices[str(right)]
        receipt['cases'].append('PASS: separate reviewed repository entry, independent durable drafts/query/Diff context, tab return, all-tab close, explicit reopen and new native process restore; editing the restored draft preserves its original text and the other project')
        receipt['process_tree']=tree.receipt(sample);assert receipt['process_tree']['coverage']=='COMPLETE' and not receipt['process_tree']['accounting']['active_processes'];receipt['verdict']='PASS'
    except Exception as error:
        receipt.update(verdict='FAIL',failure=repr(error),failure_traceback=traceback.format_exc())
        if hwnd and child and child.poll() is None:
            try:key(hwnd,0x10);capture('failure-window')
            except Exception as capture_error:receipt['capture_error']=repr(capture_error)
        raise
    finally:
        if child and child.poll() is None:
            key(hwnd,0x1B)
            try:settle()
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
    print(json.dumps({k:v for k,v in receipt.items() if k not in ('process_tree','harness_sha256')}))


if __name__=='__main__':main()
