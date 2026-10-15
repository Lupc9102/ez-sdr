#!/usr/bin/env python3
"""Serialized focused R19 verification; requires the user's 10% CPU cap."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys

root = Path('/home/lupc/Documents/ez-sdr')
out = root / 'audit/2026-10-07/repair-evidence'
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
quota, period = (Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) / int(period) <= 0.1

steps = [
    ('r19-rational-clock-tests-1010', ['cargo', 'test', '-p', 'ez-daemon', '--lib', 'audio_rate::clock_tests::', '-j', '1', '--', '--test-threads=1', '--nocapture']),
    ('r19-state-configuration-tests-1010', ['cargo', 'test', '-p', 'ez-daemon', '--lib', 'state::tests::audio_', '-j', '1', '--', '--test-threads=1', '--nocapture']),
    ('r19-protocol-configuration-tests-1010', ['cargo', 'test', '-p', 'ez-proto', '--lib', 'messages::audio_configuration_tests::', '-j', '1', '--', '--test-threads=1', '--nocapture']),
]
retry = '--retry-state' in sys.argv
if retry:
    steps = [(name + '-retry', command) for name, command in steps[1:]]
manifest = {'cpu_max': [quota, period], 'test_threads': 1, 'cargo_jobs': 1, 'steps': []}
manifest_path = out / ('r19-controls-clock-results-1010-retry.json' if retry else 'r19-controls-clock-results-1010.json')
for name, command in steps:
    record = {'command': command, 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'log': str((out / (name + '.log')).relative_to(root))}
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    record.update(exit_code=result.returncode, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), log_sha256=hashlib.sha256((out / (name + '.log')).read_bytes()).hexdigest())
    manifest['steps'].append(record)
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(record), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
