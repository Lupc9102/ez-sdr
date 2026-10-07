"""Reproduce the software counterexamples; successful execution is not a fix.

The four ignored acceptance tests assert correct behavior and are expected to
fail. Every other command must succeed. No SDR, sound device or network is used.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
EVIDENCE = OUT / "evidence"
EVIDENCE.mkdir(exist_ok=True)
commands = []
gates = []


def run(name, args, expected=0, extra_env=None):
    print(f"Running {name}: {' '.join(args)}", flush=True)
    started = time.monotonic()
    env = os.environ.copy()
    env.update(extra_env or {})
    path = EVIDENCE / f"{name}.log"
    with path.open("w") as output:
        result = subprocess.run(args, cwd=ROOT, env=env, stdout=output,
                                stderr=subprocess.STDOUT, timeout=900)
    text = path.read_text()
    commands.append(dict(name=name, args=args, environment=extra_env or {},
                         expected_exit=expected, actual_exit=result.returncode,
                         duration_seconds=round(time.monotonic()-started, 3),
                         log=str(path.relative_to(OUT))))
    check(f"{name}: expected exit {expected}", result.returncode == expected)
    return text


def check(name, condition):
    gates.append(dict(name=name, passed=bool(condition)))
    if not condition:
        raise AssertionError(name)


def number(text, pattern):
    match = re.search(pattern, text)
    if not match:
        raise AssertionError(f"Missing measurement: {pattern}")
    return float(match.group(1))


def verify():
    baseline = run("workspace-tests", ["cargo", "test", "--workspace", "--no-default-features"])
    check("normal tests have no failures", "test result: FAILED" not in baseline)
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", baseline)
    check("workspace test results present", len(results) >= 10)
    # Counts include separate binary/lib harnesses. They are deliberately not
    # presented as a count of unique behavioral requirements.
    commands[-1]["harness_results"] = [dict(passed=int(p), failed=int(f), ignored=int(i))
                                       for p, f, i in results]
    run("default-feature-check", ["cargo", "check", "--workspace", "--all-targets"])
    run("hardware-feature-check", ["cargo", "check", "-p", "ez-daemon", "--all-features", "--all-targets"])

    satellite = run("satellite", ["cargo", "run", "-p", "lrpt-decode", "--example", "audit_logic", "--release"])
    for rate in [640000, 2048000]:
        for phase in [0, 15, 30, 45, 60, 90, 180]:
            expected = 0 if phase in [45, 60, 90] else 12
            check(f"S02: rate {rate}, phase {phase}, RS codewords {expected}",
                  f"oqpsk rate={rate} phase={phase} rs_ok={expected}\n" in satellite)
    check("S04: lost later frames, clean six-frame control succeeds",
          "phase_slip_180 continuous_6frame_stream rs_ok=12 clean_control_rs_ok=24" in satellite)
    check("S06: partial header loses packet", "split_header bytes=3 packets=0" in satellite)
    for split in [6, 9]:
        check(f"S06: complete-header control split {split}", f"split_header bytes={split} packets=1" in satellite)
    check("S07: cross-VCID corrupted payload", 'cross_vcid_same_apid data=["BBAA"]' in satellite)
    check("S05: earlier admissible marker discarded", "extracted=1 expected=2" in satellite)
    check("S08: late-origin black prefix", "first_segment_sequence_16300 height=Some(3040)" in satellite)
    check("S09: exact DC dequantization error", "expected_uniform_pixel=160 actual_first_pixel=161" in satellite)
    check("S09: independent non-flat vector differs",
          number(satellite, r"independent_nonflat_jpeg pixels=896 max_error=(\d+)") >= 4)
    check("S10: zero-input false carrier lock", "zero_input_costas_locked=true" in satellite)

    aged = run("viterbi-noise-recovery", ["cargo", "run", "-p", "lrpt-decode", "--example", "audit_logic", "--release"],
               extra_env={"EZ_AUDIT_LONG_VITERBI": "1"})
    check("S03: identical clean input succeeds in fresh decoder", "fresh_control_errors=0 fresh_output_ones=851" in aged)
    check("S03: aged decoder remains dead", "clean_comparison_bits=1920 errors=823 output_ones=0" in aged)

    radio = run("radio", ["cargo", "run", "-p", "ez-gui", "--no-default-features", "--example", "audit_radio_logic", "--release"])
    wanted48 = number(radio, r"daemon_USB rate=48000 tone_hz=300 rms=([\d.]+)")
    wrong48 = number(radio, r"daemon_USB rate=48000 tone_hz=-300 rms=([\d.]+)")
    wanted200 = number(radio, r"daemon_USB rate=200000 tone_hz=300 rms=([\d.]+)")
    wrong200 = number(radio, r"daemon_USB rate=200000 tone_hz=-300 rms=([\d.]+)")
    check("R12: 48k control rejection exceeds 500:1", wanted48/wrong48 > 500)
    check("R12: production-rate rejection below 4:1", wanted200/wrong200 < 4)
    check("R04: 49k PCM aliases audibly", number(radio, r"1kHz_alias_amplitude=([\d.]+)") > .4)
    check("R04: actual WFM pipeline also aliases", number(radio, r"9kHz_alias_amplitude=([\d.]+)") > .004)
    check("R10: RAW timebase mismatch", "mono_samples=384000 declared_audio_rate=48000" in radio)
    check("R02: float replay loses weak signal/headroom", "replay_CU8=[128, 128, 255, 0]" in radio)
    before = number(radio, r"200Hz_amplitude_before_reset=([\d.]+)")
    after = number(radio, r"after_reset=([\d.]+)")
    check("R11: configured HPF lost on reset", after/before > 4)
    check("R09: 6k AM folds to 2k audio", number(radio, r"2kHz_alias=([\d.]+)") > .07)
    low = number(radio, r"NFM deviation_2500Hz rate=192000 tone_amplitude=([\d.]+)")
    high = number(radio, r"NFM deviation_2500Hz rate=2048000 tone_amplitude=([\d.]+)")
    check("R05: FM gain depends on RF clock", 10 < low/high < 11)

    channel = run("channelizer", ["cargo", "run", "-p", "ez-daemon", "--example", "audit_channelizer", "--release"])
    # Preserve all numerical output even if formatting later changes.
    check("R03: aliased carrier measured", "0.436185" in channel and "0.499629" in channel)
    control = run("ingest-control", ["cargo", "run", "-p", "ez-daemon", "--example", "audit_ingest_control"])
    check("R14: rejected tune leaks into applied metadata without epoch change",
          "rejected_tune_error=true reported_frequency=101000000 next_block_frequency=101000000 capture_epoch_changed=false next_block_discontinuous=false" in control)

    failed = run("native-acceptance-counterexamples",
                 ["cargo", "test", "-p", "ez-gui", "--no-default-features", "--lib", "audit_", "--", "--ignored", "--nocapture"], expected=101)
    for name in ["audit_signed_file_replay", "audit_ssb_power_meter_passband",
                 "audit_narrow_cw_power_meter", "audit_reject_old_capture_generation"]:
        check(f"acceptance assertion fails as documented: {name}",
              f"::{name} ... FAILED" in failed)
    check("exactly four documented acceptance failures", "0 passed; 4 failed; 0 ignored" in failed)


if __name__ == "__main__":
    error = None
    try:
        verify()
    except Exception as exc:
        error = f"{type(exc).__name__}: {exc}"
    inventory = json.loads((OUT / "findings.json").read_text())
    files = sorted({s["file"] for f in inventory["findings"] for s in f["sources"]})
    provenance = {file: hashlib.sha256((ROOT/file).read_bytes()).hexdigest() for file in files}
    record = dict(base_commit=inventory["base_commit"], production_fixes=False,
                  commands=commands, counterexample_gates=gates,
                  source_sha256=provenance, success=error is None, error=error,
                  meaning="Audit reproductions verified; four acceptance tests intentionally fail. Hardware/on-air performance unverified.")
    (OUT / "validation.json").write_text(json.dumps(record, indent=2) + "\n")
    if error:
        raise SystemExit(error)
    print(f"Verified {len(commands)} commands and {len(gates)} counterexample/build gates.")
