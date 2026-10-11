"""Full-pass synthetic 80k transmitter; independent libfec and analytical RRC.

Usage: python generate_80k_long_pass.py LIBFEC.so REF.cadu OUT
Convolutional coding is vectorized; temporary permutation arrays are released
before streaming RF generation. No application encoding/DSP is imported.
"""
import ctypes
import hashlib
import json
from pathlib import Path
import sys
import numpy as np
from generate_oqpsk_vectors import branch

library, reference_path, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
reference = reference_path.read_bytes()
assert len(reference) % 1024 == 0
payloads = [reference[i + 4:i + 896] for i in range(0, len(reference), 1024)]
assert len(payloads) >= 7000
fec = ctypes.CDLL(str(library))
fec.init_rs_char.argtypes = [ctypes.c_int] * 6
fec.init_rs_char.restype = ctypes.c_void_p
fec.encode_rs_char.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte), ctypes.POINTER(ctypes.c_ubyte)]
codec = fec.init_rs_char(8, 0x187, 112, 11, 32, 0)
assert codec
pn, state = [], 255
for _ in range(255):
    value = 0
    for _ in range(8):
        value = (value << 1) | (state & 1)
        feedback = (state ^ (state >> 3) ^ (state >> 5) ^ (state >> 7)) & 1
        state = (state >> 1) | (feedback << 7)
    pn.append(value)
assert bytes(pn[:8]).hex() == 'ff480ec09a0d70bc'
frames = []
for payload in payloads:
    transport = np.zeros((255, 4), dtype=np.uint8)
    for lane in range(4):
        data = (ctypes.c_ubyte * 223)(*payload[lane::4])
        parity = (ctypes.c_ubyte * 32)()
        fec.encode_rs_char(codec, data, parity)
        transport[:, lane] = list(data) + list(parity)
    frames.append(bytes.fromhex('1acffc1d') + bytes(int(v) ^ pn[i % 255] for i, v in enumerate(transport.ravel())))
rng = np.random.default_rng(1102026)
information = np.concatenate([rng.integers(0, 2, 5000, dtype=np.uint8), np.unpackbits(np.frombuffer(b''.join(frames), dtype=np.uint8)), rng.integers(0, 2, 512, dtype=np.uint8)])
levels = np.bitwise_xor.accumulate(information)
register = levels << 6
for lag in range(1, 7):
    register[lag:] |= levels[:-lag] << (6 - lag)
lookup = np.array([v.bit_count() & 1 for v in range(256)], dtype=np.uint8)
coded = np.empty((len(levels), 2), dtype=np.uint8)
for lane, polynomial in enumerate((0o171, 0o133)):
    coded[:, lane] = lookup[register & polynomial] ^ 1
coded = coded.ravel()
del information, levels, register, frames
coded_count = len(coded)
delay = 35 * 2048 * 36
count = ((coded_count + delay + 512 + 71) // 72) * 72
time = np.arange(count, dtype=np.int64)
source = time - (time % 36) * 2048 * 36
interleaved = np.zeros(count, dtype=np.uint8)
present = (source >= 0) & (source < coded_count)
interleaved[present] = coded[source[present]]
wire = np.concatenate([np.tile(np.unpackbits(np.array([0x27], dtype=np.uint8)), (count // 72, 1)), interleaved.reshape(-1, 72)], axis=1).ravel()
del time, source, interleaved, present, coded
wire = np.concatenate([rng.integers(0, 2, 35, dtype=np.uint8), wire, np.array([0], dtype=np.uint8)])
np.packbits(wire).tofile(output / 'wire.bits')
(output / 'reference.cadu').write_bytes(reference)
(output / 'payloads.bin').write_bytes(b''.join(payloads))
pairs = 1.0 - 2.0 * wire.reshape(-1, 2)
del wire
rate, baud = 320000, 80000
samples = len(pairs) * 4 + 200
path = output / 'long-pass-noise-drift.cf32'
noise_rng = np.random.default_rng(8139)
sigma = .4 / np.sqrt(2 * 10**(30 / 10))
with path.open('wb') as sink:
    for first in range(0, samples, 131072):
        seconds = np.arange(first, min(first + 131072, samples)) / rate
        clock = seconds * baud * (1 + 70 / 1e6) - .27
        iq = .4 * (branch(pairs[:, 0], clock) + 1j * branch(pairs[:, 1], clock - .5))
        iq *= np.exp(1j * (np.deg2rad(37) + 2 * np.pi * (137 * seconds + .5 * seconds**2)))
        iq += sigma * (noise_rng.normal(size=len(iq)) + 1j * noise_rng.normal(size=len(iq)))
        iq.astype('<c8').tofile(sink)
        if first % (131072 * 100) == 0:
            print(f'RF generated: {first / rate:.1f}/{samples / rate:.1f} seconds', flush=True)
sha = hashlib.sha256()
with path.open('rb') as file:
    for block in iter(lambda: file.read(1048576), b''):
        sha.update(block)
case = dict(name='long-pass-noise-drift', file=path.name, sha256=sha.hexdigest(), samples=samples,
            direction=1, phase=37, offset=137, ppm=70, snr_db=30, ramp=1, impairment=False, require_all=True)
manifest = dict(profile='Oqpsk80Interleaved', sample_rate_hz=rate, symbol_rate_hz=baud, reference_frames=len(payloads),
    duration_seconds=samples / rate, coded_bits=coded_count, cases=[case],
    reference_sha256=hashlib.sha256(reference).hexdigest(),
    libfec_commit='9750ca0a6d0a786b506e44692776b541f90daa91',libfec_sha256=hashlib.sha256(library.read_bytes()).hexdigest(),
    generator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    provenance='Synthetic full-pass 80k waveform carrying original independently decoded public 72k packet/image content. Not an off-air 80k capture.')
(output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps(manifest, indent=2))
