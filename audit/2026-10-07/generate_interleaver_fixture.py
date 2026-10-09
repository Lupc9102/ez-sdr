"""Independent Meteor 80k transport fixture and optional pulse-shaped IQ.

Usage: python generate_interleaver_fixture.py LIBFEC.so DEINTERLEAVER.so OUT [--iq]

The deinterleaver reference build is dbdexter-dev/meteor_decode at
 e7ca456ea969dc4834b5544c3789de1dd5d2f663:
 cc -O2 -shared -fPIC -o deinterleaver.so deinterleave/deinterleave.c
libfec and the independent NRZ-M/convolutional encoder are documented in
 generate_oqpsk_vectors.py. No application transport or DSP code is imported.
"""

import ctypes
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from generate_oqpsk_vectors import reference_frames, branch


def main():
    fec_path, reference_path, directory = map(Path, sys.argv[1:4])
    directory.mkdir(parents=True, exist_ok=True)
    symbols, payloads = reference_frames(fec_path)
    coded = (symbols < 0).astype(np.uint8).ravel()
    delay = 35 * 2048 * 36
    count = ((len(coded) + delay + 512 + 71) // 72) * 72
    time = np.arange(count, dtype=np.int64)
    # Closed-form transmit permutation, independently of the receiver FIFO:
    # branch b delays b * 2048 visits, with 36 coded bits per visit cycle.
    source = time - (time % 36) * 2048 * 36
    interleaved = np.zeros(count, dtype=np.uint8)
    present = (source >= 0) & (source < len(coded))
    interleaved[present] = coded[source[present]]
    marker = np.unpackbits(np.array([0x27], dtype=np.uint8))
    wire = np.concatenate([np.tile(marker, (count // 72, 1)),
                           interleaved.reshape(-1, 72)], axis=1).ravel()
    reference = ctypes.CDLL(str(reference_path))
    reference.deinterleave_num_samples.argtypes = [ctypes.c_size_t]
    reference.deinterleave_num_samples.restype = ctypes.c_size_t
    reference.deinterleave.argtypes = [ctypes.POINTER(ctypes.c_int8),
                                      ctypes.POINTER(ctypes.c_int8), ctypes.c_size_t]
    expected = np.zeros(count, dtype=np.int8)
    input_at = 0
    pointer = ctypes.POINTER(ctypes.c_int8)
    for start in range(0, count, 4096):
        length = min(4096, count - start)
        consumed = reference.deinterleave_num_samples(length)
        reference.deinterleave(expected[start:].ctypes.data_as(pointer),
                               wire[input_at:].ctypes.data_as(pointer), length)
        input_at += consumed
    assert input_at == len(wire)
    assert np.array_equal(expected[delay:delay + len(coded)], coded)
    # Begin inside an arbitrary IQ bit phase; marker acquisition must find it.
    prefix = np.random.default_rng(22026).integers(0, 2, 35, dtype=np.uint8)
    padded_wire = np.concatenate([prefix, wire, np.array([0], dtype=np.uint8)])
    np.packbits(padded_wire).tofile(directory / "meteor-80k-wire.bits")
    np.packbits(expected.astype(np.uint8)).tofile(directory / "meteor-80k-reference.bits")
    (directory / "meteor-80k-payloads.bin").write_bytes(payloads)
    manifest = {
        "wire_bit_count": len(padded_wire), "output_bit_count": count,
        "prefix_bits": len(prefix), "marker": "0x27", "marker_stride_bits": 80,
        "data_per_marker_bits": 72, "branches": 36, "branch_delay_visits": 2048,
        "combined_delay_coded_bits": delay,
        "reference_commit": "e7ca456ea969dc4834b5544c3789de1dd5d2f663",
        "reference_url": "https://github.com/dbdexter-dev/meteor_decode/tree/e7ca456ea969dc4834b5544c3789de1dd5d2f663",
        "reference_library_sha256": hashlib.sha256(reference_path.read_bytes()).hexdigest(),
        "libfec_commit": "9750ca0a6d0a786b506e44692776b541f90daa91",
        "transmit_construction": "closed-form per-branch delay, marker insertion outside permutation",
        "expected_corrected_cadus": 3,
        "files": {},
    }
    if "--iq" in sys.argv[4:]:
        pairs = 1.0 - 2.0 * padded_wire.reshape(-1, 2)
        rate, baud = 320000, 80000
        samples = len(pairs) * 4 + 32
        # Stream analytical pulse evaluation to bound temporary memory.
        for direction in [-1, 1]:
            path = directory / f"meteor-80k-stagger-{direction}.cf32"
            with path.open("wb") as output:
                for first in range(0, samples, 131072):
                    seconds = np.arange(first, min(first + 131072, samples)) / rate
                    clock = seconds * baud * (1 + 70 / 1e6) - .27
                    iq = branch(pairs[:, 0], clock) + 1j * branch(pairs[:, 1], clock - direction * .5)
                    iq *= .4 * np.exp(1j * (np.deg2rad(37) + 2 * np.pi * 137 * seconds))
                    iq.astype("<c8").tofile(output)
            manifest.setdefault("iq_cases", []).append({
                "file": path.name, "sample_rate": rate, "symbol_rate": baud,
                "phase_degrees": 37, "frequency_offset_hz": 137,
                "clock_drift_ppm": 70, "stagger_direction": direction,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            })
    for name in ["meteor-80k-wire.bits", "meteor-80k-reference.bits", "meteor-80k-payloads.bin"]:
        path = directory / name
        manifest["files"][name] = {"bytes": path.stat().st_size,
                                   "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    (directory / "meteor-80k-fixture.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
