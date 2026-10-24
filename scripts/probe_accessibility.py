"""Scoped real Windows UIA publication and action qualification, owned fixtures.

No synthetic accessibility mock or product hook. UIA acceptance alone does not
qualify screen readers, complete form/text coverage, other OSes or resources.
"""
import argparse, ctypes as c
from ctypes import wintypes as w
import hashlib,json,os
from pathlib import Path
import platform,sqlite3,subprocess as sp,time,traceback
from probe_workspace import native,native_title,wait_entry,wait_quiet,capture_entry
from windows_job_probe import OwnedTree

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('exe','git','out'):parser.add_argument('--'+name,required=True)
    parser.add_argument('--owned-topmost',action='store_true')
    parser.add_argument('--smoke',action='store_true',help='Only provider startup, live value, Back and normal exit; no repository trust or Git work')
    parser.add_argument('--entry-forms',action='store_true',help='Actual Create review/Back/init/separate trust and local identity UIA journey')
    parser.add_argument('--keyboard-entry',action='store_true',help='With entry-forms, verify actual Ctrl+A replacement and Home insertion through owned keyboard input')
    args=parser.parse_args()
    if args.keyboard_entry and not args.entry_forms:parser.error('keyboard-entry requires entry-forms')
    if args.smoke and args.entry_forms:parser.error('select only one scoped mode')
    workspace=Path(__file__).resolve().parents[1];root=Path(args.out).resolve()
    if not root.is_relative_to(workspace/'target/qualification'):parser.error('out must be a new owned qualification directory')
    root.mkdir(parents=True,exist_ok=False);(root/'harness-source.py').write_bytes(Path(__file__).read_bytes());(root/'uia-client-source.ps1').write_bytes(Path(__file__).with_name('uia_client.ps1').read_bytes())
    exe,git=str(Path(args.exe).resolve()),str(Path(args.git).resolve());repo=root/'project 中文';repo.mkdir();profile=root/'profile';global_config=root/'global.gitconfig';global_config.write_bytes(b'[user]\nname = Fixture\nemail = fixture@example.invalid\n')
    env={k:v for k,v in os.environ.items() if not k.startswith('GIT_')};env.update(GIT_CONFIG_GLOBAL=str(global_config),GIT_CONFIG_NOSYSTEM='1')
    def run(*argv):return sp.run([git,'-C',str(repo),*argv],env=env,check=True,capture_output=True).stdout
    run('init','-q','-b','main');(repo/'a.txt').write_bytes(b'base\n');run('add','.');run('commit','-qm','initial');(repo/'a.txt').write_bytes(b'changed\n')
    initial=run('write-tree');head=run('rev-parse','HEAD');user,gdi,sample,key=native();tree=OwnedTree();child=None;hwnd=None;previous=user.GetForegroundWindow();sequence=0
    receipt=dict(verdict='RUNNING',artifact_sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),os=platform.platform(),owned_topmost=args.owned_topmost,latency_status='NOT-RUN',resource_status='NOT-RUN',screen_reader_status='NOT-RUN',visual_verdict='REVIEW-REQUIRED',cases=[])
    callback_type=c.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM);user.EnumWindows.argtypes=[callback_type,w.LPARAM]
    @callback_type
    def callback(window,_):
        nonlocal hwnd
        pid=w.DWORD();user.GetWindowThreadProcessId(window,c.byref(pid))
        if pid.value==child.pid and user.IsWindowVisible(window):hwnd=window
        return True
    def found():user.EnumWindows(callback,0);assert child.poll() is None;return hwnd is not None
    def title(value):wait_entry(lambda:native_title(hwnd,user)==value,'native context missing: '+value);wait_quiet(hwnd,user,gdi)
    def settled():
        deadline=time.monotonic()+15;quiet_since=None;rect=w.RECT();user.GetClientRect(hwnd,c.byref(rect));scale=user.GetDpiForWindow(hwnd)/96;dc=user.GetDC(hwnd)
        try:
            while time.monotonic()<deadline:
                live=[pid for pid in tree.pids(active_only=True) if tree.images.get(pid,'').casefold()=='git.exe']
                working=gdi.GetPixel(dc,rect.right-round(12*scale)-1,round(12*scale)+1)==0xD69C56
                if live or working:quiet_since=None
                elif quiet_since is None:quiet_since=time.monotonic()
                elif time.monotonic()-quiet_since>=0.25:return
                time.sleep(0.05)
            raise RuntimeError('owned work did not settle')
        finally:user.ReleaseDC(hwnd,dc)
    def opened():wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - '+repo.name+' | ') and native_title(hwnd,user).endswith(' Ready'),'selected repository view missing');settled()
    def capture(name):capture_entry(hwnd,user,gdi,root,name+'.png')
    def client(mode,**values):
        nonlocal sequence
        sequence+=1;request=root/f'client-{sequence:02}-request.json';output=root/f'client-{sequence:02}-result.json';request.write_text(json.dumps(dict(mode=mode,**values)),encoding='utf-8')
        attempt=sp.run(['powershell.exe','-NoProfile','-NonInteractive','-File',str(Path(__file__).with_name('uia_client.ps1')),'-Handle',str(hwnd),'-OwnerPid',str(child.pid),'-RequestFile',str(request),'-OutputFile',str(output)],env=env,capture_output=True,timeout=25)
        (root/f'client-{sequence:02}.log').write_bytes(attempt.stdout+attempt.stderr)
        assert attempt.returncode==0,(attempt.stdout+attempt.stderr).decode(errors='replace')
        return json.loads(output.read_text(encoding='utf-8-sig'))
    try:
        with (root/'stderr.log').open('w',encoding='utf-8') as log:child=tree.spawn([exe,'--git',git,'--profile',str(profile),'gui'],env=env,stdout=sp.DEVNULL,stderr=log)
        wait_entry(found,'owned native window missing');user.ShowWindow(hwnd,9)
        if args.owned_topmost:
            user.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT];assert user.SetWindowPos(hwnd,w.HWND(-1),0,0,0,0,0x0043)
        key(hwnd,0x10);title('MiniCon SrcMgr');nodes=client('list')['nodes'];assert any(n['name']=='Open' and n['role']=='ControlType.Button' for n in nodes),'real UIA provider did not expose Open';capture('welcome-uia')
        first_authority=None
        if args.entry_forms:
            created=root/'created project 中文';created.mkdir();(created/'keep.bin').write_bytes(b'preserved\x00content')
            client('invoke',name='Create');title('MiniCon SrcMgr - Create repository')
            assert client('disabled',name='Review creation')['verdict']=='REFUSED_DISABLED'
            client('set',name='Directory (parent must exist)',value=str(created))
            if args.keyboard_entry:
                key(hwnd,0x41,ctrl=True);key(hwnd,str(created))
                nodes=client('list')['nodes'];assert any(n['name']=='Directory (parent must exist)' and n['value']==str(created) for n in nodes),'actual Ctrl+A did not replace the existing path'
            client('set',name='Initial branch',value='feature/native')
            client('invoke',name='Review creation');wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - Create review | '),'creation review missing')
            nodes=client('list')['nodes'];assert not any(n['name']=='Initial branch' for n in nodes),'review exposes editable creation fields'
            client('invoke',name='Back');title('MiniCon SrcMgr - Create repository')
            assert not (created/'.git').exists() and (created/'keep.bin').read_bytes()==b'preserved\x00content'
            nodes=client('list')['nodes'];assert any(n['name']=='Initial branch' and n['value']=='feature/native' for n in nodes),'Back lost initial branch'
            client('invoke',name='Review creation');wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - Create review | '),'second creation review missing')
            first_authority=time.time_ns()//100+116444736000000000
            client('invoke',name='Create repository');wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - Trust review | '),'creation bypassed separate trust')
            def created_git(*argv):return sp.run([git,'-C',str(created),*argv],env=env,check=True,capture_output=True).stdout
            assert created_git('symbolic-ref','HEAD').strip()==b'refs/heads/feature/native'
            assert not (created/'.git/index').exists() and (created/'keep.bin').read_bytes()==b'preserved\x00content'
            capture('created-separate-trust-uia');client('invoke',name='Cancel');title('MiniCon SrcMgr - Open repository | Ready')
            assert not (profile/'trust.v1').exists(), 'init or cancelled trust granted repository authority'
            client('invoke',name='Back');title('MiniCon SrcMgr')
        client('invoke',name='Open');title('MiniCon SrcMgr - Open repository | Ready')
        client('set',name='Repository path',value=str(repo)+' missing')
        client('set',name='Repository path',value=str(repo));nodes=client('list')['nodes'];assert any(n['name']=='Repository path' and n['value']==str(repo) for n in nodes),'ValuePattern did not edit the actual field'
        if args.smoke:
            client('invoke',name='Back');title('MiniCon SrcMgr')
            user.PostMessageW(hwnd,0x10,0,0);child.wait(timeout=15);assert child.returncode==0
            receipt['process_tree']=tree.receipt(sample)
            assert receipt['process_tree']['coverage']=='COMPLETE' and not receipt['process_tree']['accounting']['active_processes']
            assert not [p for p in receipt['process_tree']['processes'] if p.get('image','').casefold()=='git.exe'],'smoke unexpectedly executed Git'
            assert run('rev-parse','HEAD')==head and run('write-tree')==initial and (repo/'a.txt').read_bytes()==b'changed\n'
            receipt['cases']=['PASS: product loader, owned UIA provider, exact live path replacement, Back and normal exit; no owned Git and unchanged HEAD/index/source']
            receipt['verdict']='PASS'
            print(json.dumps({k:v for k,v in receipt.items() if k!='process_tree'}))
            return
        client('invoke',name='Open');wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - Trust review | '),'native UIA Open bypassed/missed trust');capture('trust-uia')
        before_authority=time.time_ns()//100+116444736000000000
        client('invoke',name='Trust and open');opened();nodes=client('list')['nodes'];assert any(n['name']=='Stage' and n['enabled'] for n in nodes)
        if args.entry_forms:
            client('invoke',name='Identity settings');title('MiniCon SrcMgr - Identity settings | Ready')
            edit_started=time.time_ns()//100+116444736000000000
            identity_name='Native identity 中文'
            client('set',name='Name',value=identity_name)
            if args.keyboard_entry:
                key(hwnd,0x24);key(hwnd,'Edited ');identity_name='Edited '+identity_name
            client('set',name='Email',value='native@example.invalid')
            nodes=client('list')['nodes'];assert any(n['name']=='Name' and n['value']==identity_name for n in nodes)
            capture('identity-uia');edit_finished=time.time_ns()//100+116444736000000000;client('invoke',name='Save local')
            wait_entry(lambda:native_title(hwnd,user).startswith('MiniCon SrcMgr - Trust review | '),'identity config change bypassed separate trust review')
            assert run('config','--local','--get','user.name').decode('utf-8').strip()==identity_name
            assert run('config','--local','--get','user.email').strip()==b'native@example.invalid'
            client('invoke',name='Trust and open');opened()
            assert run('rev-parse','HEAD')==head and run('write-tree')==initial and (repo/'a.txt').read_bytes()==b'changed\n'
            user.PostMessageW(hwnd,0x10,0,0);child.wait(timeout=15);assert child.returncode==0
            receipt['process_tree']=tree.receipt(sample);assert receipt['process_tree']['coverage']=='COMPLETE' and not receipt['process_tree']['accounting']['active_processes']
            git_events=[p for p in receipt['process_tree']['processes'] if p.get('image','').casefold()=='git.exe']
            assert not [p for p in git_events if p['created_100ns']<first_authority],'Git ran before creation authority'
            assert any(p['created_100ns']>=before_authority for p in git_events),'explicit existing-repository trust did not execute Git'
            assert not [p for p in git_events if edit_started<=p['created_100ns']<edit_finished],'field editing scheduled Git work'
            receipt['cases']=['PASS: real UIA Create field replacement, initial disabled review, Back preserving input/no metadata, explicit init with exact branch and untouched existing bytes, then separate trust cancellation', 'PASS: real UIA local identity fields and deliberate Save reach exact local Git config; existing HEAD/index/source remain unchanged']
            if args.keyboard_entry:receipt['cases'].append('PASS: actual owned Ctrl+A replaces the full existing path; Home inserts at the identity start; exact UIA/config values observed and no Git scheduled during field edits; native clipboard/IME composition remain NOT-RUN')
            receipt['verdict']='PASS'
            print(json.dumps({k:v for k,v in receipt.items() if k!='process_tree'}))
            return
        result=client('stale',name='Stage',next='Ignore',after='Cancel');assert result['verdict']=='REFUSED_STALE';title('MiniCon SrcMgr - Review | Project ignore | Ready')
        assert run('write-tree')==initial and not (repo/'.gitignore').exists();nodes=client('list')['nodes'];assert not any(n['name']=='Stage' for n in nodes),'modal UIA tree exposes background mutation'
        client('invoke',name='Cancel');opened();client('invoke',name='Stage');wait_entry(lambda:run('show',':a.txt')==b'changed\n','UIA Stage did not reach the actual index');opened()
        staged=run('write-tree');assert client('disabled',name='Stage')['verdict']=='REFUSED_DISABLED';assert run('write-tree')==staged;capture('staged-uia')
        client('invoke',name='Prepare commit');title('MiniCon SrcMgr - Commit | Review ready');message='UIA draft 中文';client('set',name='Commit message',value=message)
        nodes=client('list')['nodes'];assert any(n['name']=='Commit message' and n['value']==message for n in nodes);capture('composer-uia');client('invoke',name='Save and close');opened()
        with sqlite3.connect((profile/'profile.sqlite3').as_uri()+'?mode=ro',uri=True) as db:assert db.execute('SELECT body FROM drafts').fetchall()==[(message,)],'UIA draft did not reach durable SQLite state'
        assert run('rev-parse','HEAD')==head and run('write-tree')==staged and (repo/'a.txt').read_bytes()==b'changed\n'
        user.PostMessageW(hwnd,0x10,0,0);child.wait(timeout=15);assert child.returncode==0
        receipt['process_tree']=tree.receipt(sample);assert receipt['process_tree']['coverage']=='COMPLETE' and not receipt['process_tree']['accounting']['active_processes']
        git_events=[p for p in receipt['process_tree']['processes'] if p.get('image','').casefold()=='git.exe'];assert not [p for p in git_events if p['created_100ns']<before_authority],'Git executed before explicit trust';assert any(p['created_100ns']>=before_authority for p in git_events)
        receipt['cases']=['PASS: actual cross-process UIA discovers roles/names/values and opens an untrusted repository only after separate explicit trust','PASS: stale and disabled native requests do not change Git state; modal tree excludes background Stage; real UIA Stage updates the exact index; ValuePattern edits and saves the Unicode commit draft without committing']
        receipt['verdict']='PASS'
    except Exception as error:
        receipt.update(verdict='FAIL',failure=repr(error),failure_traceback=traceback.format_exc())
        if hwnd and child and child.poll() is None:
            try:key(hwnd,0x10);capture('failure-window')
            except Exception as e:receipt['capture_error']=repr(e)
        raise
    finally:
        if child and child.poll() is None:
            try:
                if hwnd:
                    def ordinary():
                        current=native_title(hwnd,user)
                        return current=='MiniCon SrcMgr' or (current.startswith('MiniCon SrcMgr - '+repo.name+' | ') and current.endswith(' Ready'))
                    for _ in range(4):
                        if ordinary():break
                        try:key(hwnd,0x1B)
                        except Exception as error:
                            receipt.setdefault('cleanup_key_errors',[]).append(repr(error))
                            owner=w.DWORD();user.GetWindowThreadProcessId(hwnd,c.byref(owner))
                            assert owner.value==child.pid,'cleanup window no longer belongs to the launched process'
                            assert user.PostMessageW(hwnd,0x100,0x1B,1),'owned Escape cleanup was refused'
                            user.PostMessageW(hwnd,0x101,0x1B,0xC0000001)
                            receipt['cleanup_posted_escapes']=receipt.get('cleanup_posted_escapes',0)+1
                        time.sleep(0.1)
                    wait_entry(ordinary,'cleanup did not leave the nested modal path')
            except Exception as error:receipt['cleanup_readiness_error']=repr(error)
            if hwnd:user.PostMessageW(hwnd,0x10,0,0)
            try:child.wait(timeout=15)
            except sp.TimeoutExpired:receipt['forced_close']=True;child.terminate();child.wait(timeout=5)
        receipt['exit_code']=child.returncode if child else None
        try:
            if 'process_tree' not in receipt:receipt['process_tree']=tree.receipt(sample)
        except Exception as e:receipt['process_error']=repr(e)
        tree.close()
        (root/'receipt.json').write_text(json.dumps(receipt,indent=2),encoding='utf-8')
        if previous:user.SetForegroundWindow(previous)
    assert receipt['verdict']=='PASS';print(json.dumps({k:v for k,v in receipt.items() if k!='process_tree'}))

if __name__=='__main__':main()
