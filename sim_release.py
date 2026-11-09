import hashlib, json, shutil, subprocess, sys
from pathlib import Path
ROOT = Path('.').resolve()
work = ROOT / 'target/tmp/relsim'
if work.exists():
    shutil.rmtree(work)
(work / 'cells').mkdir(parents=True)
(work / 'signed' / 'cells' / 'win-x86_64').mkdir(parents=True)
(work / 'signed' / 'cells' / 'win-aarch64').mkdir(parents=True)
binary = ROOT / 'target/debug/minicon-srcmgr.exe'
readme = work / 'first-run.txt'
readme.write_text('sim\n', encoding='utf-8')
sha = 'a' * 40
cells = {'lnx-x86_64':'linux','lnx-aarch64':'linux','win-x86_64':'windows','win-aarch64':'windows','osx-x86_64':'macos','osx-aarch64':'macos'}
for cell, osname in cells.items():
    out = work / 'cells' / ('cell-' + cell) / 'package' / cell
    out.mkdir(parents=True)
    subprocess.run([sys.executable,'scripts/package_cell.py','--cell',cell,'--binary',str(binary),'--readme',str(readme),
                    '--version','0.1.2','--source-sha',sha,'--runtime-status','passed',
                    '--signing','blocked' if osname=='windows' else 'unsigned','--out',str(out)], check=True, cwd=ROOT)
    (out / 'runtime-receipt.json').write_text(json.dumps({'schema':1,'kind':'srcmgr-cell-runtime','status':'passed'}), encoding='utf-8')
for cell in ('win-x86_64','win-aarch64'):
    exe = work / 'signed' / 'cells' / cell / 'minicon-srcmgr.exe'
    body = (b'MZ' + cell.encode()) * 40
    exe.write_bytes(body)
    digest = hashlib.sha256(body).hexdigest()
    insp = {
        'schema_version': 2, 'file_name': 'minicon-srcmgr.exe', 'sha256': digest, 'size_bytes': len(body),
        'status': 'Valid', 'product_name': 'MiniCon SrcMgr', 'product_version': '0.1.2',
        'signer': {'subject': 'CN=PARTNERNET SOFTWARE PTY LTD, O=PARTNERNET SOFTWARE PTY LTD, C=AU',
                   'issuer': 'CN=Microsoft ID Verified CS AOC CA 03, O=Microsoft Corporation, C=US',
                   'thumbprint': 'A'*40, 'not_before': '2026-11-08T00:00:00Z', 'not_after': '2026-11-11T00:00:00Z'},
        'timestamp_certificate': {'subject': 'CN=Microsoft Public RSA Time Stamping Authority', 'issuer': 'CN=Microsoft Public RSA Timestamping CA 2020', 'thumbprint': 'B'*40},
    }
    (work / 'signed' / f'inspection-{cell}.json').write_text(json.dumps(insp), encoding='utf-8')
(work / 'signed' / 'signing-receipt.json').write_text(json.dumps({'release_eligible': True}), encoding='utf-8')
print('staged')
