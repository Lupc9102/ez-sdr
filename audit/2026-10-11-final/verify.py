#!/usr/bin/env python3
"""Run final release checks and retain command, exit status, time and log hashes.

Run from the repository root. This does not open an SDR or publish a release.
Logs and generated previews are local audit evidence, not automated field proof.
"""
import argparse
import datetime
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
AUDIT = Path(__file__).resolve().parent
cpu_policy = json.loads((AUDIT / "cpu-policy.json").read_text())
os.sched_setaffinity(0, set(cpu_policy["shared_audit_cpu_set"]))
os.setpriority(os.PRIO_PROCESS, 0, max(os.getpriority(os.PRIO_PROCESS, 0), 10))
parser = argparse.ArgumentParser()
parser.add_argument("group", choices=["quality", "runtime", "reference", "performance", "security", "full", "headless"])
args = parser.parse_args()
environment = dict(os.environ, CARGO_BUILD_JOBS="4", RUST_TEST_THREADS="4")
cache = Path("/home/lupc/.cache/ez-sdr-audit")
groups = {
    "headless": [("headless-final", ["cargo", "test", "--release", "--workspace", "--locked", "--no-default-features"], {})],
    "full": [("workspace-release-default", ["cargo", "test", "--release", "--workspace", "--locked"], {})],
    "quality": [
        ("format", ["cargo", "fmt", "--all", "--", "--check"], {}),
        ("clippy", ["cargo", "clippy", "--workspace", "--locked", "--all-targets", "--", "-D", "warnings"], {}),
        ("docs", ["cargo", "doc", "--workspace", "--locked", "--no-deps"], {"RUSTDOCFLAGS": "-D warnings"}),
        ("optional-hardware", ["cargo", "check", "--workspace", "--locked", "--all-features", "--all-targets"], {}),
        ("release-build", ["cargo", "build", "--workspace", "--locked", "--release", "--features", "ez-daemon/rtlsdr,dump1090/rtlsdr"], {}),
    ],
    "runtime": [
        ("headless-tests", ["cargo", "test", "--release", "--workspace", "--locked", "--no-default-features"], {}),
        ("ui-previews", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "--lib", "app::app_ui_tests::render_workspaces", "--", "--exact", "--ignored", "--nocapture"], {"EZ_SDR_PREVIEW_DIR": str(AUDIT / "previews")}),
        ("tcp-iq", ["cargo", "test", "--release", "-p", "ez-daemon", "--locked", "hardware::tcp_iq::tests::", "--", "--ignored", "--nocapture"], {}),
        ("rigctl", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "rigctl::tests::", "--", "--ignored", "--nocapture"], {}),
        ("fractional-fft", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "spectrum::tests::complex_large_fft_preserves_weak_tone_and_fractional_sample_rate", "--", "--exact", "--nocapture"], {}),
        ("silent-audio-driver", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "audio_output::tests::silent_real_driver_callbacks_stop_and_restart_release_old_receiver", "--", "--exact", "--ignored", "--nocapture"], {"EZ_AUDIT_SILENT_AUDIO": "1"}),
    ],
    "reference": [
        ("rds-public-reference", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "radio_rds::tests::public_reference_multiplex_matches_independent_redsea_metadata", "--", "--exact", "--ignored", "--nocapture"], {"EZ_RDS_REFERENCE_WAV": str(cache / "radio-public-1010/mpx-testfile-yksi-f32.wav")}),
        ("meteor-public-72k", ["cargo", "test", "--release", "-p", "lrpt-decode", "--locked", "--lib", "tests::public_off_air_carrier_recovers_reference_payload_bytes", "--", "--exact", "--ignored", "--nocapture"], {
            "EZ_METEOR_CS8": str(cache / "meteor-public-1010/meteor-72k.cs8"),
            "EZ_METEOR_CADU": str(cache / "meteor-public-1010/meteor_m2-x_lrpt.cadu"),
            "EZ_METEOR_START_SECONDS": "180", "EZ_METEOR_SECONDS": "20",
            "EZ_METEOR_VALIDATION_OUTPUT": str(AUDIT / "meteor-public-72k")}),
        ("meteor-image-oracle", ["python3", "audit/2026-10-07/verify_public_image.py", str(AUDIT / "meteor-public-72k/reference.cadu"), str(AUDIT / "meteor-public-72k"), str(AUDIT / "meteor-public-72k/oracle")], {}),
    ],
    "performance": [
        ("radio-throughput", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "--test", "radio_performance", "--", "--ignored", "--nocapture", "--test-threads=1"], {}),
        ("frame-profile", ["cargo", "test", "--release", "-p", "ez-gui", "--locked", "--lib", "app::app_performance_tests::profile_frames_with_continuous_native_recording", "--", "--exact", "--ignored", "--nocapture", "--test-threads=1"], {"EZ_SDR_PERFORMANCE_DIR": str(AUDIT / "performance")}),
    ],
    "security": [
        ("cargo-audit", ["/tmp/ez-sdr-audit-tools/bin/cargo-audit", "audit", "--json"], {}),
        ("cargo-deny", ["/tmp/ez-sdr-audit-tools/bin/cargo-deny", "deny", "check"], {}),
        ("cargo-audit-unfiltered", ["/tmp/ez-sdr-audit-tools/bin/cargo-audit", "audit", "--file", str(ROOT / "Cargo.lock"), "--json", "--no-fetch"], {}),
    ],
}
manifest = {"group": args.group, "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "cpu_policy": cpu_policy["method"], "checks": []}
for name, command, extra in groups[args.group]:
    path = AUDIT / "evidence" / (name + ".log")
    path.parent.mkdir(parents=True, exist_ok=True)
    begin = time.monotonic()
    record = {"name": name, "command": command, "environment": extra}
    with path.open("w") as log:
        try:
            cwd = Path("/tmp") if name == "cargo-audit-unfiltered" else ROOT
            record["cwd"] = str(cwd)
            result = subprocess.run(command, cwd=cwd, env=dict(environment, **extra), stdout=log, stderr=subprocess.STDOUT, timeout=1800)
            record["exit_code"] = result.returncode
        except (subprocess.TimeoutExpired, OSError) as error:
            log.write(str(error) + "\n")
            record["exit_code"] = None
            record["error"] = str(error)
    if command[:2] == ["cargo", "test"]:
        summaries = re.findall(r"test result: .*? (\d+) passed; (\d+) failed; (\d+) ignored", path.read_text(errors="replace"))
        record["tests_passed"] = sum(int(row[0]) for row in summaries)
        record["tests_failed"] = sum(int(row[1]) for row in summaries)
        record["tests_ignored"] = sum(int(row[2]) for row in summaries)
        if record["exit_code"] == 0 and not record["tests_passed"]:
            record["exit_code"] = 1
            record["error"] = "Command selected zero passing tests; this is not verification."
    record.update(seconds=round(time.monotonic() - begin, 3), log=str(path.relative_to(ROOT)), log_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
    manifest["checks"].append(record)
    manifest["updated_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (AUDIT / (args.group + ".json")).write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(record), flush=True)
print("Passed", sum(c["exit_code"] == 0 for c in manifest["checks"]), "of", len(manifest["checks"]), flush=True)
