import json, subprocess, sys, shutil
from pathlib import Path
ROOT = Path('.').resolve()
root = ROOT / 'target/tmp/sixsim'
if root.exists():
    shutil.rmtree(root)
(root / 'cells').mkdir(parents=True)
binary = ROOT / 'target/debug' / ('minicon-srcmgr.exe' if sys.platform == 'win32' else 'minicon-srcmgr')
sha = subprocess.run(['git','rev-parse','HEAD'], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
cells = {'lnx-x86_64':'linux','lnx-aarch64':'linux','win-x86_64':'windows','win-aarch64':'windows','osx-x86_64':'macos','osx-aarch64':'macos'}
readme = root / 'README.txt'
readme.write_text('sim\n', encoding='utf-8')
for cell, osname in cells.items():
    out = root / 'cells' / ('cell-' + cell)
    out.mkdir(parents=True)
    done = subprocess.run([sys.executable,'scripts/package_cell.py','--cell',cell,'--binary',str(binary),'--readme',str(readme),'--version','0.1.0','--source-sha',sha,'--runtime-status','passed','--signing','blocked' if osname=='windows' else 'unsigned','--out',str(out)], cwd=ROOT, capture_output=True, text=True)
    if done.returncode != 0:
        print('FAIL', cell, done.stdout[-300:], done.stderr[-500:]); sys.exit(1)
    (out / 'runtime-receipt.json').write_text(json.dumps({'schema':1,'kind':'srcmgr-cell-runtime','status':'passed'}), encoding='utf-8')
print('staged')
