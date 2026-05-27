"""Convert an Apple CgBI (pngcrush -iphone) PNG into a standard PNG.

Apple's iOS PNG variant differs from spec in three ways:
  1. Carries an extra CgBI chunk right after the signature.
  2. IDAT is raw deflate, not zlib-wrapped.
  3. Pixel channels are BGRA with premultiplied alpha.

Reverses all three so PIL/System.Drawing/etc. can open the file.

Usage:  python defry_ios_png.py <input.png> <output.png>
"""

import struct
import sys
import zlib

from PIL import Image


def defry(src_bytes: bytes) -> bytes:
    if src_bytes[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit("not a PNG")

    pos = 8
    chunks = []
    has_cgbi = False
    while pos < len(src_bytes):
        length = struct.unpack(">I", src_bytes[pos:pos + 4])[0]
        ctype = src_bytes[pos + 4:pos + 8]
        data = src_bytes[pos + 8:pos + 8 + length]
        # crc consumed but not needed since we'll re-emit
        pos += 12 + length
        if ctype == b"CgBI":
            has_cgbi = True
            continue
        chunks.append((ctype, data))

    if not has_cgbi:
        # Already a standard PNG; just pass through.
        return src_bytes

    # IHDR is always first.
    ihdr = next(d for t, d in chunks if t == b"IHDR")
    width, height = struct.unpack(">II", ihdr[:8])
    bit_depth, color_type = ihdr[8], ihdr[9]
    if bit_depth != 8 or color_type != 6:
        raise SystemExit(
            f"defry only supports 8-bit RGBA (got bit_depth={bit_depth}, "
            f"color_type={color_type})"
        )

    # Collect IDAT, decompress with raw deflate (-MAX_WBITS = no zlib header).
    idat_blob = b"".join(d for t, d in chunks if t == b"IDAT")
    raw = zlib.decompress(idat_blob, -zlib.MAX_WBITS)

    # Strip per-scanline filter bytes (PNG filter type 0..4), reapply None.
    stride = width * 4
    pixels = bytearray(width * height * 4)
    prev_row = bytes(stride)
    for y in range(height):
        ftype = raw[y * (stride + 1)]
        row = raw[y * (stride + 1) + 1: (y + 1) * (stride + 1)]
        # Unfilter to get raw pixel bytes.
        if ftype == 0:  # None
            unfiltered = bytes(row)
        elif ftype == 1:  # Sub
            unf = bytearray(row)
            for i in range(4, stride):
                unf[i] = (unf[i] + unf[i - 4]) & 0xFF
            unfiltered = bytes(unf)
        elif ftype == 2:  # Up
            unfiltered = bytes((a + b) & 0xFF for a, b in zip(row, prev_row))
        elif ftype == 3:  # Average
            unf = bytearray(stride)
            for i in range(stride):
                left = unf[i - 4] if i >= 4 else 0
                up = prev_row[i]
                unf[i] = (row[i] + (left + up) // 2) & 0xFF
            unfiltered = bytes(unf)
        elif ftype == 4:  # Paeth
            unf = bytearray(stride)
            for i in range(stride):
                a = unf[i - 4] if i >= 4 else 0
                b = prev_row[i]
                c = prev_row[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                if pa <= pb and pa <= pc:
                    pred = a
                elif pb <= pc:
                    pred = b
                else:
                    pred = c
                unf[i] = (row[i] + pred) & 0xFF
            unfiltered = bytes(unf)
        else:
            raise SystemExit(f"unknown filter type {ftype} at row {y}")

        # Swap BGRA -> RGBA and un-premultiply alpha.
        out_row = bytearray(stride)
        for i in range(0, stride, 4):
            b, g, r, a = unfiltered[i], unfiltered[i + 1], unfiltered[i + 2], unfiltered[i + 3]
            if a != 0 and a != 255:
                r = min(255, (r * 255 + a // 2) // a)
                g = min(255, (g * 255 + a // 2) // a)
                b = min(255, (b * 255 + a // 2) // a)
            out_row[i:i + 4] = bytes((r, g, b, a))
        pixels[y * stride:(y + 1) * stride] = out_row
        prev_row = unfiltered

    # Hand the clean pixel buffer to Pillow to re-encode as a standard PNG.
    img = Image.frombytes("RGBA", (width, height), bytes(pixels))
    import io
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    src_path, dst_path = sys.argv[1], sys.argv[2]
    with open(src_path, "rb") as f:
        src = f.read()
    out = defry(src)
    with open(dst_path, "wb") as f:
        f.write(out)
    print(f"defried {src_path} ({len(src):,} B) -> {dst_path} ({len(out):,} B)")


if __name__ == "__main__":
    main()
