# BladeCam2 USB protocol (recovered from DataRayOcx.ocx 8.0E7x)

Source functions (Ghidra, image base 0x180000000) are listed so each claim can be re-checked.

## Transport

- USB `VID 0x1632 / PID 0x2101`, FTDI FT60x (FT600/FT601) bridge, driven through `ftd3xx.dll`
  (loaded dynamically, `FUN_1800382f0`).
- Device is found by `FT_CreateDeviceInfoList` + `FT_GetDeviceInfoDetail`, matching
  `ID == 0x16322101`, then `FT_Create(index, FT_OPEN_BY_INDEX=0x10, &h)`. If the device
  flags have bit 2 (`FT_FLAGS_SUPERSPEED`) set, the link is USB 3.
- Pipes: `0x02` command OUT, `0x82` reply IN, `0x03` (unused OUT), `0x83` video IN.
- `FT_SetPipeTimeout` 500 ms on `0x02,0x03,0x82,0x83` at open; while streaming,
  `max(500, 4 × exposure_ms)`.
- `FT_EnableGPIO(h, mask=3, dir=3)`, `FT_WriteGPIO(h, 3, 0)`.

## Command packets (`FUN_18003be30`)

Every transaction is an 8-byte write to pipe `0x02`:

```
byte 0   0x02
byte 1   0x03                (register transaction)
byte 2   cmd                 (FPGA command, table below)
byte 3-4 arg1  (u16 LE)
byte 5-6 arg2  (u16 LE)
byte 7   0x00
```

followed by a 4-byte read from pipe `0x82`: `[0x02, ?, lo, hi]` → result `hi<<8 | lo`.
Failure (status ≠ 0 or short transfer) triggers recovery (`FUN_18003abc0`):
GPIO pulse `WriteGPIO(3,3)`, 10 ms, `WriteGPIO(3,0)`, then `FT_AbortPipe` +
`FT_ClearStreamPipe(h,0,0,pipe)` on `0x83` and `0x82`.

Special packets (byte 1 = `0x01`, no reply read, sleep 500 ms afterwards):
`02 01 93 00 00 00 00 00` (reconfigure FPGA, used at every open, `FUN_18003aac0`) and
`02 01 97 …` (used before firmware update only).

| cmd  | arg1 / arg2          | meaning                                                   |
|------|----------------------|-----------------------------------------------------------|
| 0x01 | –                    | read firmware version (DataRay stores `ver - 0x66`)       |
| 0x02 | value                | write scratch register                                   |
| 0x03 | –                    | read scratch register (open test expects `0x4243`)        |
| 0x04 | 0 / 1                | video stream disable / enable                             |
| 0x06 | sensor reg / value   | write image-sensor register (I²C via FPGA)                |
| 0x07 | sensor reg           | read image-sensor register                                |
| 0x09 | byte index 0..55     | read one byte of the on-camera calibration block          |
| 0x0A | 1                    | request one frame                                         |
| 0x0B | –                    | apply sensor configuration (after init tables)            |
| 0x0C | 1 then 0             | sensor reset pulse (100 ms each)                          |
| 0x0D | width                | output frame width (after binning)                        |
| 0x0E | height               | output frame height                                       |
| 0x0F | 0 / 1                | special-frame mode (DataRay's auto-background); keep 0    |
| 0x11 | 2 / 1                | sensor standby around reset (2 = before, 1 = after)       |
| 0x90–0x92, 0x95 | –         | firmware-update / FPGA-config control. **Not used by us.** |

## Open sequence (`FUN_1800394a0`)

1. timeouts, GPIO as above
2. `cmd02(0x4243)`, `cmd03`, `cmd95`, reconfigure `02 01 93`, sleep 500 ms
3. `cmd02(0x4243)`; `cmd03` must return `0x4243`
4. `cmd01` → firmware version
5. `cmd11(2)`, `cmd0C(1)`, 100 ms, `cmd0C(0)`, 100 ms, `cmd11(1)`
6. read back sensor registers with `cmd07` (reg 0 = chip version)
7. `cmd09` × 56 → calibration block (starts with `.IRD`)

## Sensor variants (`FUN_1800382f0` tail)

| chip version (sensor reg 0) | sensor   | full res   | pitch  | DataRay type | default HBLANK |
|-----------------------------|----------|------------|--------|--------------|----------------|
| `0x1621`                    | MT9T001  | 2048×1536  | 3.2 µm | 0x2B (XHR)   | 0x15           |
| anything else               | MT9M001  | 1280×1024  | 5.2 µm | 0x2A (HR)    | 0x13           |

Pixel clock 48 MHz (`2.0833e-5 ms` per tick).

### Window / readout (`FUN_18003acd0` MT9M001, `FUN_18003ae10` MT9T001)

ROI `{x, w, y, h}` in full-sensor pixels. Binning mode `bin`: `0` → 2×2, `1` → 1×1, `4` → 4×4.

MT9M001: `R1E=0x8000`, `R01=y+0x0C`, `R02=x+0x14`, `R03=(h-1)|1`, `R04=(w-1)|1`, `R05=hblank`,
`R62=0x498`, `R1E = (bin==4 ? 0x800C : 0x8000)`, `R20 = (bin==0 ? 0x111D : 0x1105)`.

MT9T001: `R1E=0xC040`, `R01=y+0x14`, `R02=x+0x20`, `R03=(h-1)|1`, `R04=(w-1)|1`,
`R22=R23 = {bin0:1, bin1:0, bin4:3}`, `R62=0`.

Then `cmd0B`. Output size: `w/b × h/b` with `b = {0:2, 1:1, 4:4}`.

### Exposure (`FUN_18003b0e0`), `t` in ms, `T = 2.0833333e-5`

MT9M001: `row = w + HB + 0xE2`; `t = (SW·row − 180)·T`.
Set: `HB = 0x13`, `SW = max(1, floor(t/T / (w+0xF5)))`, leftover
`r = t/T − SW·(w+0xF5)`, if `r ≥ SW` then `HB = r/SW + 0x13`.
Write `R09 = SW & 0x3FFF`, `R05 = HB & 0x7FF`.

MT9T001: `row = max(0x287, HB + 0x171 + w_out)`; `t = (row·SW − 199)·T`.
Set: `HB = 0x15`, `SW = max(1, floor(t/T / max(0x287, w_out+0x186)))`.
Write `R09 = SW & 0xFFFF`, `R08 = SW >> 16`, `R05 = HB`.

### Gain (`FUN_18003bfa0`)

`R35 = table[gain − 1]`, gain 1..44. Tables (identical up to index 36):
`0C..20` (1.5×..4× in 0.125 steps), `51..60`, then MT9M001 `61..67`,
MT9T001 `0160,0260,…,0760`.

### Black level (`FUN_18003b8c0`)

MT9M001: `R5F = v | (v+8)<<8 | 0x8000`, read back `R5F & 0x3F`.
MT9T001: `R49 = (v & 0x3FF) << 2`, read back `R49 >> 2`.

## Streaming (`FUN_18003c100`)

Setup: `cmd04(0)`, sensor init, exposure, `cmd0F(0)`, timeouts, `cmd0D(w_out)`, `cmd0E(h_out)`, `cmd04(1)`.

Per frame: `cmd0A(1)`, `FT_ReadPipe(0x83, buf, w_out·h_out·2)`.
Validity: status 0, bytes > 0, **bit 15 of the first word set** (frame-start marker).
Bit 14 of the first word flags the special frame requested with `cmd0F(1)`.
On error: GPIO pulse, abort/clear pipes, `cmd04(0)`, redo setup.

Pixel format (`FUN_18003bbe0`): 16-bit LE words, 10-bit sample in bits 0..9; DataRay
scales with `word << 6` (the marker bits fall off the top).

## DataRay post-processing, for reference

- Stats (`FUN_18003a8d0`): 3-tap horizontal smoothed `(a+2b+c)/4` → peak, min, mean over the
  interior.
- Background subtraction (`FUN_18003b9e0`): `clamp(p − bg + 0x147, 1, 0xFFFF)`, `bg` averaged
  over 5 frames (`FUN_18003dbd0`).
- Auto-exposure (`FUN_180039920`): `k = (57343.125/peak + 1)/2` clamped to `[0.15, 7]`;
  apply if `k ∉ [0.95,1.05]` or `peak > 60292`; use `sqrt(k)` unless `peak ≥ 62258`;
  exposure capped at 2000 ms.
