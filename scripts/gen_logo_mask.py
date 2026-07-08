#!/usr/bin/env python3
"""Generate the Heptapus boot-splash alpha mask embedded in the kernel (WP-05 step 2).

Decodes heptapus_logo_primary_black.png (8-bit RGBA, non-interlaced) with the standard library
only (zlib), extracts its alpha channel — which is the octopus + "HEPTAPUS GROUP" shape — box-
downscales it to TARGET x TARGET, and writes a raw one-byte-per-pixel coverage mask that the
splash tints per theme. Reproducible: re-run to regenerate the asset.

Usage:  python scripts/gen_logo_mask.py
Output: crates/qos-os-kernel/src/assets/heptapus_logo_mask.bin  (TARGET*TARGET bytes)
"""
import struct
import zlib
import os

SRC = "heptapus_logo_primary_black.png"
OUT = os.path.join("crates", "qos-os-kernel", "src", "assets", "heptapus_logo_mask.bin")
TARGET = 400  # output mask is TARGET x TARGET


def decode_png_alpha(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos = 8
    width = height = bit_depth = color_type = interlace = None
    idat = bytearray()
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        ctype = data[pos + 4:pos + 8]
        chunk = data[pos + 8:pos + 8 + length]
        pos += 12 + length  # length + type + data + CRC
        if ctype == b"IHDR":
            width, height, bit_depth, color_type, _comp, _filt, interlace = struct.unpack(
                ">IIBBBBB", chunk)
        elif ctype == b"IDAT":
            idat += chunk
        elif ctype == b"IEND":
            break
    assert bit_depth == 8 and color_type == 6, f"need 8-bit RGBA, got depth={bit_depth} ct={color_type}"
    assert interlace == 0, "interlaced PNG not supported"

    raw = zlib.decompress(bytes(idat))
    bpp = 4  # RGBA
    stride = width * bpp
    # Defilter scanlines in place.
    out = bytearray(height * stride)
    prev = bytearray(stride)
    ip = 0
    for y in range(height):
        ftype = raw[ip]; ip += 1
        line = bytearray(raw[ip:ip + stride]); ip += stride
        if ftype == 1:  # Sub
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 0xFF
        elif ftype == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:  # Average
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:  # Paeth
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        out[y * stride:(y + 1) * stride] = line
        prev = line
    # Extract alpha channel (byte 3 of each pixel).
    alpha = bytearray(width * height)
    for i in range(width * height):
        alpha[i] = out[i * bpp + 3]
    return width, height, alpha


def box_downscale(alpha, w, h, tw, th):
    out = bytearray(tw * th)
    for ty in range(th):
        sy0 = ty * h // th
        sy1 = max(sy0 + 1, (ty + 1) * h // th)
        for tx in range(tw):
            sx0 = tx * w // tw
            sx1 = max(sx0 + 1, (tx + 1) * w // tw)
            acc = 0
            cnt = 0
            for sy in range(sy0, sy1):
                base = sy * w
                for sx in range(sx0, sx1):
                    acc += alpha[base + sx]
                    cnt += 1
            out[ty * tw + tx] = acc // cnt if cnt else 0
    return out


def main():
    w, h, alpha = decode_png_alpha(SRC)
    print(f"decoded {SRC}: {w}x{h}")
    mask = box_downscale(alpha, w, h, TARGET, TARGET)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    open(OUT, "wb").write(bytes(mask))
    nonzero = sum(1 for b in mask if b)
    print(f"wrote {OUT}: {TARGET}x{TARGET} = {len(mask)} bytes ({nonzero} non-transparent)")


if __name__ == "__main__":
    main()
