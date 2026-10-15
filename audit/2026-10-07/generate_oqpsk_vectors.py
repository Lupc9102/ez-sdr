"""Independent carrier acceptance: libfec RS + analytical transmit RRC pulses.

No application encoder, pulse/filter code or IQ generator is imported. Requires
NumPy and a shared libfec containing init_rs_char/encode_rs_char. The reference
build used quiet/libfec at 9750ca0a6d0a786b506e44692776b541f90daa91:
  cc -O2 -shared -fPIC -o /tmp/ez-sdr-libfec.so init_rs_char.c encode_rs_char.c

Usage: python generate_oqpsk_vectors.py LIBFEC.so OUTPUT_DIR [--sampling]
These are carrier/CCSDS primitives, not a marker-interleaved 80k mission profile.
"""

import ctypes
import hashlib
import json
from pathlib import Path
import sys

import numpy as np


def reference_frames(library, instrument_segment=None):
    fec = ctypes.CDLL(str(library))
    fec.init_rs_char.argtypes = [ctypes.c_int] * 6
    fec.init_rs_char.restype = ctypes.c_void_p
    fec.encode_rs_char.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte),
                                 ctypes.POINTER(ctypes.c_ubyte)]
    codec = fec.init_rs_char(8, 0x187, 112, 11, 32, 0)
    assert codec
    rng = np.random.default_rng(8172026)
    pn, state = [], 255
    for _ in range(255):
        byte = 0
        for _ in range(8):
            byte = (byte << 1) | (state & 1)
            feedback = (state ^ (state >> 3) ^ (state >> 5) ^ (state >> 7)) & 1
            state = (state >> 1) | (feedback << 7)
        pn.append(byte)
    assert bytes(pn[:8]) == bytes.fromhex("ff480ec09a0d70bc")
    frames, payloads = [], []
    for counter in range(100, 103):
        payload = bytearray(rng.integers(0, 256, 892, dtype=np.uint8).tobytes())
        payload[:10] = bytes([0x40, 5]) + counter.to_bytes(3, "big") + bytes([0, 0, 0, 7, 255])
        if instrument_segment is not None:
            # One independent image segment per 43-packet instrument cycle.
            sequence = (counter - 100) * 43
            packet = bytes.fromhex("0040") + (0xc000 | sequence).to_bytes(2, "big") + (len(instrument_segment) - 1).to_bytes(2, "big") + instrument_segment
            idle_length = 892 - 10 - len(packet) - 6
            assert idle_length > 0
            idle = bytes.fromhex("07ffc000") + (idle_length - 1).to_bytes(2, "big") + bytes(idle_length)
            payload = bytearray(bytes([0x40, 5]) + counter.to_bytes(3, "big") + bytes(5) + packet + idle)
            assert len(payload) == 892
        payloads.append(bytes(payload))
        transport = np.zeros((255, 4), dtype=np.uint8)
        for channel in range(4):
            data = (ctypes.c_ubyte * 223)(*payload[channel::4])
            parity = (ctypes.c_ubyte * 32)()
            fec.encode_rs_char(codec, data, parity)
            transport[:, channel] = list(data) + list(parity)
        whitened = bytes(value ^ pn[i % 255] for i, value in enumerate(transport.ravel()))
        frames.append(bytes.fromhex("1acffc1d") + whitened)
    information = np.concatenate([rng.integers(0, 2, 5000, dtype=np.uint8),
                                 np.unpackbits(np.frombuffer(b"".join(frames), dtype=np.uint8)),
                                 rng.integers(0, 2, 512, dtype=np.uint8)])
    # NRZ-M levels followed by the public rate-1/2 K=7 trellis convention.
    levels = np.bitwise_xor.accumulate(information)
    coded, state = [], 0
    for bit in levels:
        register = (int(bit) << 6) | state
        coded.append([((register & polynomial).bit_count() & 1) ^ 1
                      for polynomial in (0o171, 0o133)])
        state = register >> 1
    return 1.0 - 2.0 * np.asarray(coded), b"".join(payloads)


def rrc(time):
    beta = 0.5
    with np.errstate(divide="ignore", invalid="ignore"):
        value = (np.sin(np.pi * time * (1 - beta)) + 4 * beta * time
                 * np.cos(np.pi * time * (1 + beta))) / (np.pi * time * (1 - (4 * beta * time) ** 2))
    value = np.where(np.abs(time) < 1e-9, 1 - beta + 4 * beta / np.pi, value)
    limit = beta / np.sqrt(2) * ((1 + 2 / np.pi) * np.sin(np.pi / (4 * beta))
                               + (1 - 2 / np.pi) * np.cos(np.pi / (4 * beta)))
    return np.where(np.abs(np.abs(4 * beta * time) - 1) < 1e-9, limit, value)


def branch(symbols, time):
    center = np.floor(time).astype(np.int64)
    output = np.zeros(time.shape)
    for delta in range(-6, 7):
        index = center + delta
        valid = (index >= 0) & (index < len(symbols))
        output[valid] += symbols[index[valid]] * rrc(time[valid] - index[valid])
    return output


def main():
    library, output = map(Path, sys.argv[1:3])
    output.mkdir(parents=True, exist_ok=True)
    symbols, payloads = reference_frames(library)
    (output / "expected-payloads.bin").write_bytes(payloads)
    cases = []
    # Full 360-degree sweep for each stagger orientation, then independent
    # fractional sampling, carrier offset and clock drift controls.
    settings = [(640000, 80000, phase, direction, 0, 0)
                for direction in (-1, 1) for phase in range(0, 360, 15)]
    settings += [(rate, baud, phase, direction, offset, ppm)
                 for rate, baud in [(2048000, 80000), (288000, 72000)]
                 for direction in (-1, 1)
                 for phase, offset, ppm in [(37, 0, 150), (123, 1000, -150), (287, -1000, 150)]]
    if "--sampling" in sys.argv[3:]:
        settings = [(int(baud * sps), baud, phase, direction, offset, ppm)
                    for baud in (72000, 80000) for sps in (2, 2.5, 3)
                    for direction in (-1, 1)
                    for phase, offset, ppm in [(37, 0, 150), (123, 1000, -150), (287, -1000, 150)]]
    for index, (rate, baud, phase, direction, offset, ppm) in enumerate(settings):
        count = int((len(symbols) + 8) * rate / baud)
        seconds = np.arange(count, dtype=np.float64) / rate
        time = seconds * baud * (1 + ppm / 1e6) - 0.27
        iq = branch(symbols[:, 0], time) + 1j * branch(symbols[:, 1], time - direction * 0.5)
        iq *= 0.4 * np.exp(1j * (np.deg2rad(phase) + 2 * np.pi * offset * seconds))
        rng = np.random.default_rng(0x991 + index)
        iq += 0.002 * (rng.standard_normal(count) + 1j * rng.standard_normal(count))
        path = output / f"case-{index:02d}.cf32"
        iq.astype("<c8").tofile(path)
        cases.append({"file": path.name, "sample_rate": rate, "symbol_rate": baud,
                      "phase_degrees": phase, "stagger_direction": direction,
                      "frequency_offset_hz": offset, "clock_drift_ppm": ppm,
                      "samples": count, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    manifest = {"libfec_commit": "9750ca0a6d0a786b506e44692776b541f90daa91",
                "libfec_sha256": hashlib.sha256(library.read_bytes()).hexdigest(),
                "pulse": "analytical RRC beta=0.5, 12-symbol support, fractional start=0.27",
                "noise_component_sigma": 0.002, "preamble_symbols": 5000,
                "expected_payload_sha256": hashlib.sha256(payloads).hexdigest(), "cases": cases}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Generated {len(cases)} independent OQPSK cases in {output}")


if __name__ == "__main__":
    main()
