#!/usr/bin/env python3
"""Resume saved reference evidence after disconnected progress pipes.

Do not rerun the costly successful RF test. Check every saved source/log hash
before running the independent oracle and completing its pending manifest.
"""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[3]
evidence = root / 'audit/2026-10-07/repair-evidence'

def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1048576), b''):
            value.update(block)
    return value.hexdigest()

def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()

path = evidence / 's01-off-air-full-1010-attempt-1.json'
manifest = json.loads(path.read_text())
assert 'finished_utc' not in manifest
assert len(manifest['steps']) == 1 and manifest['steps'][0]['exit_code'] == 0
for name, expected in manifest['source_sha256'].items():
    assert digest(root / name) == expected, name
for step in manifest['steps']:
    assert digest(root / step['log']) == step['log_sha256']
output = Path(manifest['environment']['EZ_METEOR_VALIDATION_OUTPUT'])
product = json.loads((output / 'result.json').read_text())
assert product['selected_reference_order_matches']
assert product['input_bytes'] == Path(manifest['environment']['EZ_METEOR_CS8']).stat().st_size
assert digest(Path(manifest['environment']['EZ_METEOR_CS8'])) == manifest['iq_prefix_sha256']
assert digest(Path(manifest['environment']['EZ_METEOR_CADU'])) == manifest['reference_cadu_sha256']
# Rerun only the fast independent oracle so the resumed command has its own
# measured time and exit status. The saved RF output is used unchanged.
command = [sys.executable, 'audit/2026-10-07/verify_public_image.py',
           str(output / 'reference.cadu'), str(output), str(output / 'oracle')]
log = evidence / 's01-off-air-full-1011-resumed-oracle.log'
assert not log.exists()
started = now()
with log.open('w') as target:
    result = subprocess.run(command, cwd=root, stdout=target, stderr=subprocess.STDOUT)
assert result.returncode == 0, log
oracle = json.loads((output / 'oracle/verification.json').read_text())
assert oracle['packet_bytes_verified'] and oracle['packet_bytes_compared'] > 0
assert all(c['max_error'] <= 1 for c in oracle['comparisons'])
manifest['steps'].append(dict(command=command, started_utc=started, finished_utc=now(),
                              exit_code=result.returncode, log=str(log.relative_to(root)), log_sha256=digest(log)))
manifest.update(finished_utc=now(), packet_bytes_verified=True,
                packet_bytes_compared=oracle['packet_bytes_compared'],
                reference_packet_sha256=oracle['reference_packet_sha256'], comparisons=oracle['comparisons'],
                recovered_frames=product['corrected_frames'],
                missing_reference_frames=len(product['reference_missing_frame_keys']),
                all_reference_frames_recovered=product['all_reference_frames_recovered'],
                input_bytes=product['input_bytes'], product_json=str(output / 'result.json'),
                artifacts=[dict(path=str(p), sha256=digest(p)) for p in sorted(output.rglob('*')) if p.is_file()],
                resumption=dict(reason='Progress pipe closed after RF test passed (helper exited 120). Saved RF sources, input and log hashes verified before continuation.',
                                runner_sha256=digest(Path(__file__)), full_cpu_authorized=True))
path.write_text(json.dumps(manifest, indent=2) + '\n')
print('Full RF packet/image oracle passed and saved evidence finalized.')
