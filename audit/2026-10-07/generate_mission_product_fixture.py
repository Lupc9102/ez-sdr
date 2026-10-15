"""Generate a 72k NRZ-M image-product fixture using external libfec and analytical RRC.

Usage: python generate_mission_product_fixture.py LIBFEC.so OUTPUT_DIR
libfec/reference pins and carrier convention: generate_oqpsk_vectors.py.
The embedded JPEG entropy/pixels come from the independently decoded libjpeg fixture.
"""
import sys
import json
import hashlib
from pathlib import Path
import numpy as np
from generate_oqpsk_vectors import reference_frames, branch

library, output = map(Path, sys.argv[1:3])
output.mkdir(parents=True, exist_ok=True)
segment = (Path(__file__).parent / "fixtures/nonflat-q50.segment").read_bytes()
symbols, payloads = reference_frames(library, segment)
pairs = symbols.reshape(-1, 2)
rate, baud = 288000, 72000
clock = np.arange(len(pairs) * 4 + 200) / 4 * (1 + 70 / 1e6) - .27
iq = .4 * (branch(pairs[:, 0], clock) + 1j * branch(pairs[:, 1], clock - .5))
iq *= np.exp(1j * (np.deg2rad(37) + 2 * np.pi * 137 * np.arange(len(clock)) / rate))
iq_path = output / "meteor-product-72k.cf32"
iq.astype("<c8").tofile(iq_path)
(output / "meteor-product-72k-payloads.bin").write_bytes(payloads)
metadata = {"profile": "Oqpsk72", "sample_rate": rate, "symbol_rate": baud,
            "carrier_phase_degrees": 37, "carrier_offset_hz": 137, "clock_drift_ppm": 70,
            "stagger_direction": 1, "cadus": 3, "image_apid": 64, "image_segments": 3,
            "packet_sequences": [0, 43, 86], "libfec_commit": "9750ca0a6d0a786b506e44692776b541f90daa91",
            "iq_sha256": hashlib.sha256(iq_path.read_bytes()).hexdigest(),
            "payloads_sha256": hashlib.sha256(payloads).hexdigest(),
            "instrument_segment_sha256": hashlib.sha256(segment).hexdigest()}
(output / "meteor-product-72k.json").write_text(json.dumps(metadata, indent=2) + "\n")
print(json.dumps(metadata))
