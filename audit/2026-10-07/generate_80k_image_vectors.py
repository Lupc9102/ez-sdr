"""Independent 80k waveform generator using real independently decoded image data.

Usage: python generate_80k_image_vectors.py LIBFEC.so DEINTERLEAVER.so REF.cadu OUT
REF contains corrected original CADUs, not application-produced frames. Public
72k packet/image content is remodulated as synthetic 80k; it is NOT off-air 80k.
No application encoding, transport, filtering or demodulation code is imported.
"""
import ctypes
import hashlib
import json
from pathlib import Path
import sys
import numpy as np
from generate_oqpsk_vectors import branch

fec_path, oracle_path, reference_path, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
reference = reference_path.read_bytes()
assert len(reference) % 1024 == 0
payloads = [reference[i + 4:i + 896] for i in range(0, len(reference), 1024)]
assert len(payloads) >= 100
fec = ctypes.CDLL(str(fec_path))
fec.init_rs_char.argtypes = [ctypes.c_int] * 6
fec.init_rs_char.restype = ctypes.c_void_p
fec.encode_rs_char.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte), ctypes.POINTER(ctypes.c_ubyte)]
codec = fec.init_rs_char(8, 0x187, 112, 11, 32, 0)
assert codec
pn, state = [], 255
for _ in range(255):
    value = 0
    for bit in range(8):
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
state, coded = 0, []
for bit in levels:
    register = (int(bit) << 6) | state
    coded.extend(((register & polynomial).bit_count() & 1) ^ 1 for polynomial in (0o171, 0o133))
    state = register >> 1
coded = np.array(coded, dtype=np.uint8)
delay = 35 * 2048 * 36
count = ((len(coded) + delay + 512 + 71) // 72) * 72
time = np.arange(count, dtype=np.int64)
source = time - (time % 36) * 2048 * 36
interleaved = np.zeros(count, dtype=np.uint8)
present = (source >= 0) & (source < len(coded))
interleaved[present] = coded[source[present]]
wire = np.concatenate([np.tile(np.unpackbits(np.array([0x27], dtype=np.uint8)), (count // 72, 1)), interleaved.reshape(-1, 72)], axis=1).ravel()
# Independently decode the transmitted permutation with the pinned native C
# receiver. All useful coded bits must match, including its large startup delay.
oracle = ctypes.CDLL(str(oracle_path))
oracle.deinterleave_num_samples.argtypes = [ctypes.c_size_t]
oracle.deinterleave_num_samples.restype = ctypes.c_size_t
oracle.deinterleave.argtypes = [ctypes.POINTER(ctypes.c_int8), ctypes.POINTER(ctypes.c_int8), ctypes.c_size_t]
expected = np.zeros(count, dtype=np.int8)
wire_signed = wire.astype(np.int8)
pointer = ctypes.POINTER(ctypes.c_int8)
consumed = 0
for first in range(0, count, 4096):
    length = min(4096, count - first)
    consumed_now = oracle.deinterleave_num_samples(length)
    oracle.deinterleave(expected[first:].ctypes.data_as(pointer), wire_signed[consumed:].ctypes.data_as(pointer), length)
    consumed += consumed_now
assert consumed == len(wire)
assert np.array_equal(expected[delay:delay + len(coded)], coded)
wire = np.concatenate([rng.integers(0, 2, 35, dtype=np.uint8), wire, np.array([0], dtype=np.uint8)])
pairs = 1.0 - 2.0 * wire.reshape(-1, 2)
(output / 'reference.cadu').write_bytes(reference)
(output / 'payloads.bin').write_bytes(b''.join(payloads))
np.packbits(wire).tofile(output / 'wire.bits')
cases = [
    dict(name='clean-positive', direction=1, phase=37, offset=137, ppm=70, snr_db=None, ramp=0, impairment=False, require_all=True),
    dict(name='clean-negative', direction=-1, phase=287, offset=-137, ppm=-70, snr_db=None, ramp=0, impairment=False, require_all=True),
    dict(name='noise-and-drift', direction=1, phase=123, offset=-1000, ppm=-150, snr_db=25, ramp=7, impairment=False, require_all=True),
    dict(name='fade-and-phase-slip', direction=1, phase=37, offset=137, ppm=70, snr_db=30, ramp=0, impairment=True, require_all=False),
]
rate, baud = 320000, 80000
samples = len(pairs) * 4 + 200
for case in cases:
    path = output / (case['name'] + '.cf32')
    noise_rng = np.random.default_rng(8139)
    with path.open('wb') as sink:
        for first in range(0, samples, 131072):
            seconds = np.arange(first, min(first + 131072, samples)) / rate
            clock = seconds * baud * (1 + case['ppm'] / 1e6) - .27
            iq = .4 * (branch(pairs[:, 0], clock) + 1j * branch(pairs[:, 1], clock - case['direction'] * .5))
            angle = np.deg2rad(case['phase']) + 2 * np.pi * (case['offset'] * seconds + .5 * case['ramp'] * seconds**2)
            if case['impairment']:
                iq[(seconds >= 2) & (seconds < 2.15)] *= .01
                angle += np.where(seconds >= 6, np.pi, 0)
            iq *= np.exp(1j * angle)
            if case['snr_db'] is not None:
                sigma = .4 / np.sqrt(2 * 10**(case['snr_db'] / 10))
                iq += sigma * (noise_rng.normal(size=len(iq)) + 1j * noise_rng.normal(size=len(iq)))
            iq.astype('<c8').tofile(sink)
    case.update(file=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest(), samples=samples)
metadata = dict(profile='Oqpsk80Interleaved', sample_rate_hz=rate, symbol_rate_hz=baud, reference_frames=len(payloads),
    native_deinterleaver_coded_bits_verified=len(coded), cases=cases,
    reference_sha256=hashlib.sha256(reference).hexdigest(),
    libfec_commit='9750ca0a6d0a786b506e44692776b541f90daa91', libfec_sha256=hashlib.sha256(fec_path.read_bytes()).hexdigest(),
    deinterleaver_commit='e7ca456ea969dc4834b5544c3789de1dd5d2f663', deinterleaver_sha256=hashlib.sha256(oracle_path.read_bytes()).hexdigest(),
    provenance='Synthetic 80k waveform carrying original independently decoded public 72k packet/image content. Not an off-air 80k capture.',
    generator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
(output / 'manifest.json').write_text(json.dumps(metadata, indent=2) + '\n')
print(json.dumps(metadata, indent=2))
