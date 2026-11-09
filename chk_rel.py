import hashlib, json, zipfile
from pathlib import Path
out = Path('target/tmp/relsim/out')
z = zipfile.ZipFile(out / 'minicon-srcmgr-0.1.2-win-x86_64.zip')
print(sorted(z.namelist()))
body = z.read('minicon-srcmgr.exe')
print('exe sha inside zip', hashlib.sha256(body).hexdigest()[:16], 'len', len(body))
receipt = json.loads((out / 'cell-receipt-win-x86_64.json').read_text())
print('signing', receipt['signing'], 'version', receipt['product_version'])
print('sums has', len((out/'SHA256SUMS.txt').read_text().strip().splitlines()), 'lines')
