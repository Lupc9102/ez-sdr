"""Decode ideal transmitted 80k bits with pinned independent native C primitives.

Usage: python verify_80k_native_reference.py liblrpt.so VECTOR_DIR
The legacy CLI's -d is pre-Viterbi per-quadrature differential decoding. Modern
SatDump m2x_mode uses post-Viterbi NRZ-M instead. This adapter uses that documented
modern convention, with UNMODIFIED C deinterleave/Viterbi/descrambler/RS routines.
It validates ideal transport bytes; RF impairments are tested by the actual app.
"""
import ctypes
import hashlib
import json
from pathlib import Path
import sys
import numpy as np

library, directory = map(Path, sys.argv[1:])
manifest = json.loads((directory / 'manifest.json').read_text())
wire = np.unpackbits(np.frombuffer((directory / 'wire.bits').read_bytes(), dtype=np.uint8))[35:]
length = len(wire) // 80 * 80
wire = wire[:length].astype(np.int8)
assert np.all(wire.reshape(-1, 80)[:, :8] == np.unpackbits(np.array([0x27], dtype=np.uint8)))
count = length // 80 * 72
native = ctypes.CDLL(str(library))
ptr = ctypes.POINTER(ctypes.c_int8)
native.deinterleave.argtypes = [ptr, ptr, ctypes.c_size_t]
native.deinterleave_num_samples.argtypes = [ctypes.c_size_t]
native.deinterleave_num_samples.restype = ctypes.c_size_t
coded = np.zeros(count, dtype=np.int8)
consumed = 0
for first in range(0, count, 4096):
    n = min(4096, count - first)
    used = native.deinterleave_num_samples(n)
    native.deinterleave(coded[first:].ctypes.data_as(ptr), wire[consumed:].ctypes.data_as(ptr), n)
    consumed += used
assert consumed == length
# Native Viterbi consumes the two polynomial outputs in opposite pair order.
# Its fixed input order is explicit here, not inferred by searching payloads.
soft = ((1 - 2 * coded.astype(np.int16)) * 100).astype(np.int8)
soft = soft.reshape(-1, 2)[:, ::-1].copy().ravel()
output = np.zeros(len(soft) // 64 * 4, dtype=np.uint8)
native.viterbi_init()
native.viterbi_decode.argtypes = [ctypes.POINTER(ctypes.c_uint8), ptr, ctypes.c_int]
for first in range(0, len(output), 256):
    n = min(256, len(output) - first)
    native.viterbi_decode(output[first:].ctypes.data_as(ctypes.POINTER(ctypes.c_uint8)), soft[first * 16:].ctypes.data_as(ptr), n)
levels = np.unpackbits(output)
bits = levels ^ np.concatenate([np.array([0], dtype=np.uint8), levels[:-1]])
decoded = np.packbits(bits).tobytes()
native.descramble_init()
native.rs_init()
native.descramble.argtypes = [ctypes.c_void_p]
native.rs_fix.argtypes = [ctypes.c_void_p]
native.rs_fix.restype = ctypes.c_int
frames, corrections, position = [], [], 0
marker = bytes.fromhex('1acffc1d')
while True:
    position = decoded.find(marker, position)
    if position == -1:
        break
    raw = decoded[position:position + 1024]
    assert len(raw) == 1024
    frame = (ctypes.c_uint8 * 1024).from_buffer_copy(raw)
    native.descramble(frame)
    correction = native.rs_fix(ctypes.byref(frame, 4))
    assert correction >= 0
    frames.append(bytes(frame))
    corrections.append(correction)
    position += 1024
expected = (directory / 'reference.cadu').read_bytes()
assert len(frames) == manifest['reference_frames']
assert b''.join(frame[4:896] for frame in frames) == b''.join(expected[i + 4:i + 896] for i in range(0, len(expected), 1024))
reference = b''.join(frames)
(directory / 'native-reference.cadu').write_bytes(reference)
result = dict(reference_frames=len(frames), corrected_payload_bytes=len(frames) * 892,
    rs_corrected_symbols=sum(corrections), reference_sha256=hashlib.sha256(reference).hexdigest(),
    native_library_sha256=hashlib.sha256(library.read_bytes()).hexdigest(),
    adapter_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    differential_convention='post-Viterbi NRZ-M, matching modern SatDump 80k m2x_mode',
    convention_source='https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/plugins/meteor_support/meteor/module_meteor_lrpt_decoder.cpp',
    scope='Independent full native-C transport decode of ideal transmitted wire bits. Actual app RF cases additionally include pulse shaping, clock/carrier drift, noise, fade and phase slip.')
(directory / 'native-reference.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
