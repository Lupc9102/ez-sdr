#!/usr/bin/env python3
"""Retain only the independently identified usable prefix of the public ZIQ."""
import datetime
import hashlib
import json
from pathlib import Path
import struct
import subprocess

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
cache = Path('/home/lupc/.cache/ez-sdr-audit/meteor-public-1010')
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
quota, period = (Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.01
baseline = json.loads((audit / 'public-meteor-reference.json').read_text())
downloads = json.loads((audit / 'public-meteor-download-1010.json').read_text())
download = next(a for a in downloads['artifacts'] if a['kind'] == 'baseband_ziq')
assert download['matches_previous_identity'] and download['exit_code'] == 0
expected = next(a for a in baseline['artifacts'] if a['kind'] == 'decompressed_cs8_prefix')
source_path = Path(download['local_path'])
partial = cache / 'meteor-72k.cs8.partial'
target = cache / 'meteor-72k.cs8'
log_path = audit / 'repair-evidence/public-meteor-prefix-1010.log'
command = ['zstd', '--decompress', '--stdout', '--no-progress']
with source_path.open('rb', buffering=0) as source:
    # The pinned SatDump writer stores an eight-byte rate and annotation size:
    # four-byte signature, one-byte compression/depth, u64 rate, u64 length.
    signature, compressed, bits, sample_rate, annotation_length = struct.unpack('<4sBBQQ', source.read(22))
    assert (signature, compressed, bits, sample_rate, annotation_length) == (b'ZIQ_', 1, 8, 288000, 0)
    with partial.open('wb') as output, log_path.open('wb') as log:
        result = subprocess.run(command, stdin=source, stdout=output, stderr=log)
digest = hashlib.sha256()
with partial.open('rb') as stream:
    for block in iter(lambda: stream.read(1024 * 1024), b''):
        digest.update(block)
record = {
    'finished_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'cpu_max': [quota, period], 'source_path': str(source_path),
    'source_sha256': download['sha256'], 'header_bytes': 22,
    'format_reference': baseline['ziq_format_reference'],
    'sample_rate_hz': sample_rate, 'command': command,
    'exit_code': result.returncode, 'stderr_log': str(log_path.relative_to(root)),
    'bytes': partial.stat().st_size, 'sha256': digest.hexdigest(),
    'recording_complete': False,
    'validation_boundary': 'The compressed recording ends prematurely. A matching complete-pair decoded prefix is usable; it is not a certified complete recording or a new decoder comparison.',
}
record['complete_pair_prefix'] = record['bytes'] % 2 == 0
record['matches_previous_prefix'] = (record['bytes'], record['sha256']) == (expected['bytes'], expected['sha256'])
valid = (result.returncode == 1 and 'premature end' in log_path.read_text().lower()
         and record['complete_pair_prefix'] and record['matches_previous_prefix'])
if valid:
    partial.replace(target)
    record['local_path'] = str(target)
(audit / 'public-meteor-prefix-1010.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record), flush=True)
assert valid, 'The restored prefix does not match the previously identified usable data'
