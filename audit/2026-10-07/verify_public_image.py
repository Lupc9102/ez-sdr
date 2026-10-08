"""Independent full-pass image oracle: corrected public CADUs + Pillow/libjpeg.

No application parser, sequence unwrap, quantizer or IDCT is imported. Standard
JPEG headers come from Pillow; each mission entropy payload is byte-stuffed and
decoded by libjpeg. CDS timestamps locate rows independently of packet counters.

Usage: python verify_public_image.py REFERENCE.cadu REPAIRED_DIR ORACLE_DIR
"""

from collections import Counter, defaultdict
from io import BytesIO
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image


def space_packets(data):
    pending, last = {}, {}
    for offset in range(0, len(data), 1024):
        frame = data[offset:offset + 1024]
        assert len(frame) == 1024 and frame[:4] == bytes.fromhex("1acffc1d")
        vcdu = frame[4:896]  # corrected data; excludes the four RS parity tails
        channel = vcdu[1] & 63
        counter = int.from_bytes(vcdu[2:5], "big")
        pointer = int.from_bytes(vcdu[8:10], "big") & 2047
        zone = vcdu[10:]
        if channel in last and (last[channel] + 1) & 0xFFFFFF != counter:
            pending.pop(channel, None)
        last[channel] = counter
        if pointer == 2046:  # idle-only packet zone
            continue
        if channel in pending:
            old = pending.pop(channel) + (zone if pointer == 2047 else zone[:pointer])
            if len(old) >= 6 and len(old) >= int.from_bytes(old[4:6], "big") + 7:
                yield old[:int.from_bytes(old[4:6], "big") + 7]
            else:
                pending[channel] = old
        if pointer == 2047:
            continue
        assert pointer <= len(zone)
        position = pointer
        while position < len(zone):
            packet = zone[position:]
            if len(packet) < 6:
                pending[channel] = packet
                break
            length = int.from_bytes(packet[4:6], "big") + 7
            if length > len(packet):
                pending[channel] = packet
                break
            yield packet[:length]
            position += length


def jpeg_header(quality):
    baseline = BytesIO()
    Image.new("L", (112, 8)).save(baseline, "JPEG", quality=50)
    standard = Image.open(BytesIO(baseline.getvalue())).quantization[0]
    # Meteor specifies a floating scale rounded at each table entry. libjpeg's
    # quality convenience API truncates the scale first, which differs at q43.
    # Supply exact integer DQT values instead of using that convenience API.
    scale = 5000 / quality if 20 <= quality < 50 else 200 - 2 * quality
    table = [max(1, int(value * scale / 100 + 0.5)) for value in standard]
    assert max(table) <= 255
    stream = BytesIO()
    Image.new("L", (112, 8)).save(stream, "JPEG", qtables=[table],
                                  optimize=False, progressive=False)
    jpeg = stream.getvalue()
    assert Image.open(BytesIO(jpeg)).quantization[0] == table
    cursor = 2
    while jpeg[cursor:cursor + 2] != b"\xff\xda":
        cursor += 2 + int.from_bytes(jpeg[cursor + 2:cursor + 4], "big")
    return jpeg[:cursor + 2 + int.from_bytes(jpeg[cursor + 2:cursor + 4], "big")]


def main():
    source, repaired, output = map(Path, sys.argv[1:])
    output.mkdir(parents=True, exist_ok=True)
    packets = list(space_packets(source.read_bytes()))
    histogram = Counter(int.from_bytes(packet[:2], "big") & 2047 for packet in packets)
    segments = []
    headers = {}
    for packet in packets:
        apid = int.from_bytes(packet[:2], "big") & 2047
        payload = packet[6:]
        if not 64 <= apid <= 69:
            continue
        assert len(payload) > 14 and payload[9:13] == bytes.fromhex("0000fff0")
        assert payload[8] % 14 == 0 and payload[8] // 14 < 14
        days = int.from_bytes(payload[:2], "big")
        millis = int.from_bytes(payload[2:6], "big")
        micros = int.from_bytes(payload[6:8], "big")
        assert days > 0 and millis < 86_400_000 and micros < 1000
        timestamp = days * 86_400_000_000 + millis * 1000 + micros
        quality = payload[13]
        assert 20 <= quality <= 100
        if quality not in headers:
            headers[quality] = jpeg_header(quality)
        jpeg = headers[quality] + payload[14:].replace(b"\xff", b"\xff\x00") + b"\xff\xd9"
        pixels = np.array(Image.open(BytesIO(jpeg)).convert("L"))
        assert pixels.shape == (8, 112)
        segments.append((apid, timestamp, payload[8] // 14, pixels))

    first_time = min(segment[1] for segment in segments)
    # Recover the scan period from consecutive observed timestamps, including
    # missing groups. The packet counter is deliberately not used for placement.
    times = sorted({segment[1] for segment in segments})
    steps = np.diff(times)
    period = float(np.median(steps))
    assert 1_000_000 <= period <= 1_500_000
    last_group = round((times[-1] - first_time) / period)
    masks = defaultdict(dict)
    images = {apid: np.zeros(((last_group + 1) * 8, 1568), dtype=np.uint8)
              for apid in histogram if 64 <= apid <= 69}
    for apid, timestamp, column, pixels in segments:
        row = round((timestamp - first_time) / period)
        assert abs(timestamp - first_time - row * period) < 500  # <0.5 ms drift
        x, y = column * 112, row * 8
        images[apid][y:y + 8, x:x + 112] = pixels
        masks[apid][row] = masks[apid].get(row, 0) | (1 << column)

    comparisons = []
    for apid, expected in images.items():
        Image.fromarray(expected).save(output / f"oracle-{apid}.png")
        actual = np.array(Image.open(repaired / f"apid-{apid}.png").convert("L"))
        assert actual.shape == expected.shape, (apid, actual.shape, expected.shape)
        error = np.abs(actual.astype(np.int16) - expected.astype(np.int16))
        maximum = int(error.max())
        mean = float(error.mean())
        assert maximum <= 1, (apid, maximum, mean)
        comparisons.append({"apid": apid, "width": actual.shape[1], "height": actual.shape[0],
                            "pixels": int(error.size), "max_error": maximum, "mean_error": mean,
                            "oracle_pixel_sha256": hashlib.sha256(expected.tobytes()).hexdigest()})
    valid_segments = sum(mask.bit_count() for channel in masks.values() for mask in channel.values())
    complete_lines = sum(mask == 0x3FFF for channel in masks.values() for mask in channel.values()) * 8
    application = json.loads((repaired / "result.json").read_text())
    assert application["decoded_segments"] == valid_segments
    assert application["complete_lines"] == complete_lines
    assert application["packets"] == {str(apid): count for apid, count in histogram.items()}
    result = {"oracle": "Pillow/libjpeg entropy decode; independent CDS-time placement",
              "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "packet_counts": dict(histogram), "decoded_segments": valid_segments,
              "complete_lines": complete_lines, "measured_group_period_us": period,
              "first_timestamp_us": first_time, "last_timestamp_us": times[-1],
              "comparisons": comparisons}
    text = json.dumps(result, indent=2)
    (output / "verification.json").write_text(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
