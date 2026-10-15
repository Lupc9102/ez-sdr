#!/usr/bin/env python3
"""Check the real shared audio driver with silence after prior jobs terminate."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'
unit = 'ez-sdr-audit-audio-driver-1010.service'
parser = argparse.ArgumentParser()
parser.add_argument('--attempt', type=int, default=1)
args = parser.parse_args()
assert args.attempt > 0
group = Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
cpu_path = Path('/sys/fs/cgroup') / group.lstrip('/') / 'cpu.max'
quota, period = cpu_path.read_text().split() if cpu_path.exists() else ('unavailable', 'unavailable')
# Full CPU use explicitly authorized on 2026-10-11.


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command):
    return subprocess.run(command, cwd=root, check=True)


run(['rustfmt', '--edition', '2021', 'ez-gui/src/audio_output.rs'])
workers = ['ez-sdr-audit-finish-and-reference-1010.service',
           'ez-sdr-audit-s01-finalize-1010.service',
           'ez-sdr-audit-rds-reference-1010.service']
while True:
    terminal = []
    for worker in workers:
        result = subprocess.run(['systemctl', '--user', 'show', worker, '-p', 'LoadState', '-p', 'ActiveState'],
                                capture_output=True, text=True)
        values = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
        terminal.append(result.returncode == 0 and (
            values.get('LoadState') == 'not-found' or values.get('ActiveState') in ['inactive', 'failed']))
    if all(terminal):
        break
    time.sleep(30)
manifest_path = audit / f'repair-audio-driver-verification-1010-attempt-{args.attempt}.json'
assert not manifest_path.exists(), 'Preserve previous results; select a new attempt'
default_sink = subprocess.check_output(['pactl', 'get-default-sink'], text=True).strip()
sinks = subprocess.check_output(['pactl', 'list', 'short', 'sinks'], text=True)
assert any(default_sink in line and 'PipeWire' in line for line in sinks.splitlines())
source_paths = ['ez-gui/src/audio_output.rs', 'ez-gui/Cargo.toml', 'Cargo.lock',
                'audit/2026-10-07/repair-evidence/audio-driver-runner-1010.py']
source_hashes = {path: digest(root / path) for path in source_paths}
environment = dict(os.environ, EZ_AUDIT_SILENT_AUDIO='1')
manifest = {
    'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'verification_state': 'pending', 'source_sha256': source_hashes,
    'default_sink_before': default_sink, 'sink_listing_before': sinks,
    'cpu_max': [quota, period], 'cargo_jobs': 1, 'test_threads': 1,
    'restrict_realtime': True, 'full_cpu_authorized': True, 'silence_only': True, 'steps': [],
    'verification_boundary': 'Real shared PipeWire output: callback queue consumption, native worker stop/receiver teardown and restart for mono/stereo zero samples. Does not certify acoustics, RF hardware, complete audio timing/calibration, off-air recordings or disk overload.',
}
steps = [
    ('test', ['cargo', 'test', '-p', 'ez-gui', '--lib',
              'audio_output::tests::silent_real_driver_callbacks_stop_and_restart_release_old_receiver',
              '-j', '1', '--', '--exact', '--ignored', '--test-threads=1', '--nocapture']),
    ('clippy', ['cargo', 'clippy', '-p', 'ez-gui', '--lib', '--tests', '-j', '1', '--', '-D', 'warnings']),
    ('format', ['cargo', 'fmt', '--all', '--', '--check']),
]
for name, command in steps:
    log_path = evidence / f'audio-driver-{name}-1010-attempt-{args.attempt}.log'
    assert not log_path.exists()
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    with log_path.open('w') as log:
        result = subprocess.run(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
    step = {'command': command, 'started_utc': started, 'exit_code': result.returncode,
            'log': str(log_path.relative_to(audit)), 'log_sha256': digest(log_path),
            'finished_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    manifest['steps'].append(step)
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(step), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
test_log = (evidence / f'audio-driver-test-1010-attempt-{args.attempt}.log').read_text()
assert 'test result: ok. 1 passed; 0 failed; 0 ignored;' in test_log
metric_line = next(line.split('AUDIO_DRIVER_RESULT ', 1)[1] for line in test_log.splitlines()
                   if 'AUDIO_DRIVER_RESULT ' in line)
metrics = json.loads(metric_line)
assert [case['input_channels'] for case in metrics['cases']] == [1, 2]
for case in metrics['cases']:
    assert case['sample_rate_hz'] == 48000 and case['queued_sample_frames'] == 2880
    assert case['silence_only'] and case['old_receiver_disconnected']
    assert case['stop_call_us'] < 100000 and case['receiver_retirement_us'] < 3000000
default_after = subprocess.check_output(['pactl', 'get-default-sink'], text=True).strip()
assert default_after == default_sink, 'Default output changed during verification; do not relabel the selected device'
satellite_proof = audit / 'repair-public-rf-verification-1010.json'
graph = root / 'graphify-out/graph.json'
graph_current = satellite_proof.exists() and graph.exists() and graph.stat().st_mtime >= (root / source_paths[0]).stat().st_mtime
if not graph_current:
    deleted = set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
    log_path = evidence / f'audio-driver-graph-1010-attempt-{args.attempt}.log'
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
        'graph_mtime': graph.stat().st_mtime, 'audio_source_mtime': (root / source_paths[0]).stat().st_mtime,
    }
for path, expected in source_hashes.items():
    assert digest(root / path) == expected
ledger = json.loads((audit / 'remediation.json').read_text())
baseline = json.loads((audit / 'findings.json').read_text())['findings']
assert {item['id']: item['repair_gate'] for item in baseline} == {
    item['id']: item['original_repair_gate'] for item in ledger['findings']}
assert next(item for item in ledger['findings'] if item['id'] == 'S01')['status'] == 'open'
manifest.update(verification_state='real_shared_audio_driver_lifecycle_verified', metrics=metrics,
                default_sink_after=default_after, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
with (audit / 'REMEDIATION.md').open('a') as doc:
    doc.write('\n## Silent real audio-driver lifecycle — 2026-10-10\n\n')
    doc.write(f"The native playback worker starts and consumes zero-sample queues on the actual shared PipeWire output ({default_sink}), stops, releases its old callback receiver and restarts with a fresh queue. Mono and stereo input cases run at 48 kHz. Each submits 2,880 zero-sample frames with no active queue overflow or driver error. Queue consumption proves callback execution; it is not a measurement of exact DAC output frames. Both stop calls return within 100 ms and callback receiver teardown completes within three seconds. See {manifest_path.name} for measured timings, source hashes, commands and driver identity.\n\n")
    doc.write('The former 10 ms CPAL initialization timeout was incorrectly documented as a latency hint. Playback now allows a bounded five-second server/device startup on its background worker; UI stop remains non-blocking. The explicit native-device test, GUI library/test Clippy and formatting pass; the graph refresh covers the new test. Full CPU use was explicitly authorized on 2026-10-11; checks used one Cargo job and one test thread with realtime priority disabled. This closes only the measured shared-driver lifecycle observation. Acoustics/calibration, physical RF hardware, off-air stereo/CTCSS/RDS and interleaved 80k comparisons, and disk-overload requirements remain unverified. Original finding statuses and repair gates remain unchanged.\n')
originals = [str(audit / name) for name in ['REPORT.md', 'findings.json', 'validation.json']]
run(['git', 'diff', '--quiet', '--', *originals])
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root).strip()
owned = {'ez-gui/src/audio_output.rs', str(Path(__file__).resolve().relative_to(root)),
         str(manifest_path.relative_to(root)), 'audit/2026-10-07/REMEDIATION.md',
         'audit/2026-10-07/repair-audio-driver-verification-1010-attempt-1.json',
         'audit/2026-10-07/repair-evidence/audio-driver-test-1010-attempt-1.log',
         'audit/2026-10-07/repair-evidence/audio-startup-regression-1011.log'}
owned.update(str(audit / step['log']) for step in manifest['steps'])
owned = {str(Path(path).relative_to(root)) if Path(path).is_absolute() else path for path in owned}
run(['git', 'add', '-f', '--', *sorted(owned)])
staged = set(subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True).splitlines())
assert staged <= owned and 'AGENTS.md' not in staged
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only', '--diff-filter=D'], cwd=root).strip()
run(['git', 'diff', '--cached', '--check', '--', *sorted(path for path in staged if not path.endswith('.log'))])
run(['git', 'commit', '-m', 'fix: allow audio backend initialization and verify real driver lifecycle'])
print('Measured real shared audio-driver lifecycle evidence committed; other field gates remain open.', flush=True)
