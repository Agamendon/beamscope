"""Grep ASCII / UTF-16 strings in a PE file and print them with their virtual addresses.

usage: python tools/pestr.py <pe-file> <regex> [--wide]
"""
import re
import struct
import sys


def sections(b):
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    nsec = struct.unpack_from("<H", b, pe + 6)[0]
    optsz = struct.unpack_from("<H", b, pe + 20)[0]
    magic = struct.unpack_from("<H", b, pe + 24)[0]
    base = struct.unpack_from("<Q" if magic == 0x20B else "<I", b, pe + 24 + (24 if magic == 0x20B else 28))[0]
    out = []
    off = pe + 24 + optsz
    for i in range(nsec):
        name = b[off:off + 8].rstrip(b"\0").decode()
        vsz, va, rsz, raw = struct.unpack_from("<IIII", b, off + 8)
        out.append((name, base + va, vsz, raw, rsz))
        off += 40
    return base, out


def off2va(secs, o):
    for name, va, vsz, raw, rsz in secs:
        if raw <= o < raw + rsz:
            return va + (o - raw), name
    return None, None


def main():
    fn, pat = sys.argv[1], sys.argv[2]
    wide = "--wide" in sys.argv
    b = open(fn, "rb").read()
    _, secs = sections(b)
    rx = re.compile(pat, re.I)
    if wide:
        for m in re.finditer(rb"(?:[\x20-\x7e]\x00){4,}", b):
            s = m.group().decode("utf-16le")
            if rx.search(s):
                va, sec = off2va(secs, m.start())
                print(f"{va:x} {sec} W {s!r}")
    else:
        for m in re.finditer(rb"[\x20-\x7e\t\r\n]{4,}", b):
            s = m.group().decode("ascii")
            if rx.search(s):
                va, sec = off2va(secs, m.start())
                print(f"{va:x} {sec} A {s!r}")


if __name__ == "__main__":
    main()
