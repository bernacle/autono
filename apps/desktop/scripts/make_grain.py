"""Deterministic, static ground texture; never generated at application launch."""
from pathlib import Path
import random
import struct
import zlib

random.seed(7)
rows = b"".join(b"\x00" + bytes(random.randint(96, 160) for _ in range(256)) for _ in range(256))


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", 256, 256, 8, 0, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))
target = Path(__file__).resolve().parent.parent / "src/assets/grain-256.png"
target.parent.mkdir(parents=True, exist_ok=True)
target.write_bytes(png)
