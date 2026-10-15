#!/usr/bin/env python3
"""Independently verify a full-pass synthetic 80k stream; retain each step."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[3]
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
cache = Path('/home/lupc/.cache/ez-sdr-audit/synthetic-80k-1011')
vectors = cache / 'long-pass'
proof_path = audit / 'repair-synthetic-80k-long-verification-1011.json'
assert not proof_path.exists(), 'Preserve prior evidence; use a new attempt'
def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as file:
        for block in iter(lambda: file.read(1048576), b''):
            digest.update(block)
    return digest.hexdigest()
def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()
manifest = json.loads((vectors / 'manifest.json').read_text())
assert manifest['reference_frames'] == 7338 and manifest['duration_seconds'] >= 800
assert sha(audit / 'generate_80k_long_pass.py') == manifest['generator_sha256']
assert sha(vectors / 'reference.cadu') == manifest['reference_sha256']
case = manifest['cases'][0]
assert sha(vectors / case['file']) == case['sha256']
previous_path = audit / 'repair-synthetic-80k-verification-1011.json'
previous = json.loads(previous_path.read_text())
assert previous['verification_state'] == 'independent_synthetic_80k_packets_images_verified'
for name, expected in previous['source_sha256'].items():
    assert sha(root / name) == expected, name
for check in previous['checks']:
    assert check['exit_code'] == 0 and sha(audit / check['log']) == check['log_sha256']
source_hashes = dict(previous['source_sha256'])
source_hashes.update({str(p.relative_to(root)): sha(p) for p in [audit / 'generate_80k_long_pass.py', Path(__file__)]})
proof = dict(verification_state='pending', started_utc=now(), generator=manifest, source_sha256=source_hashes,
    reused_unchanged_source_checks=dict(proof=str(previous_path.relative_to(audit)), sha256=sha(previous_path), checks=previous['checks']),
    steps=[], verification_boundary='Synthetic full-pass 80k using independently decoded public 72k packet/image content. Original off-air 80k and physical/field release gates remain open.')
proof_path.write_text(json.dumps(proof, indent=2) + '\n')
environment = dict(os.environ, EZ_80K_IMAGE_VECTORS=str(vectors))
for label, command in [
    ('native-reference', [sys.executable, 'audit/2026-10-07/verify_80k_native_reference.py', str(cache / 'reference-build/liblrpt.so'), str(vectors)]),
    ('rf', ['cargo', 'test', '-p', 'lrpt-decode', '--release', '--lib', 'synthetic_product_validation::independent_80k_rf_images_match_reference_across_impairments_and_partitions', '-j', '4', '--', '--exact', '--ignored', '--test-threads=1', '--nocapture'])]:
    log = evidence / f'synthetic-80k-long-{label}-1011.log'
    assert not log.exists()
    started = now()
    with log.open('w') as file:
        result = subprocess.run(command, cwd=root, env=environment, stdout=file, stderr=subprocess.STDOUT)
    step = dict(command=command, started_utc=started, finished_utc=now(), exit_code=result.returncode,
                log=str(log.relative_to(audit)), log_sha256=sha(log))
    proof['steps'].append(step)
    proof_path.write_text(json.dumps(proof, indent=2) + '\n')
    print(json.dumps(step), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
native = json.loads((vectors / 'native-reference.json').read_text())
assert native['reference_frames'] == 7338
proof['native_reference'] = native
comparisons = []
for chunk in [997, 16384]:
    output = vectors / f"decoded-{case['name']}-{chunk}"
    actual = json.loads((output / 'result.json').read_text())
    assert actual['corrected_frames'] == 7338 and actual['all_reference_frames_recovered']
    assert actual['selected_reference_order_matches'] and actual['input_bytes'] == case['samples'] * 8
    log = evidence / f'synthetic-80k-long-oracle-{chunk}-1011.log'
    assert not log.exists()
    command = [sys.executable, 'audit/2026-10-07/verify_public_image.py', str(vectors / 'native-reference.cadu'), str(output), str(output / 'oracle-native')]
    started = now()
    with log.open('w') as file:
        result = subprocess.run(command, cwd=root, stdout=file, stderr=subprocess.STDOUT)
    assert result.returncode == 0, log
    oracle = json.loads((output / 'oracle-native/verification.json').read_text())
    assert oracle['packet_bytes_verified'] and all(c['max_error'] <= 1 for c in oracle['comparisons'])
    comparisons.append(dict(chunk_size=chunk, command=command, started_utc=started, finished_utc=now(), exit_code=0,
        log=str(log.relative_to(audit)), log_sha256=sha(log), oracle=oracle,
        artifacts=[dict(path=str(p),sha256=sha(p)) for p in sorted(output.rglob('*')) if p.is_file()]))
proof['comparisons'] = comparisons
for name, expected in source_hashes.items():
    assert sha(root / name) == expected
proof.update(verification_state='independent_long_pass_synthetic_80k_packets_images_verified', finished_utc=now())
proof_path.write_text(json.dumps(proof, indent=2) + '\n')
print('Long-pass 80k: all 7338 CADUs, packet bytes and independent pixels verified at both partitions.', flush=True)
