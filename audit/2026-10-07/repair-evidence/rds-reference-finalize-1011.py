#!/usr/bin/env python3
"""Finish a passed RDS fixture without repeating its test or GUI checks."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[3]
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def run(command):
    return subprocess.run(command, cwd=root, check=True)

manifest_path = audit / 'repair-public-rds-verification-1010.json'
manifest = json.loads(manifest_path.read_text())
assert manifest['verification_state'] == 'pending'
assert len(manifest['steps']) == 1 and manifest['steps'][0]['exit_code'] == 0
source_hashes = manifest['source_sha256']
source_paths = list(source_hashes)
for name, expected in source_hashes.items():
    assert digest(root / name) == expected, name
for step in manifest['steps']:
    assert digest(audit / step['log']) == step['log_sha256']
intake = manifest['reference']
assert digest(Path(intake['local_path'])) == intake['sha256']
assert digest(Path(intake['wav_path'])) == intake['wav_sha256']
# GUI library/test Clippy covers the unchanged RDS fixture too. The verified
# audio repair ran after this edit and records the complete command and log.
audio_path = audit / 'repair-audio-driver-verification-1010-attempt-2.json'
audio = json.loads(audio_path.read_text())
assert audio['verification_state'] == 'real_shared_audio_driver_lifecycle_verified'
for name, expected in audio['source_sha256'].items():
    assert digest(root / name) == expected, name
for label in ['clippy', 'fmt']:
    step = next(step for step in audio['steps'] if label in step['command'])
    assert step['exit_code'] == 0
    assert datetime.datetime.fromisoformat(step['started_utc']).timestamp() >= (root / 'ez-gui/src/radio_rds.rs').stat().st_mtime
    assert digest(audit / step['log']) == step['log_sha256']
    manifest['steps'].append(step)
manifest['resumption'] = dict(reason='Progress pipe closed after successful fixture test; original source/input/log hashes verified.',
    runner_sha256=digest(Path(__file__)), reused_check_proof=str(audio_path.relative_to(audit)),
    reused_check_proof_sha256=digest(audio_path), full_cpu_authorized=True)
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
    doc.write('This is an external MPX compatibility check. The upstream source does not explicitly document this specific fixture as off-air. Original physical receiver, stereo, CTCSS, audio/disk and 80k Meteor release requirements remain unverified; finding statuses and original repair gates are unchanged. The fixture test ran under the former 10% CPU allowance. Remaining GUI Clippy/format checks were reused from the verified audio repair after full CPU access was authorized on 2026-10-11.\n')
originals = [str(audit / name) for name in ['REPORT.md', 'findings.json', 'validation.json']]
run(['git', 'diff', '--quiet', '--', *originals])
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root).strip()
owned = {'ez-gui/src/radio_rds.rs', str(Path(__file__).resolve().relative_to(root)),
         str(manifest_path.relative_to(root)), 'audit/2026-10-07/REMEDIATION.md',
         'audit/2026-10-07/repair-evidence/rds-reference-runner-1010.py'}
owned.update(str(audit / step['log']) for step in manifest['steps'])
owned = {str(Path(path).relative_to(root)) if Path(path).is_absolute() else path for path in owned}
run(['git', 'add', '-f', '--', *sorted(owned)])
staged = set(subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True).splitlines())
assert staged <= owned and 'AGENTS.md' not in staged
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only', '--diff-filter=D'], cwd=root).strip()
run(['git', 'diff', '--cached', '--check', '--', *sorted(path for path in staged if not path.endswith('.log'))])
run(['git', 'commit', '-m', 'test: validate public RDS multiplex against independent station metadata'])
print('Independent RDS fixture evidence committed; physical/field gates remain open.', flush=True)
