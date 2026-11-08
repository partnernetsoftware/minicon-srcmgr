import json, subprocess, sys, shutil
from pathlib import Path
ROOT = Path('.').resolve()
root = ROOT / 'target/tmp/sixsim'
if root.exists():
    shutil.rmtree(root)
sha = subprocess.run(['git','rev-parse','HEAD'], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
cells = {'lnx-x86_64':'linux','lnx-aarch64':'linux','win-x86_64':'windows','win-aarch64':'windows','osx-x86_64':'macos','osx-aarch64':'macos'}
def write(cell, osname, nested):
    base = root / 'cells' / ('cell-' + cell)
    where = base / 'package' / cell if nested else base
    where.mkdir(parents=True)
    receipt = {
        'schema': 1, 'kind': 'srcmgr-cell-archive', 'cell': cell, 'os': osname,
        'arch': 'x86_64' if 'x86_64' in cell else 'aarch64',
        'source_sha': sha, 'runtime_court': 'passed',
        'signing': 'blocked' if osname == 'windows' else 'unsigned',
        'assets': {'a.zip': {'sha256': 'a' * 64, 'bytes': 1}},
    }
    (where / 'cell-receipt.json').write_text(json.dumps(receipt), encoding='utf-8')
    (where / 'runtime-receipt.json').write_text(json.dumps({'status': 'passed'}), encoding='utf-8')
for cell, osname in cells.items():
    write(cell, osname, nested=(cell == 'win-x86_64'))
print('staged')