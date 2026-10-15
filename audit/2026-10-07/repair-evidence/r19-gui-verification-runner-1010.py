#!/usr/bin/env python3
"""Verify R19 consumers serially without exceeding 10% of one CPU core."""
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

def git_paths(arguments):
    return subprocess.check_output(['git', *arguments], cwd=root, text=True).splitlines()

paths = git_paths(['diff', '--name-only', '--', '*.rs']) + git_paths(['ls-files', '--others', '--exclude-standard', '--', 'ez-daemon/src', 'ez-proto/src', 'ez-gui/src'])
sources = {path: hashlib.sha256((root / path).read_bytes()).hexdigest() for path in sorted(set(paths)) if path.endswith('.rs') and (root / path).is_file()}
deleted_graph_files = git_paths(['diff', '--name-only', '--diff-filter=D', '--', 'graphify-out/'])
steps = [
    ('r19-gui-workflow-tests-1010', ['cargo', 'test', '-p', 'ez-gui', '--lib', 'source_manager::tests::daemon_audio_', '-j', '1', '--', '--test-threads=1', '--nocapture']),
    ('r19-gui-center-retune-test-1010', ['cargo', 'test', '-p', 'ez-gui', '--lib', 'source_manager::tests::daemon_center_following_retunes_absolute_audio_even_when_offset_stays_zero', '-j', '1', '--', '--test-threads=1', '--nocapture']),
    ('r19-gui-playback-tests-1010', ['cargo', 'test', '-p', 'ez-gui', '--lib', 'audio_resampler::tests::', '-j', '1', '--', '--test-threads=1', '--nocapture']),
    ('r19-all-target-clippy-1010', ['cargo', 'clippy', '-p', 'ez-daemon', '-p', 'ez-gui', '-p', 'ez-proto', '--all-targets', '-j', '1', '--', '-D', 'warnings']),
    ('r19-default-build-1010', ['cargo', 'build', '-p', 'ez-daemon', '-p', 'ez-gui', '-j', '1']),
    ('r19-format-check-1010', ['cargo', 'fmt', '--all', '--', '--check']),
    ('r19-state-format-check-1010', ['rustfmt', '--edition', '2021', '--check', 'ez-daemon/src/audio_configuration_tests.rs']),
    ('r19-graph-update-1010', ['graphify', 'update', '.']),
]
retry = '--retry-controls' in sys.argv
if retry:
    # Only the GUI control branch changed. Keep the successful playback run
    # and its original source hashes/log; repeat the affected control tests.
    steps = [(name + '-retry', command) for index, (name, command) in enumerate(steps) if index != 2]
manifest = {'cpu_max': [quota, period], 'test_threads': 1, 'cargo_jobs': 1, 'source_sha256': sources, 'steps': []}
manifest_path = out / ('r19-gui-verification-results-1010-retry.json' if retry else 'r19-gui-verification-results-1010.json')
if '--resume-retry' in sys.argv:
    assert retry, 'Resume applies only to the verified control retry'
    previous = json.loads(manifest_path.read_text())
    assert previous['source_sha256'] == sources, 'Sources changed since the interrupted run'
    assert previous['cpu_max'] == [quota, period]
    for index, record in enumerate(previous['steps']):
        assert record['command'] == steps[index][1] and record['exit_code'] == 0
        assert hashlib.sha256((root / record['log']).read_bytes()).hexdigest() == record['log_sha256']
    manifest = previous
    steps = steps[len(previous['steps']):]
    if steps:
        partial_log = out / (steps[0][0] + '.log')
        if partial_log.exists():
            partial_log.rename(out / (steps[0][0] + '-interrupted.log'))
for name, command in steps:
    record = {'command': command, 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'log': str((out / (name + '.log')).relative_to(root))}
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    if command[:2] == ['graphify', 'update']:
        for path in deleted_graph_files:
            (root / path).unlink(missing_ok=True)
    record.update(exit_code=result.returncode, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), log_sha256=hashlib.sha256((out / (name + '.log')).read_bytes()).hexdigest())
    manifest['steps'].append(record)
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(record), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
