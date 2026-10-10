#!/usr/bin/env python3
"""Close only R19 after every required software check has actually passed."""
import datetime
import hashlib
import json
from pathlib import Path
import re

root = Path('/home/lupc/Documents/ez-sdr')
audit = root / 'audit/2026-10-07'
evidence = audit / 'repair-evidence'

def read_json(path):
    return json.loads(path.read_text())

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

old_gui = read_json(evidence / 'r19-gui-verification-results-1010.json')
new_gui = read_json(evidence / 'r19-gui-verification-results-1010-retry.json')
clock_run = read_json(evidence / 'r19-controls-clock-results-1010.json')
control_retry = read_json(evidence / 'r19-controls-clock-results-1010-retry.json')
assert len(new_gui['steps']) == 7 and all(s['exit_code'] == 0 for s in new_gui['steps'])
assert len(control_retry['steps']) == 2 and all(s['exit_code'] == 0 for s in control_retry['steps'])
assert clock_run['steps'][0]['exit_code'] == 0
assert old_gui['steps'][2]['exit_code'] == 0
assert new_gui['cpu_max'] == ['10000', '100000']
for path, expected in new_gui['source_sha256'].items():
    assert digest(root / path) == expected, f'Unverified source changed: {path}'
# Only the control branch changed after the successful playback run.
for path in [
    'ez-gui/src/audio_resampler.rs', 'ez-daemon/src/pipelines/audio.rs',
    'ez-daemon/src/audio_rate.rs', 'ez-proto/src/messages.rs',
]:
    assert old_gui['source_sha256'][path] == new_gui['source_sha256'][path], path

pipeline_log = evidence / 'r19-pipeline-tests-1010.log'
pipeline_command = ['cargo', 'test', '-p', 'ez-daemon', '--lib', 'pipelines::audio::', '-j', '1', '--', '--test-threads=1', '--nocapture']
pipeline_record = {'command': pipeline_command, 'exit_code': 0, 'log': str(pipeline_log.relative_to(root)), 'log_sha256': digest(pipeline_log)}
commands = [pipeline_record, clock_run['steps'][0], *control_retry['steps'], *new_gui['steps'], old_gui['steps'][2]]
passed = 0
for command in commands:
    log = root / command['log']
    assert command['exit_code'] == 0 and digest(log) == command['log_sha256']
    results = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', log.read_text())
    if command['command'][1] == 'test':
        assert len(results) == 1 and int(results[0][0]) > 0 and results[0][1:] == ('0', '0'), command
        command['test_result'] = {'passed': int(results[0][0]), 'failed': 0, 'ignored': 0}
        passed += int(results[0][0])
assert passed == 47
pipeline_text = pipeline_log.read_text()
assert 'selected_rf_mixtures=12' in pipeline_text and 'voice_checks=336' in pipeline_text
assert 'max_relative_pcm_error=0.000003865' in pipeline_text
playback_text = (root / old_gui['steps'][2]['log']).read_text()
assert 'minimum_rejection_db=97.120270' in playback_text
assert 'minimum_rejection_db=112.350624' in playback_text

ledger = read_json(audit / 'remediation.json')
baseline = read_json(audit / 'findings.json')
gates = {f['id']: f['repair_gate'] for f in baseline['findings']}
assert len(gates) == len(ledger['findings']) == 33
assert {f['id']: f['original_repair_gate'] for f in ledger['findings']} == gates
r19 = next(f for f in ledger['findings'] if f['id'] == 'R19')
assert 'pub const PROTOCOL_VERSION: u32 = 12;' in (root / 'ez-proto/src/messages.rs').read_text()

artifact_paths = {root / c['log'] for c in commands}
for record in [*old_gui['steps'], *clock_run['steps']]:
    artifact_paths.add(root / record['log'])
for name in [
    'r19-controls-clock-runner-1010.py', 'r19-gui-verification-runner-1010.py',
    'r19-finalize-evidence-1010.py', 'r19-controls-clock-results-1010.json',
    'r19-controls-clock-results-1010-retry.json', 'r19-controls-clock-results-1010-initial-failure.json',
    'r19-gui-verification-results-1010.json', 'r19-gui-verification-results-1010-retry.json',
    'r19-state-configuration-tests-1010-initial-failure.log', 'r19-integration-check-1010.log',
    'r19-integration-format-1010.log', 'r19-gui-format-1010.log', 'r19-controls-format-1010.log',
    'r19-default-build-1010-retry-interrupted.log',
    'r19-cpu-sharing-1010.json',
    'r19-finish-and-s01-1010.py',
]:
    artifact_paths.add(evidence / name)
artifacts = [{'path': str(p.relative_to(audit)), 'sha256': digest(p)} for p in sorted(artifact_paths)]
source_hashes = dict(new_gui['source_sha256'])
for path in ['ez-daemon/src/audio_rf.rs', 'ez-daemon/src/pipelines/audio_filter.rs', 'README.md']:
    source_hashes[path] = digest(root / path)
proof = {
    'date': datetime.datetime.now(datetime.timezone.utc).date().isoformat(),
    'scope': ['R19'], 'verification_state': 'software_verified',
    'original_repair_gate': gates['R19'], 'protocol_version': 12,
    'resource_policy': {'cpu_quota': '10% of one CPU core combined', 'nice': 19, 'cargo_jobs': 1, 'test_threads': 1, 'during_public_download': 'Build capped at 9%, independent download/hash job at 1%; see r19-cpu-sharing-1010.json.'},
    'unique_affected_tests_passed': passed, 'commands': commands, 'source_sha256': source_hashes,
    'metrics': {
        'am_fm_wfm_strong_neighbor_mixtures': 12, 'neighbor_amplitude_ratio': 25,
        'maximum_relative_pcm_error': 0.000003865,
        'preserved_ssb_voice_checks': 336, 'minimum_ssb_opposite_rejection_db': 92.681208,
        'playback_frequency_cases': 122, 'minimum_playback_rejection_db': 112.350624,
        'wfm_mpx_stop_cases': 32, 'minimum_wfm_mpx_rejection_db': 97.120270,
        'maximum_relative_wfm_mixture_error': 0.000025807,
    },
    'coverage': [
        'RF selection precedes AM/FM/WFM nonlinear detection and selected-RF squelch.',
        'Independent RF width, requested channel rate and PCM output rate in protocol, state and desktop.',
        'Exact rational PCM counts at coherent boundaries; whole/split samples equal.',
        'Physical RF clock retained separately from generated PCM clock, including unknown UTC and capture-gap resets.',
        'Validated atomic FM/WFM/AM mode, RF-width, channel-rate and PCM-rate changes; invalid requests preserve applied state.',
        'Version-12 binary configuration/frame round trips and legacy JSON channel-rate alias.',
        'Desktop atomic configuration, center retune, generated-clock playback and existing audio alias gates.',
    ],
    'failed_development_runs': [
        'The first state fixture mixed a 2.0 MS/s constructor with a 2.048 MS/s source; the fixture now initializes geometry from the applied source clock.',
        'The first Clippy run rejected an unnecessary Option unwrap in the GUI retune branch; a direct match and repeated control tests resolve it.',
    ],
    'artifacts': artifacts,
    'verification_boundary': 'Software evidence only. All original off-air, receiver, audio-device and disk release gates remain unchanged and unverified where previously outstanding.',
}
(audit / 'repair-daemon-selection-verification.json').write_text(json.dumps(proof, indent=2) + '\n')
r19.update(
    status='software_verified',
    change='Protocol v12 separates RF width, channel sample rate and PCM output rate. Select complex RF before AM/FM/WFM detection and measure squelch on that selected signal. Share exact-rational audio conversion, retain separate RF/PCM clocks, validate atomic audio/channel changes and send them from the desktop.',
    verification='repair-daemon-selection-verification.json',
)
r19['regressions'] += [
    'pipelines::audio::selection_tests::selected_am_fm_wfm_reject_strong_neighbors_before_detection_and_squelch (12 mixtures; neighbor 25x stronger)',
    'pipelines::audio::selection_tests::independent_pcm_clock_preserves_rational_rf_provenance_and_partition_history',
    'pipelines::audio::selection_tests::audio_parameter_geometry_is_validated_even_when_raw_selection_is_bypassed',
    'audio_rate::clock_tests (two exact clock/count/reset tests)',
    'state::tests::audio_configuration_validates_all_three_rates_before_allocation',
    'state::tests::audio_mode_rf_width_and_pcm_rate_retune_atomically_in_both_directions',
    'messages::audio_configuration_tests (three protocol/alias/clock round trips)',
    'source_manager::tests::daemon_audio_mode_changes_send_one_atomic_three_rate_configuration',
    'audio_resampler::tests (13 current producer/playback regressions; unchanged alias gates)',
]
assert {f['id']: f['original_repair_gate'] for f in ledger['findings']} == gates
assert sum(f['status'] == 'software_verified' for f in ledger['findings']) == 32
(audit / 'remediation.json').write_text(json.dumps(ledger, indent=2) + '\n')
doc = (audit / 'REMEDIATION.md').read_text()
doc = doc.replace('31 findings have software repairs with passing regressions; 2 remain open.', '32 findings have software repairs with passing regressions; 1 remains open.', 1)
doc = doc.replace('- [ ] **R19 · P2** — Daemon narrowband listening uses a wide fixed channel without mode-specific RF selection\n', '', 1)
entry = '- **R19 — Daemon narrowband listening uses a wide fixed channel without mode-specific RF selection**: Separate RF width, channel rate and PCM rate in protocol v12; filter complex RF before detection and selected-RF squelch. Preserve independent RF/PCM clocks and validate atomic desktop retunes.\n  Evidence: 47 focused tests, all-target Clippy, default daemon/GUI builds, formatting and AST graph refresh pass. See repair-daemon-selection-verification.json.\n\n'
doc = doc.replace('## Implemented and software verified\n\n', '## Implemented and software verified\n\n' + entry, 1)
doc += '\n## R19 integration verified — 2026-10-10\n\nAll 47 unique affected daemon/protocol/GUI tests pass, including the 12 strong-neighbor mixtures, exact coherent PCM counts, atomic widening/narrowing and the unchanged 336 SSB voice checks. The existing 122-case playback sweep and 32 WFM MPX stop cases pass; WFM mixture error is 0.000025807 relative to wanted audio. The AM/NFM audio-alias fixture explicitly selects an RF passband containing its 49 kHz modulation, so RF rejection cannot conceal failure of the separate audio-rate stage.\n\nCurrent daemon/GUI/protocol all-target Clippy with warnings denied, default daemon and GUI builds, formatting and AST graph refresh pass. Failed fixture/Clippy runs, source hashes, commands and guarded runners are retained in repair-daemon-selection-verification.json. Builds and tests remain serial within a combined 10% limit of one CPU core, nice 19, one Cargo job and one test thread. Independent reference downloads and prefix restoration share that limit at 1% while the build receives 9%. User deletions and the original 33 repair gates are preserved. The ledger now has 32 software-verified findings and S01 open; physical/off-air/audio/disk release requirements remain unverified. Public recording availability checks are recorded separately in public-meteor-candidates-1010.json. The goal remains active.\n'
(audit / 'REMEDIATION.md').write_text(doc)
print(json.dumps({'software_verified': 32, 'remaining_findings': ['S01'], 'affected_tests_passed': passed, 'artifacts': len(artifacts)}))
