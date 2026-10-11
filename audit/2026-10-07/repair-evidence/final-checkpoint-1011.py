#!/usr/bin/env python3
"""Verify the current repair checkpoint without relaxing any original gate."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[3]
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def run(command):
    return subprocess.run(command, cwd=root, check=True)

proof_path = audit / 'repair-checkpoint-1011.json'
assert not proof_path.exists()
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root).strip()
originals = [audit / n for n in ['REPORT.md', 'findings.json', 'validation.json']]
original_hashes = {str(p.relative_to(root)): digest(p) for p in originals}
run(['git', 'diff', '--quiet', '--', *map(str, originals)])
deleted = set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
ledger_path = audit / 'remediation.json'
ledger = json.loads(ledger_path.read_text())
baseline = json.loads((audit / 'findings.json').read_text())['findings']
assert {f['id']: f['repair_gate'] for f in baseline} == {f['id']: f['original_repair_gate'] for f in ledger['findings']}
assert len(baseline) == 33
assert sum(f['status'] == 'software_verified' for f in ledger['findings']) == 32
assert [f['id'] for f in ledger['findings'] if f['status'] == 'open'] == ['S01']
references = {}
for name, expected in [
    ('repair-public-rf-verification-1010.json', '72k_accepted_frame_packet_image_comparisons_passed'),
    ('repair-audio-driver-verification-1010-attempt-2.json', 'real_shared_audio_driver_lifecycle_verified'),
    ('repair-public-rds-verification-1010.json', 'independent_mpx_metadata_and_partitions_verified')]:
    path = audit / name
    proof = json.loads(path.read_text())
    assert proof['verification_state'] == expected
    for source, sha in proof['source_sha256'].items():
        assert digest(root / source) == sha, source
    references[name] = digest(path)
source_hashes = {str(p.relative_to(root)): digest(p) for p in [root / 'ez-gui/src/audio_output.rs', root / 'ez-gui/src/radio_rds.rs', root / 'lrpt-decode/src/lib.rs', root / 'lrpt-decode/src/off_air_validation.rs', Path(__file__)]}
checks = []
for label, command in [
    ('build', ['cargo', 'build', '-p', 'ez-gui', '-j', '4']),
    ('format', ['cargo', 'fmt', '--all', '--', '--check']),
    ('graph', ['graphify', 'update', '.'])]:
    log_path = evidence / f'checkpoint-{label}-1011.log'
    assert not log_path.exists()
    start = datetime.datetime.now(datetime.timezone.utc).isoformat()
    with log_path.open('w') as log:
        result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    if label == 'graph':
        for name in deleted:
            p = root / name
            if name.startswith('graphify-out/') and p.is_file():
                p.unlink()
    assert result.returncode == 0, log_path
    checks.append(dict(command=command, started_utc=start, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), exit_code=0, log=str(log_path.relative_to(audit)), log_sha256=digest(log_path)))
assert deleted == set(subprocess.check_output(['git', 'ls-files', '-d', '-z'], cwd=root).decode().split('\0')) - {''}
for name, sha in {**original_hashes, **source_hashes}.items():
    assert digest(root / name) == sha, name
ledger['off_air_reference_captures'] = 'User has no current private recordings. Actual 72k RF slice and full usable public IQ prefix now pass independent packet/image comparisons: 6,899 recovered frames, 439 missing reference frames, 6,055,292 packet bytes and 25,025,280 pixels (maximum error one). The compressed source is truncated. Independent interleaved 80k capture validation remains pending.'
ledger_path.write_text(json.dumps(ledger, indent=2) + '\n')
proof = dict(verification_state='software_checkpoint_verified_external_gates_open', finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
    source_sha256=source_hashes, original_audit_sha256=original_hashes, reference_proof_sha256=references, checks=checks,
    software_verified_findings=32, open_findings=['S01'], preserved_user_deletions=len(deleted), full_cpu_authorized=True,
    receiver_inventory=subprocess.check_output(['lsusb'], text=True),
    verification_boundary='No independent interleaved 80k recording or physical SDR is available. Original off-air stereo/CTCSS, physical RF error/overload/calibration and full audio/disk release gates remain unverified. Public RDS metadata and silent real shared audio lifecycle prove only their stated scopes. Goal remains active.')
proof_path.write_text(json.dumps(proof, indent=2) + '\n')
with (audit / 'REMEDIATION.md').open('a') as doc:
    doc.write('\n## Verified repair checkpoint — 2026-10-11\n\nFull CPU access is now authorized. The audio startup timeout fix, saved actual 72k RF packet/image comparisons, independent RDS metadata fixture and silent native audio lifecycle checks are committed. Default GUI build, formatting and AST graph refresh pass at this checkpoint. All 33 original repair gates and user deletions are preserved. The ledger contains 32 software-verified findings and S01 open. Independent interleaved 80k data and physical receiver/field release evidence are unavailable; those requirements remain open. See repair-checkpoint-1011.json and its linked evidence for exact scope and retained commands.\n')
owned = {str(p.relative_to(root)) for p in [Path(__file__), proof_path, ledger_path, audit / 'REMEDIATION.md']}
owned.update(str(audit.relative_to(root) / Path(step['log'])) for step in checks)
run(['git', 'add', '-f', '--', *sorted(owned)])
staged = set(subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=root, text=True).splitlines())
assert staged <= owned and 'AGENTS.md' not in staged
assert not subprocess.check_output(['git', 'diff', '--cached', '--name-only', '--diff-filter=D'], cwd=root).strip()
run(['git', 'diff', '--cached', '--check', '--', *sorted(p for p in staged if not p.endswith('.log'))])
run(['git', 'commit', '-m', 'docs: record verified repairs and remaining external audit gates'])
