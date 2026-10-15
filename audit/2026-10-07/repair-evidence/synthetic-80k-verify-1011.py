#!/usr/bin/env python3
"""Complete synthetic 80k packet/pixel comparisons without relabeling off-air gates."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[3]
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
cache = Path('/home/lupc/.cache/ez-sdr-audit/synthetic-80k-1011')
vectors = cache / 'vectors'
def digest(path):
    sha = hashlib.sha256()
    with path.open('rb') as file:
        for block in iter(lambda: file.read(1048576), b''):
            sha.update(block)
    return sha.hexdigest()
manifest = json.loads((vectors / 'manifest.json').read_text())
assert digest(audit / 'generate_80k_image_vectors.py') == manifest['generator_sha256']
assert digest(vectors / 'reference.cadu') == manifest['reference_sha256']
for case in manifest['cases']:
    assert digest(vectors / case['file']) == case['sha256']
native = json.loads((vectors / 'native-reference.json').read_text())
assert native['reference_frames'] == manifest['reference_frames'] == 175
assert digest(audit / 'verify_80k_native_reference.py') == native['adapter_sha256']
assert digest(cache / 'reference-build/liblrpt.so') == native['native_library_sha256']
reference = (vectors / 'native-reference.cadu').read_bytes()
assert hashlib.sha256(reference).hexdigest() == native['reference_sha256']
frames = [reference[i:i + 1024] for i in range(0, len(reference), 1024)]
def key(frame):
    return [frame[5] & 63, int.from_bytes(frame[6:9], 'big')]
comparisons = []
for case in manifest['cases']:
    for chunk_size in [997, 16384]:
        output = vectors / f"decoded-{case['name']}-{chunk_size}"
        actual = json.loads((output / 'result.json').read_text())
        assert actual['mission_profile'] == 'Oqpsk80Interleaved'
        assert actual['sample_rate_hz'] == 320000
        accepted = {tuple(k) for k in actual['observed_frame_keys']}
        selected = [frame for frame in frames if tuple(key(frame)) in accepted]
        assert [key(frame) for frame in selected] == actual['observed_frame_keys']
        assert b''.join(frame[4:896] for frame in selected) == (output / 'vcdu.bin').read_bytes()
        independent = output / 'native-reference-selected.cadu'
        independent.write_bytes(b''.join(selected))
        log = evidence / f"synthetic-80k-oracle-{case['name']}-{chunk_size}-1011.log"
        assert not log.exists()
        command = [sys.executable, 'audit/2026-10-07/verify_public_image.py', str(independent), str(output), str(output / 'oracle-native')]
        started = datetime.datetime.now(datetime.timezone.utc).isoformat()
        with log.open('w') as file:
            result = subprocess.run(command, cwd=root, stdout=file, stderr=subprocess.STDOUT)
        assert result.returncode == 0, log
        oracle = json.loads((output / 'oracle-native/verification.json').read_text())
        assert oracle['packet_bytes_verified']
        assert oracle['comparisons'] and all(c['max_error'] <= 1 for c in oracle['comparisons'])
        comparisons.append(dict(case=case['name'], chunk_size=chunk_size, recovered_frames=len(selected), missing_frames=175 - len(selected),
            command=command, started_utc=started, exit_code=0, log=str(log.relative_to(audit)), log_sha256=digest(log), oracle=oracle,
            output_artifacts=[dict(path=str(p), sha256=digest(p)) for p in sorted(output.rglob('*')) if p.is_file()]))
proof = dict(verification_state='independent_synthetic_80k_packets_images_verified', finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
    generator=manifest, native_reference=native, comparisons=comparisons,
    source_sha256={str(p.relative_to(root)): digest(p) for p in [*sorted((root / 'lrpt-decode/src').glob('*.rs')), audit / 'generate_80k_image_vectors.py', audit / 'generate_oqpsk_vectors.py', audit / 'verify_80k_native_reference.py', audit / 'verify_public_image.py', Path(__file__)]},
    verification_boundary='Synthetic 80k RF carrying independently decoded public 72k image content. The native-C transport reference sees ideal transmitted bits, while the actual app receives independently pulse-shaped impaired RF samples. Original off-air 80k and physical/field release gates remain unverified.')
path = audit / 'repair-synthetic-80k-verification-1011.json'
assert not path.exists()
path.write_text(json.dumps(proof, indent=2) + '\n')
print(json.dumps([dict(case=c['case'],chunk=c['chunk_size'],frames=c['recovered_frames'],packets=c['oracle']['packet_bytes_compared'],pixels=sum(p['pixels'] for p in c['oracle']['comparisons'])) for c in comparisons],indent=2))
