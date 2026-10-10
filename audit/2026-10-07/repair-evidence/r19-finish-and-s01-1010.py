#!/usr/bin/env python3
"""Finish the verified audio batch, then run satellite comparisons serially."""
import datetime
import json
from pathlib import Path
import subprocess
import sys
import time

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
worker = 'ez-sdr-audit-r19-resume-1010.service'
unit = 'ez-sdr-audit-finish-and-reference-1010.service'
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
quota, period = (Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.001

def run(command):
    return subprocess.run(command, cwd=root, check=True)

while True:
    status = subprocess.run(
        ['systemctl', '--user', 'show', worker, '-p', 'LoadState', '-p', 'ActiveState', '-p', 'Result'],
        capture_output=True, text=True,
    )
    # A bus error is not evidence that the build terminated.
    if status.returncode == 0:
        values = dict(line.split('=', 1) for line in status.stdout.splitlines() if '=' in line)
        if values.get('LoadState') == 'not-found' or values.get('ActiveState') in ['inactive', 'failed']:
            break
    time.sleep(30)
run(['systemctl', '--user', 'set-property', '--runtime', unit, 'CPUQuota=10%'])
quota, period = (Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.1
print('Build terminal; verify its recorded checks before finalization.', flush=True)
run([sys.executable, 'audit/2026-10-07/repair-evidence/r19-finalize-evidence-1010.py'])
proof = json.loads((audit / 'repair-daemon-selection-verification.json').read_text())
assert proof['verification_state'] == 'software_verified'
originals = ['audit/2026-10-07/REPORT.md', 'audit/2026-10-07/findings.json', 'audit/2026-10-07/validation.json']
run(['git', 'diff', '--quiet', '--', *originals])
staged = subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True)
assert not staged.strip(), 'Preserve existing staged changes; do not mix them into this batch'
owned = set(proof['source_sha256'])
assert all(p == 'README.md' or p.startswith(('ez-daemon/', 'ez-gui/', 'ez-proto/')) for p in owned)
owned.update([
    'audit/2026-10-07/REMEDIATION.md', 'audit/2026-10-07/remediation.json',
    'audit/2026-10-07/repair-daemon-selection-verification.json',
    'audit/2026-10-07/public-meteor-candidates-1010.json',
    'audit/2026-10-07/public-meteor-download-1010.json',
    'audit/2026-10-07/public-meteor-prefix-1010.json',
    'audit/2026-10-07/repair-evidence/public-meteor-download-runner-1010.py',
    'audit/2026-10-07/repair-evidence/public-meteor-prefix-runner-1010.py',
    'audit/2026-10-07/repair-evidence/public-meteor-prefix-1010.log',
])
for artifact in proof['artifacts']:
    path = audit / artifact['path']
    assert path.resolve().is_relative_to(evidence.resolve())
    assert path.name.startswith('r19-')
    owned.add(str(path.relative_to(root)))
assert all((root / p).is_file() for p in owned)
run(['git', 'add', '-f', '--', *sorted(owned)])
staged = set(subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True).splitlines())
assert staged <= owned and 'AGENTS.md' not in staged
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only', '--diff-filter=D'], cwd=root, text=True).strip()
# Raw compiler/test logs retain their original whitespace and recorded hashes.
checkable = sorted(path for path in staged if not path.endswith('.log'))
run(['git', 'diff', '--cached', '--check', '--', *checkable])
run(['git', 'commit', '-m', 'fix: select daemon RF audio and preserve independent clocks'])
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
(evidence / 'r19-commit-result-1010.json').write_text(json.dumps({
    'committed_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'commit': head, 'paths': sorted(staged), 'software_verified': 32,
    'remaining_findings': ['S01'], 'cpu_max': [quota, period],
}, indent=2) + '\n')
print('Committed verified audio repair; begin the reference slice.', flush=True)
run([sys.executable, 'audit/2026-10-07/repair-evidence/s01-off-air-runner-1010.py'])
print('Slice packet/image comparisons passed; begin the complete usable IQ prefix.', flush=True)
run([sys.executable, 'audit/2026-10-07/repair-evidence/s01-off-air-runner-1010.py', '--full'])
