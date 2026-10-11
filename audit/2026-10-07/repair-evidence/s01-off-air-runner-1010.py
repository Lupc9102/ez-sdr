#!/usr/bin/env python3
"""Compare the actual 72k RF path with independently decoded public data."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
cache = Path('/home/lupc/.cache/ez-sdr-audit/meteor-public-1010')
parser = argparse.ArgumentParser()
parser.add_argument('--full', action='store_true')
parser.add_argument('--attempt', type=int, default=1)
parser.add_argument('--slice-attempt', type=int, default=1)
args = parser.parse_args()
assert args.attempt > 0
assert args.slice_attempt > 0
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
quota, period = (Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.1
active = subprocess.run(['systemctl', '--user', 'is-active', '--quiet', 'ez-sdr-audit-r19-resume-1010.service'])
assert active.returncode != 0, 'Wait for the existing Cargo job to terminate'
r19 = json.loads((audit / 'repair-daemon-selection-verification.json').read_text())
assert r19['verification_state'] == 'software_verified'

def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()

prefix = json.loads((audit / 'public-meteor-prefix-1010.json').read_text())
downloads = json.loads((audit / 'public-meteor-download-1010.json').read_text())
reference = next(a for a in downloads['artifacts'] if a['kind'] == 'reference_cadu')
iq_path = Path(prefix['local_path'])
reference_path = Path(reference['local_path'])
assert prefix['matches_previous_prefix'] and prefix['complete_pair_prefix']
assert digest(iq_path) == prefix['sha256']
assert digest(reference_path) == reference['sha256']
source_paths = sorted((root / 'lrpt-decode/src').glob('*.rs'))
source_paths += [audit / 'verify_public_image.py', Path(__file__).resolve()]
source_hashes = {str(p.relative_to(root)): digest(p) for p in source_paths}
mode = 'full' if args.full else 'slice'
tag = f's01-off-air-{mode}-1010-attempt-{args.attempt}'
manifest_path = evidence / (tag + '.json')
assert not manifest_path.exists(), 'Use another attempt number to preserve prior evidence'
output = cache / tag
assert not output.exists(), 'Use a fresh output directory'
start = 0 if args.full else 180
seconds = (iq_path.stat().st_size + 576000 - 1) // 576000 if args.full else 20
environment = dict(os.environ, EZ_METEOR_CS8=str(iq_path), EZ_METEOR_CADU=str(reference_path),
                   EZ_METEOR_START_SECONDS=str(start), EZ_METEOR_SECONDS=str(seconds),
                   EZ_METEOR_VALIDATION_OUTPUT=str(output))
steps = [
    ('clippy', ['cargo', 'clippy', '-p', 'lrpt-decode', '--lib', '--tests', '-j', '1', '--', '-D', 'warnings']),
    ('rf', ['cargo', 'test', '-p', 'lrpt-decode', '--release', '--lib', 'tests::public_off_air_carrier_recovers_reference_payload_bytes', '-j', '1', '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']),
    ('oracle', [sys.executable, 'audit/2026-10-07/verify_public_image.py', str(output / 'reference.cadu'), str(output), str(output / 'oracle')]),
]
if args.full:
    previous_path = evidence / f's01-off-air-slice-1010-attempt-{args.slice_attempt}.json'
    previous = json.loads(previous_path.read_text())
    assert previous['source_sha256'] == source_hashes
    assert previous['packet_bytes_verified']
    assert len(previous['steps']) == 3 and all(s['exit_code'] == 0 for s in previous['steps'])
    for step in previous['steps']:
        assert digest(root / step['log']) == step['log_sha256']
    # The successful slice already compiled/linted precisely these sources.
    steps = steps[1:]
manifest = {
    'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'mode': mode, 'cpu_max': [quota, period], 'cargo_jobs': 1, 'test_threads': 1,
    'mission_profile': 'Oqpsk72', 'sample_rate_hz': 288000,
    'source_sha256': source_hashes,
    'iq_prefix_sha256': prefix['sha256'], 'reference_cadu_sha256': reference['sha256'],
    'input_recording_complete': False, 'environment': {k: v for k, v in environment.items() if k.startswith('EZ_METEOR_')},
    'steps': [],
    'verification_boundary': '72k accepted CADUs/packets/products only. Missing reference frames remain explicit. No 80k or physical-hardware release gate is closed.',
}
for name, command in steps:
    record = {'command': command, 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    log_path = evidence / (tag + '-' + name + '.log')
    with log_path.open('w') as log:
        result = subprocess.run(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
    record.update(exit_code=result.returncode, log=str(log_path.relative_to(root)),
                  log_sha256=digest(log_path), finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
    manifest['steps'].append(record)
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(record), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
    if name == 'rf':
        product = json.loads((output / 'result.json').read_text())
        assert product['selected_reference_order_matches'], 'RF/reference frame chronology differs'
        assert product['input_bytes'] == (iq_path.stat().st_size if args.full else 20 * 576000)
oracle = json.loads((output / 'oracle/verification.json').read_text())
assert oracle['packet_bytes_verified'] and oracle['packet_bytes_compared'] > 0
for path, expected in source_hashes.items():
    assert digest(root / path) == expected, f'Source changed during verification: {path}'
manifest.update(
    finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
    packet_bytes_verified=True, packet_bytes_compared=oracle['packet_bytes_compared'],
    reference_packet_sha256=oracle['reference_packet_sha256'],
    comparisons=oracle['comparisons'],
    recovered_frames=product['corrected_frames'],
    missing_reference_frames=len(product['reference_missing_frame_keys']),
    all_reference_frames_recovered=product['all_reference_frames_recovered'],
    input_bytes=product['input_bytes'],
    product_json=str(output / 'result.json'),
    artifacts=[{'path': str(p), 'sha256': digest(p)} for p in sorted(output.rglob('*')) if p.is_file()],
)
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({k: manifest[k] for k in ['mode', 'input_bytes', 'recovered_frames', 'missing_reference_frames', 'packet_bytes_verified', 'all_reference_frames_recovered']}))
