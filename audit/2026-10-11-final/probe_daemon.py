#!/usr/bin/env python3
"""Exercise an isolated synthetic daemon's real process, REST and WS controls.

Uses ephemeral loopback ports and temporary recordings. The foreign-origin
probe is confined to this daemon process and changes only a synthetic tuning.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
AUDIT_SOURCE = Path(__file__).resolve().parent
AUDIT = Path(os.environ.get("EZ_AUDIT_OUTPUT_DIR", AUDIT_SOURCE)).resolve()
(AUDIT / "evidence").mkdir(parents=True, exist_ok=True)
policy = json.loads((AUDIT_SOURCE / "cpu-policy.json").read_text())
os.sched_setaffinity(0, set(policy["shared_audit_cpu_set"]))
os.nice(10)

def port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]

tcp_port, web_port = port(), port()
base = f"http://127.0.0.1:{web_port}"
result = {"source": "isolated synthetic daemon; no SDR", "checks": []}
with tempfile.TemporaryDirectory(prefix="ez-final-daemon-") as directory:
    binary = Path(os.environ.get("EZ_AUDIT_DAEMON_BINARY", ROOT / "target/release/ez-daemon"))
    result["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    command = [str(binary), "--source", "synthetic", "--sample-rate", "256000", "--freq", "100000000", "--listen-port", str(tcp_port), "--web-listen-port", str(web_port), "--recording-dir", directory, "--web-static-dir", directory + "/missing"]
    log = (AUDIT / "evidence/daemon-process.log").open("w")
    process = subprocess.Popen(command, cwd=directory, stdout=log, stderr=subprocess.STDOUT)

    def request(path, data=None, method=None, headers=None):
        body = None if data is None else json.dumps(data).encode()
        req = urllib.request.Request(base + path, data=body, method=method, headers={"Content-Type": "application/json", **(headers or {})})
        try:
            with urllib.request.urlopen(req, timeout=5) as response:
                content = response.read()
                return response.status, (json.loads(content) if response.headers.get_content_type() == "application/json" else content.decode()) if content else None
        except urllib.error.HTTPError as error:
            return error.code, error.read().decode()

    try:
        deadline = time.monotonic() + 10
        while True:
            try:
                status, initial = request("/api/status")
                if status == 200:
                    break
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.05)
        assert initial["hardware"]["sample_rate_hz"] == 256000
        spec = {"id": 77, "center_offset_hz": 0, "channel_sample_rate_hz": 64000, "kind": "Spectrum", "demod_mode": None}
        status, created = request("/api/channels", spec)
        assert status in [200, 201], (status, created)
        status, channels = request("/api/channels")
        assert status == 200 and any(c["spec"]["id"] == 77 for c in channels)
        status, invalid = request("/api/channels/77/retune", {"center_offset_hz": 1000000, "channel_sample_rate_hz": 64000})
        assert status == 400, (status, invalid)
        status, started = request("/api/channels/77/recording/start", {"format": "Cf32"})
        assert status in [200, 201], (status, started)
        time.sleep(0.3)
        status, blocked = request("/api/channels/77", method="DELETE")
        assert status == 400, (status, blocked)
        status, stopped = request("/api/channels/77/recording/stop", {})
        assert status in [200, 204], (status, stopped)
        files = [{"name": p.name, "bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in Path(directory).glob("*.cf32")]
        assert files and all(p["bytes"] > 0 and p["bytes"] % 8 == 0 for p in files)
        result["checks"].append({"name": "REST channel create, invalid retune, recording, deletion guard", "passed": True, "recordings": files})

        # A browser WS request includes Origin; CORS does not protect WS upgrades.
        sock = socket.create_connection(("127.0.0.1", web_port), timeout=5)
        key = base64.b64encode(os.urandom(16)).decode()
        handshake = f"GET /ws/control HTTP/1.1\r\nHost: 127.0.0.1:{web_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nOrigin: https://foreign-audit.example\r\n\r\n"
        sock.sendall(handshake.encode())
        response = sock.recv(8192).split(b"\r\n", 1)[0].decode()
        if " 101 " in response:
            payload = json.dumps({"SetFrequency": {"hz": 100100000}}, separators=(",", ":")).encode()
            mask = os.urandom(4)
            sock.sendall(bytes([0x81, 0x80 | len(payload)]) + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(payload)))
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                _, hardware = request("/api/hardware")
                if hardware["frequency_hz"] == 100100000:
                    break
                time.sleep(0.05)
            changed = hardware["frequency_hz"] == 100100000
        else:
            changed = False
        sock.close()
        result["checks"].append({"name": "foreign Origin WebSocket control", "response": response, "synthetic_frequency_changed": changed, "origin_rejected": " 403 " in response})
        assert " 403 " in response and not changed, (response, changed)
        frontend_status = request("/")[0]
        assert frontend_status == 200, frontend_status
        result["checks"].append({"name": "bundled web frontend", "http_status": frontend_status})
        status, removed = request("/api/channels/77", method="DELETE")
        assert status == 204, (status, removed)
        # Server-side rebinding-shaped request. This does not simulate browser
        # DNS or private-network permission behavior and is not proof of a
        # working remote-page exploit. It exposes the HTTP trust boundary.
        origin = f"http://foreign-audit.example:{web_port}"
        status, body = request("/api/hardware/frequency", {"hz": 100200000},
            headers={"Host": f"foreign-audit.example:{web_port}", "Origin": origin})
        _, hardware = request("/api/hardware")
        assert status == 403 and hardware["frequency_hz"] == 100000000, (status, body, hardware)
        result["checks"].append({"name": "REST Host/Origin boundary", "http_status": status,
            "synthetic_frequency_changed": hardware["frequency_hz"] == 100200000,
            "scope": "server rejects rebinding-shaped HTTP control; browser DNS/private-network behavior not tested"})
        for headers in [
            {"Origin": "https://foreign-audit.example"},
            {"Host": f"foreign-audit.example:{web_port}"},
        ]:
            status, body = request("/api/hardware/frequency", {"hz": 100200000}, headers=headers)
            _, hardware = request("/api/hardware")
            assert status == 403 and hardware["frequency_hz"] == 100000000, (headers, status, body, hardware)
        status, body = request("/api/hardware/frequency", {"hz": 100100000}, headers={"Origin": base})
        _, hardware = request("/api/hardware")
        assert status == 204 and hardware["frequency_hz"] == 100100000, (status, body, hardware)
        result["checks"].append({"name": "foreign Origin and Host rejected; same-origin REST control works", "passed": True})
    finally:
        began = time.monotonic()
        process.send_signal(signal.SIGINT)
        try:
            result["shutdown_exit_code"] = process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            result["shutdown_exit_code"] = process.wait()
            result["shutdown_forced"] = True
        result["shutdown_seconds"] = round(time.monotonic() - began, 3)
        result["command"] = command
        log.close()
        (AUDIT / "daemon-process.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result, indent=2))
        assert result["shutdown_exit_code"] == 0 and not result.get("shutdown_forced"), result
