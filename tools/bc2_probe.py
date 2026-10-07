"""Stand-alone BladeCam2 probe: handshake, sensor ID, calibration block, one frame.

Validates the protocol in docs/bladecam2-protocol.md against real hardware.
DataRay must be closed (the device can only be opened once).

usage: python tools/bc2_probe.py [exposure_ms] [--no-frame]
"""
import ctypes as C
import os
import struct
import sys
import time

DLL = os.path.join(os.environ.get("ProgramFiles", r"C:\Program Files"), "DataRay", "FTD3XX.dll")
ft = C.WinDLL(DLL)
H = C.c_void_p
U32 = C.c_uint32


def ok(st, what):
    if st != 0:
        raise RuntimeError(f"{what} -> FT_STATUS {st}")


n = U32()
ok(ft.FT_CreateDeviceInfoList(C.byref(n)), "FT_CreateDeviceInfoList")
print("devices:", n.value)
idx = None
for i in range(n.value):
    flags, typ, did, loc = U32(), U32(), U32(), U32()
    ser, desc = C.create_string_buffer(32), C.create_string_buffer(64)
    h = H()
    ft.FT_GetDeviceInfoDetail(U32(i), C.byref(flags), C.byref(typ), C.byref(did), C.byref(loc),
                              ser, desc, C.byref(h))
    print(f"  [{i}] flags={flags.value:#x} type={typ.value} id={did.value:#010x} "
          f"serial={ser.value!r} desc={desc.value!r}")
    if did.value == 0x16322101 and idx is None:
        idx = i
if idx is None:
    sys.exit("no BladeCam2")

h = H()
ok(ft.FT_Create(C.c_void_p(idx), U32(0x10), C.byref(h)), "FT_Create")


def write(pipe, data):
    buf = (C.c_ubyte * len(data)).from_buffer_copy(bytes(data))
    got = U32()
    st = ft.FT_WritePipe(h, C.c_ubyte(pipe), buf, U32(len(data)), C.byref(got), None)
    return st, got.value


def read(pipe, size):
    buf = (C.c_ubyte * size)()
    got = U32()
    st = ft.FT_ReadPipe(h, C.c_ubyte(pipe), buf, U32(size), C.byref(got), None)
    return st, bytes(buf[:got.value])


def reg(c, a1=0, a2=0):
    st, n_ = write(0x02, [2, 3, c, a1 & 0xFF, a1 >> 8, a2 & 0xFF, a2 >> 8, 0])
    if st or n_ != 8:
        raise RuntimeError(f"cmd {c:#x} write st={st} n={n_}")
    st, r = read(0x82, 4)
    if st or len(r) != 4:
        raise RuntimeError(f"cmd {c:#x} reply st={st} r={r.hex()}")
    return r[2] | r[3] << 8, r


try:
    for p in (0x82, 0x02, 0x03, 0x83):
        ok(ft.FT_SetPipeTimeout(h, C.c_ubyte(p), U32(500)), "FT_SetPipeTimeout")
    ok(ft.FT_EnableGPIO(h, U32(3), U32(3)), "FT_EnableGPIO")
    ok(ft.FT_WriteGPIO(h, U32(3), U32(0)), "FT_WriteGPIO")

    for c, a in ((2, 0x4243), (3, 0), (0x95, 0)):
        try:
            v, r = reg(c, a)
            print(f"pre cmd {c:#04x} -> {v:#06x} raw {r.hex()}")
        except RuntimeError as e:
            print("pre", e)
    print("reconfigure:", write(0x02, [2, 1, 0x93, 0, 0, 0, 0, 0]))
    time.sleep(0.5)
    reg(2, 0x4243)
    v, r = reg(3)
    print(f"scratch echo {v:#06x} ({'OK' if v == 0x4243 else 'FAIL'}) raw {r.hex()}")
    fw, _ = reg(1)
    print(f"firmware raw {fw:#x} -> {fw - 0x66}")
    reg(0x11, 2); reg(0x0C, 1); time.sleep(0.1); reg(0x0C, 0); time.sleep(0.1); reg(0x11, 1)
    chip, _ = reg(7, 0)
    xhr = chip == 0x1621
    print(f"sensor chip version {chip:#06x} -> {'MT9T001 (XHR)' if xhr else 'MT9M001 (HR)'}")
    cal = bytes(reg(9, i)[0] & 0xFF for i in range(56))
    print("calibration:", cal[:4], [struct.unpack_from('<d', cal, o)[0] for o in (8, 16)], cal.hex())

    if "--no-frame" in sys.argv:
        raise SystemExit
    exp_ms = float(next((a for a in sys.argv[1:] if not a.startswith("--")), "1.0"))
    W, Hh = (2048, 1536) if xhr else (1280, 1024)
    BIN2 = "--bin2" in sys.argv
    T = 2.0833333333333336e-05
    reg(4, 0)
    if xhr:
        for r_, v_ in ((0x1e, 0xC040), (0x01, 0x14), (0x02, 0x20), (0x03, (Hh - 1) | 1),
                       (0x04, (W - 1) | 1), (0x23, 0), (0x22, 0), (0x62, 0)):
            reg(6, r_, v_)
        row = max(0x287, W + 0x186); sw = max(1, int(exp_ms / T / row))
        reg(6, 0x09, sw & 0xFFFF); reg(6, 0x08, sw >> 16); reg(6, 0x05, 0x15)
    else:
        for r_, v_ in ((0x1e, 0x8000), (0x01, 0x0C), (0x02, 0x14), (0x03, (Hh - 1) | 1),
                       (0x04, (W - 1) | 1), (0x05, 0x13), (0x62, 0x498), (0x20, 0x111D if BIN2 else 0x1105)):
            reg(6, r_, v_)
        sw = max(1, min(0x3FFF, int(exp_ms / T / (W + 0xF5))))
        reg(6, 0x09, sw); reg(6, 0x05, 0x13)
    reg(6, 0x35, 0x0C)
    reg(0x0B)
    reg(0x0F, 0)
    to = max(500, int(exp_ms * 4))
    for p in (0x82, 0x02, 0x03, 0x83):
        ft.FT_SetPipeTimeout(h, C.c_ubyte(p), U32(to))
    if BIN2:
        W, Hh = W // 2, Hh // 2
    reg(0x0D, W); reg(0x0E, Hh); reg(4, 1)
    for k in range(3):
        t0 = time.perf_counter()
        reg(0x0A, 1)
        st, data = read(0x83, W * Hh * 2)
        dt = (time.perf_counter() - t0) * 1000
        if not data:
            print(f"frame {k}: st={st} bytes=0 ({dt:.1f} ms)"); continue
        words = struct.unpack(f"<{len(data)//2}H", data)
        vals = [w & 0x3FF for w in words]
        print(f"frame {k}: st={st} bytes={len(data)} ({dt:.1f} ms) first={words[0]:#06x} "
              f"min={min(vals)} max={max(vals)} mean={sum(vals)/len(vals):.1f} "
              f"highbits={sorted(set(w >> 10 for w in words))[:8]}")
    open(os.path.join(os.path.dirname(__file__), "..", "re", "bc2_frame.raw"), "wb").write(data)
    print(f"saved re/bc2_frame.raw ({W}x{Hh} u16le)")
    reg(4, 0)
finally:
    ft.FT_Close(h)
