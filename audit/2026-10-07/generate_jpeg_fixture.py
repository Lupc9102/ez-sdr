"""Independent JPEG control: Pillow/libjpeg, standard luminance tables at quality 50.

The segment contains 14 grayscale MCUs in a single row, matching MSU-MR geometry.
JPEG markers and byte stuffing are removed; only the instrument entropy payload
is passed to the application. This verifies nonzero DC/AC decoding, not RF validity.
"""
from io import BytesIO
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent / "fixtures"
root.mkdir(exist_ok=True)
image = Image.new("L", (112, 8))
image.putdata([(x * 3 + y * 17 + (70 if (x // 8) % 2 else 0)) % 256
               for y in range(8) for x in range(112)])
stream = BytesIO()
image.save(stream, format="JPEG", quality=50, optimize=False, progressive=False)
jpeg = stream.getvalue()
decoded = Image.open(BytesIO(jpeg)).convert("L")
cursor = 2
while jpeg[cursor:cursor + 2] != b"\xff\xda":
    length = int.from_bytes(jpeg[cursor + 2:cursor + 4], "big")
    cursor += 2 + length
length = int.from_bytes(jpeg[cursor + 2:cursor + 4], "big")
entropy = jpeg[cursor + 2 + length:-2].replace(b"\xff\x00", b"\xff")
header = bytearray(14)
header[11:14] = b"\xff\xf0\x32"
(root / "nonflat-q50.segment").write_bytes(header + entropy)
(root / "nonflat-q50.pixels").write_bytes(decoded.tobytes())
print(f"Pillow/libjpeg fixture: entropy={len(entropy)} bytes; expected={112 * 8} pixels")
