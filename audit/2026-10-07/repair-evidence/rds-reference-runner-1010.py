#!/usr/bin/env python3
"""Validate the independent public MPX fixture after existing checks terminate."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
cache = Path('/home/lupc/.cache/ez-sdr-audit/radio-public-1010')
unit = 'ez-sdr-audit-rds-reference-1010.service'
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
cpu_path = Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max'
quota, period = cpu_path.read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.001


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command):
    return subprocess.run(command, cwd=root, check=True)


# This test-only file is outside the active LRPT source snapshot. Format it
# before the pending workspace format check, using only this waiting quota.
run(['rustfmt', '--edition', '2021', 'ez-gui/src/radio_rds.rs'])
workers = ['ez-sdr-audit-finish-and-reference-1010.service',
           'ez-sdr-audit-s01-finalize-1010.service']
while True:
    terminal = []
    for worker in workers:
        result = subprocess.run(['systemctl', '--user', 'show', worker, '-p', 'LoadState', '-p', 'ActiveState'],
                                capture_output=True, text=True)
        values = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
        terminal.append(result.returncode == 0 and (
            values.get('LoadState') == 'not-found' or values.get('ActiveState') in ['inactive', 'failed']))
    if all(terminal):
        break
    time.sleep(30)
run(['systemctl', '--user', 'set-property', '--runtime', unit, 'CPUQuota=9.9%'])
quota, period = cpu_path.read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.1
manifest_path = audit / 'repair-public-rds-verification-1010.json'
assert not manifest_path.exists(), 'Keep previous evidence; use a new attempt'
intake = json.loads((cache / 'prepared.json').read_text())
assert digest(Path(intake['local_path'])) == intake['sha256']
assert digest(Path(intake['wav_path'])) == intake['wav_sha256']
source_paths = ['ez-gui/src/radio_rds.rs', 'ez-gui/Cargo.toml', 'Cargo.lock',
                'audit/2026-10-07/repair-evidence/rds-reference-runner-1010.py']
source_hashes = {path: digest(root / path) for path in source_paths}
environment = dict(os.environ, EZ_RDS_REFERENCE_WAV=intake['wav_path'])
manifest = {
    'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'verification_state': 'pending', 'source_sha256': source_hashes,
    'reference': intake, 'cpu_max': [quota, period], 'cargo_jobs': 1, 'test_threads': 1,
    'resource_policy': 'During checks: 9.9% of one core plus the 0.1% waiting audio helper. Earlier RF and finalization stages are terminal. Nice 19, one Cargo job and one test thread.',
    'steps': [],
    'verification_boundary': 'Third-party MPX fixture and independently tested PI/PTY only. Specific off-air provenance is undocumented. No physical receiver, stereo separation, CTCSS or audio/disk release gate is closed.',
}
steps = [
    ('test', ['cargo', 'test', '-p', 'ez-gui', '--lib',
              'radio_rds::tests::public_reference_multiplex_matches_independent_redsea_metadata',
              '-j', '1', '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']),
    ('clippy', ['cargo', 'clippy', '-p', 'ez-gui', '--lib', '--tests', '-j', '1', '--', '-D', 'warnings']),
    ('format', ['cargo', 'fmt', '--all', '--', '--check']),
]
for name, command in steps:
    log_path = evidence / f'rds-reference-{name}-1010.log'
    assert not log_path.exists(), 'Keep prior logs'
    start = datetime.datetime.now(datetime.timezone.utc).isoformat()
    with log_path.open('w') as log:
        result = subprocess.run(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
    step = {'command': command, 'started_utc': start, 'exit_code': result.returncode,
            'log': str(log_path.relative_to(audit)), 'log_sha256': digest(log_path),
            'finished_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    manifest['steps'].append(step)
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(step), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
test_log = (evidence / 'rds-reference-test-1010.log').read_text()
assert 'test result: ok. 1 passed; 0 failed; 0 ignored;' in test_log
metric_line = next(line.split('RDS_REFERENCE_RESULT ', 1)[1] for line in test_log.splitlines()
                   if 'RDS_REFERENCE_RESULT ' in line)
metrics = json.loads(metric_line)
assert metrics['pi'] == 0x6201 and metrics['pty'] == 14 and metrics['valid_groups'] >= 2
assert metrics['input_samples'] == 134400 and metrics['sample_rate_hz'] == 192000
assert metrics['partitions'] == [1, 37, 997]

# Reuse the preceding graph refresh only if it demonstrably followed this edit.
satellite_proof = audit / 'repair-public-rf-verification-1010.json'
graph = root / 'graphify-out/graph.json'
graph_current = satellite_proof.exists() and graph.exists() and graph.stat().st_mtime >= (root / source_paths[0]).stat().st_mtime
if not graph_current:
    deleted = set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
    log_path = evidence / 'rds-reference-graph-1010.log'
    with log_path.open('w') as log:
        result = subprocess.run(['graphify', 'update', '.'], cwd=root, stdout=log, stderr=subprocess.STDOUT)
    for path in deleted:
        target = root / path
        if path.startswith('graphify-out/') and target.is_file():
            target.unlink()
    assert result.returncode == 0
    assert deleted == set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
    manifest['steps'].append({'command': ['graphify', 'update', '.'], 'exit_code': 0,
                              'log': str(log_path.relative_to(audit)), 'log_sha256': digest(log_path)})
else:
    manifest['preceding_graph_refresh'] = {
        'proof': str(satellite_proof.relative_to(audit)), 'proof_sha256': digest(satellite_proof),
        'graph_mtime': graph.stat().st_mtime, 'rds_source_mtime': (root / source_paths[0]).stat().st_mtime,
    }
for path, expected in source_hashes.items():
    assert digest(root / path) == expected, f'Source changed during verification: {path}'
assert digest(Path(intake['wav_path'])) == intake['wav_sha256']
ledger = json.loads((audit / 'remediation.json').read_text())
baseline = json.loads((audit / 'findings.json').read_text())['findings']
assert {item['id']: item['repair_gate'] for item in baseline} == {
    item['id']: item['original_repair_gate'] for item in ledger['findings']}
assert next(item for item in ledger['findings'] if item['id'] == 'S01')['status'] == 'open'
manifest.update(verification_state='independent_mpx_metadata_and_partitions_verified', metrics=metrics,
                finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
with (audit / 'REMEDIATION.md').open('a') as doc:
    doc.write('\n## Independent public RDS MPX fixture — 2026-10-10\n\n')
    doc.write(f"The pinned Redsea fixture (0.7 seconds, 192 kHz mono, SHA-256 {intake['sha256']}) produces PI 0x6201 and PTY 14 (Serious Classical), matching the independent upstream test. Our decoder validates {metrics['valid_groups']} groups. Whole-input and 1/37/997-sample partitions produce identical complete snapshots. The explicit recording test, GUI library/test Clippy and formatting pass; the graph refresh covers the new test. See repair-public-rds-verification-1010.json.\n\n")
    doc.write('This is an external MPX compatibility check. The upstream source does not explicitly document this specific fixture as off-air. Original physical receiver, stereo, CTCSS, audio/disk and 80k Meteor release requirements remain unverified; finding statuses and original repair gates are unchanged. Checks ran serially under 10% of one CPU core, nice 19, one Cargo job and one test thread.\n')
originals = [str(audit / name) for name in ['REPORT.md', 'findings.json', 'validation.json']]
run(['git', 'diff', '--quiet', '--', *originals])
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root).strip()
owned = {'ez-gui/src/radio_rds.rs', str(Path(__file__).resolve().relative_to(root)),
         str(manifest_path.relative_to(root)), 'audit/2026-10-07/REMEDIATION.md'}
owned.update(str(audit / step['log']) for step in manifest['steps'])
owned = {str(Path(path).relative_to(root)) if Path(path).is_absolute() else path for path in owned}
run(['git', 'add', '-f', '--', *sorted(owned)])
staged = set(subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True).splitlines())
assert staged <= owned and 'AGENTS.md' not in staged
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only', '--diff-filter=D'], cwd=root).strip()
run(['git', 'diff', '--cached', '--check', '--', *sorted(path for path in staged if not path.endswith('.log'))])
run(['git', 'commit', '-m', 'test: validate public RDS multiplex against independent station metadata'])
print('Independent RDS fixture evidence committed; physical/field gates remain open.', flush=True)
