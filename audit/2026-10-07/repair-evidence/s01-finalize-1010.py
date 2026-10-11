#!/usr/bin/env python3
"""Record only completed 72k comparisons; preserve the unresolved 80k gate."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
worker = 'ez-sdr-audit-finish-and-reference-1010.service'
unit = 'ez-sdr-audit-s01-finalize-1010.service'


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def run(command):
    return subprocess.run(command, cwd=root, check=True)


group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
cpu_path = Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max'
quota, period = cpu_path.read_text().split() if cpu_path.exists() else ('unavailable', 'unavailable')
# Full CPU use was explicitly authorized on 2026-10-11. Prior jobs are terminal.
proof_path = audit / 'repair-public-rf-verification-1010.json'
assert not proof_path.exists(), 'Preserve prior evidence; do not finalize twice'
originals = [audit / name for name in ['REPORT.md', 'findings.json', 'validation.json']]
run(['git', 'diff', '--quiet', '--', *map(str, originals)])
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root).strip()
baseline = json.loads((audit / 'findings.json').read_text())['findings']
ledger = json.loads((audit / 'remediation.json').read_text())
gates = {item['id']: item['repair_gate'] for item in baseline}
assert gates == {item['id']: item['original_repair_gate'] for item in ledger['findings']}
assert sum(item['status'] == 'software_verified' for item in ledger['findings']) == 32
assert next(item for item in ledger['findings'] if item['id'] == 'S01')['status'] == 'open'

records = []
pending_copies = []
owned = {
    'lrpt-decode/src/lib.rs', 'lrpt-decode/src/off_air_validation.rs',
    'audit/2026-10-07/verify_public_image.py',
    'audit/2026-10-07/repair-evidence/s01-off-air-runner-1010.py',
    'audit/2026-10-07/repair-evidence/s01-finalize-1010.py',
    'audit/2026-10-07/repair-evidence/reference-resume-1011.py',
    'audit/2026-10-07/repair-evidence/r19-commit-result-1010.json',
}
for mode, expected_steps in [('slice', 3), ('full', 2)]:
    manifest_path = evidence / f's01-off-air-{mode}-1010-attempt-1.json'
    manifest = json.loads(manifest_path.read_text())
    assert manifest.get('finished_utc') and manifest['mode'] == mode
    assert len(manifest['steps']) == expected_steps
    assert all(step['exit_code'] == 0 for step in manifest['steps'])
    assert manifest['packet_bytes_verified'] and manifest['packet_bytes_compared'] > 0
    assert manifest['comparisons'] and all(c['max_error'] <= 1 for c in manifest['comparisons'])
    for path, expected in manifest['source_sha256'].items():
        assert digest(root / path) == expected, f'Source changed: {path}'
    for step in manifest['steps']:
        log = root / step['log']
        assert log.resolve().is_relative_to(evidence.resolve())
        assert digest(log) == step['log_sha256']
        owned.add(step['log'])
    for artifact in manifest['artifacts']:
        assert digest(Path(artifact['path'])) == artifact['sha256']
    product_path = Path(manifest['product_json'])
    product = json.loads(product_path.read_text())
    assert product['selected_reference_order_matches']
    oracle_path = product_path.parent / 'oracle/verification.json'
    oracle = json.loads(oracle_path.read_text())
    assert oracle['packet_bytes_verified']
    assert oracle['reference_packet_sha256'] == manifest['reference_packet_sha256']
    copies = []
    for name, source in [('product', product_path), ('oracle', oracle_path)]:
        target = evidence / f's01-off-air-{mode}-1010-{name}.json'
        assert not target.exists()
        pending_copies.append((source, target))
        owned.add(str(target.relative_to(root)))
        copies.append({'path': str(target.relative_to(audit)), 'sha256': digest(source)})
    owned.add(str(manifest_path.relative_to(root)))
    records.append({'mode': mode, 'manifest': str(manifest_path.relative_to(audit)),
                    'manifest_sha256': digest(manifest_path), 'summaries': copies,
                    'input_bytes': manifest['input_bytes'], 'recovered_frames': manifest['recovered_frames'],
                    'missing_reference_frames': manifest['missing_reference_frames'],
                    'all_reference_frames_recovered': manifest['all_reference_frames_recovered'],
                    'packet_bytes_compared': manifest['packet_bytes_compared'],
                    'comparisons': manifest['comparisons']})
assert records[0]['input_bytes'] == 20 * 576000
assert records[1]['input_bytes'] == json.loads((audit / 'public-meteor-prefix-1010.json').read_text())['bytes']

checks = []
deleted = set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
for name, command in [('format', ['cargo', 'fmt', '--all', '--', '--check']),
                      ('graph', ['graphify', 'update', '.'])]:
    log_path = evidence / f's01-final-{name}-1010.log'
    with log_path.open('w') as log:
        result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    if name == 'graph':
        for path in deleted:
            target = root / path
            if path.startswith('graphify-out/') and target.is_file():
                target.unlink()
    assert result.returncode == 0, f'{name} failed; inspect {log_path}'
    checks.append({'command': command, 'exit_code': result.returncode,
                   'log': str(log_path.relative_to(audit)), 'log_sha256': digest(log_path)})
    owned.add(str(log_path.relative_to(root)))
assert deleted == set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
for path, expected in manifest['source_sha256'].items():
    assert digest(root / path) == expected
for source, target in pending_copies:
    target.write_bytes(source.read_bytes())

proof = {
    'verification_state': '72k_accepted_frame_packet_image_comparisons_passed',
    'finished_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'source_sha256': manifest['source_sha256'], 'comparisons': records,
    'finalizer_sha256': digest(Path(__file__).resolve()),
    'checks': checks, 'cpu_max': [quota, period], 'cargo_jobs': 1, 'test_threads': 1,
    'resource_policy': 'Saved RF tests ran within the former combined 10% CPU allowance. Full CPU use was explicitly authorized for resumed oracle checks and finalization on 2026-10-11.',
    'verification_boundary': 'Actual RF decoder accepted CADUs, completed packet bytes and images compared against original independent reference CADUs and a separate Python/Pillow oracle. Missing frames are explicit. The source recording has a truncated compressed tail. Interleaved 80k and physical hardware/audio/disk gates remain unverified; S01 stays open.',
}
proof_path.write_text(json.dumps(proof, indent=2) + '\n')
item = next(item for item in ledger['findings'] if item['id'] == 'S01')
item['regressions'].append('Explicit Oqpsk72 actual-RF slice and full usable IQ prefix: byte-identical completed packets and independent Pillow/CDS-time image comparisons; missing reference frames reported. See repair-public-rf-verification-1010.json.')
(audit / 'remediation.json').write_text(json.dumps(ledger, indent=2) + '\n')
full = records[1]
pixels = sum(c['pixels'] for c in full['comparisons'])
with (audit / 'REMEDIATION.md').open('a') as doc:
    doc.write('\n## Actual public 72k RF packet/image comparison — 2026-10-10\n\n')
    doc.write(f"The 20-second slice and complete usable 522,059,776-byte IQ prefix pass packet-byte and image comparisons from the actual RF decoder. The full prefix recovers {full['recovered_frames']:,} corrected frames and compares {full['packet_bytes_compared']:,} completed packet bytes and {pixels:,} image pixels against the independent original CADUs and Python/Pillow/CDS-time oracle. Maximum pixel error remains at most one. {full['missing_reference_frames']:,} reference frames are missing and are not counted as recovered. The compressed recording is truncated.\n\n")
    doc.write('Clippy, optimized actual-RF tests, formatting and AST graph refresh pass. Source hashes, commands, full-frame coverage and comparison summaries are in repair-public-rf-verification-1010.json. The saved RF tests used the former 10% CPU allowance; resumed oracle checks and finalization used the full CPU access authorized on 2026-10-11. User deletions and all 33 original repair gates are preserved. The ledger remains at 32 software-verified findings with S01 open: interleaved 80k off-air comparison and original physical hardware/audio/disk release checks are still unverified.\n')
assert gates == {item['id']: item['original_repair_gate'] for item in ledger['findings']}
owned.update(str(p.relative_to(root)) for p in [proof_path, audit / 'remediation.json', audit / 'REMEDIATION.md'])
run(['git', 'diff', '--quiet', '--', *map(str, originals)])
run(['git', 'add', '-f', '--', *sorted(owned)])
staged = set(subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True).splitlines())
assert staged <= owned and 'AGENTS.md' not in staged
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only', '--diff-filter=D'], cwd=root).strip()
run(['git', 'diff', '--cached', '--check', '--', *sorted(path for path in staged if not path.endswith('.log'))])
run(['git', 'commit', '-m', 'test: compare actual Meteor RF packets and images with independent reference'])
print('72k evidence committed; S01/80k and physical release gates remain open.', flush=True)
