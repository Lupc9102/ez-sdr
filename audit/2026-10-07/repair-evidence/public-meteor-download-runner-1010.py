#!/usr/bin/env python3
"""Refresh the public reference files without relying on expired /tmp data."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
cache = Path('/home/lupc/.cache/ez-sdr-audit/meteor-public-1010')
cache.mkdir(parents=True, exist_ok=True)
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
quota, period = (Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.01
baseline = json.loads((audit / 'public-meteor-reference.json').read_text())
manifest = {
    'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'cpu_max': [quota, period],
    'source_issue': baseline['source_issue'],
    'validation_boundary': 'Availability and file identity only. No new RF/profile/packet/image validation is claimed.',
    'artifacts': [],
}
manifest_path = audit / 'public-meteor-download-1010.json'
for artifact in baseline['artifacts']:
    if not artifact['download_url']:
        continue
    name = artifact['download_url'].rsplit('/', 1)[-1]
    target = cache / name
    partial = cache / (name + '.partial')
    headers = cache / (name + '.headers')
    command = [
        'curl', '--fail', '--location', '--silent', '--show-error',
        '--retry', '2', '--retry-delay', '2', '--max-time', '1800',
        '--limit-rate', '1M', '--header', 'Cache-Control: no-cache',
        '--dump-header', str(headers), '--output', str(partial), artifact['download_url'],
    ]
    record = {'kind': artifact['kind'], 'url': artifact['download_url'], 'command': command}
    result = subprocess.run(command)
    record['exit_code'] = result.returncode
    if result.returncode == 0:
        digest = hashlib.sha256()
        with partial.open('rb') as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(block)
        record.update(bytes=partial.stat().st_size, sha256=digest.hexdigest())
        record['matches_previous_identity'] = (record['bytes'], record['sha256']) == (artifact['bytes'], artifact['sha256'])
        if record['matches_previous_identity']:
            partial.replace(target)
            record['local_path'] = str(target)
    record['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    manifest['artifacts'].append(record)
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(record), flush=True)
    if result.returncode or not record.get('matches_previous_identity'):
        raise SystemExit(result.returncode or 1)
manifest['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
