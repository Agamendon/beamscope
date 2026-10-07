# BeamScope: a replacement for DataRay's beam-profiling software

BeamScope is a fast, modern replacement for the DataRay app, built for the **BladeCam2**. It uses a
BladeCam2 driver of its own, written from protocol details recovered by reverse engineering
`DataRayOcx.ocx` in Ghidra (through ghidra-mcp). It reads and writes DataRay `.wcf` files.

## Installing

1. Clone the repo and double-click **`installer\BeamScope-Setup.exe`**.
   It installs for the current user only: no admin rights, no wizard, and it takes a few seconds.
   It adds BeamScope to the Start menu and the desktop, and launches it.
2. Windows may show "Windows protected your PC", because the installer isn't code-signed.
   Click **More info → Run anyway**.
3. To uninstall, go to Settings → Apps → Installed apps → BeamScope → Uninstall.

**Camera requirements.** The BladeCam2 needs its USB driver, which is installed with DataRay's software.
If DataRay has ever worked with the camera on that PC, BeamScope will too. FTDI's `ftd3xx.dll` ships
inside the installer. Close DataRay before connecting: only one program can open the camera at a time.
Without a camera, **Simulator** shows a synthetic beam.

## Repository layout

```
installer/            BeamScope-Setup.exe: the installer (Velopack), rebuilt by scripts/build-installer.ps1
beamscope/            Tauri 2 app (Rust backend, React + TypeScript + WebGL frontend)
docs/                 bladecam2-protocol.md: the recovered USB protocol, with Ghidra addresses
tools/                RE helpers (Ghidra HTTP client, PE string finder, camera probe, UI test drivers)
scripts/              build-installer.ps1
capture*.wcf          reference captures saved by DataRay 8.0E70 (used by tests)
```

The decompiled DataRay code (`re/decomp`) and DataRay's own installer are deliberately kept out of git.

## Building

Needs Rust, Node.js 20+, and for the installer the .NET 8 SDK plus `vpk`:

```bash
winget install Microsoft.DotNet.SDK.8
dotnet tool install -g vpk
```

```bash
cd beamscope && npm install && npm run tauri dev      # development
cd beamscope/src-tauri && cargo test --release        # tests
powershell -ExecutionPolicy Bypass -File scripts/build-installer.ps1   # new installer\BeamScope-Setup.exe
```

To release a new version, bump `version` in `beamscope/src-tauri/tauri.conf.json` before running the
script. Velopack upgrades installations in place.

## What it does

- **Live view:** WebGL rendering at the camera's frame rate. About 50 fps at 2×2 binning and about 10 fps at
  full resolution on the BladeCam2-HR (DataRay's own capture reported 37 fps at 2×2). Wheel to zoom,
  drag to pan, double-click to reset. Six colormaps, log scale, saturation highlighting, centroid,
  D4σ ellipse and profile-line overlays.
- **Acquisition:** exposure, auto-exposure (DataRay's rule, but backs off faster when saturated), 44-step gain,
  black level with DataRay-style auto black, full / 2×2 / 4×4 readout, N-frame averaging,
  background capture and subtraction.
- **Metrics:**
  - ISO 11146 second-moment widths (D4σ along X/Y and the principal axes), orientation,
    ellipticity and effective diameter, with the ISO 11146-3 iterative 3× integration area and an
    auto baseline taken from the image border.
  - DataRay's own definitions alongside: area-based effective diameter and thresholded centroid.
  - Optional profiles along the beam's principal axes (u = major, v = minor) through the centroid,
    like DataRay's rotated crosshair. Clip and Gaussian widths are then measured along u/v.
  - Optional overlay of the ISO 11146-3 integration area (the 3× D4σ rectangle used for the moments),
    next to the D4σ ellipse.
  - Two clip-level widths (13.5 % and 50 % by default) and Levenberg–Marquardt Gaussian fits
    (1/e² diameter and r²) on the line profiles.
  - Centroid, peak with a saturation warning, and relative total power.
- **Logging:** records any metric per frame or at a set interval, with a strip chart, mean/σ/min/max and CSV export.
- **M²:** add points at z positions (D4σ, Gaussian or clip width), fit d²(z) = A + Bz + Cz²
  per ISO 11146-1, and get M², waist, waist position, Rayleigh length and divergence for X and Y,
  plus the ISO sampling-rule warnings. Points can be toggled for exclusion, and results export to CSV.
- **Files:** opens `.wcf` files from DataRay 5.x–8.x and saves `.wcf` that DataRay can load. Also exports a PNG
  snapshot, pixel CSV and profile CSV.

Keyboard: `Space` start/stop, `Ctrl+O` open, `Ctrl+S` save.

## Validation

- On DataRay's reference files, the widths agree:

  | File | BeamScope D4σ | BeamScope Gaussian 1/e² | BeamScope 13.5 % clip | DataRay (stored 13.5 % clip) |
  |---|---|---|---|---|
  | `Test_Perfect_Gaussian_2mm.wcf` | 1998.9 µm | 2000.00 µm | 2001.2 µm | 1999.75 µm |
  | `Test_Perfect_Gaussian_100um..wcf` | 101.1 µm | 100.42 µm | 100.69 µm | 100.70 µm |

- On both BladeCam2 captures saved by DataRay (`capture.wcf`, `capture_again.wcf`), BeamScope reproduces
  DataRay's stored results exactly. Its "effective diameter" comes out at 193.896 / 201.558 µm, and its
  thresholded centroid matches to 4 decimals. DataRay computes both in `FUN_180195090`: pixels at or above
  clip level 1 of (peak − baseline), with a 3-px border excluded. BeamScope's primary centroid and D4σ
  follow ISO 11146 instead (iterative integration area), so its centroid differs from DataRay's by
  ~0.1–0.2 px on a saturated beam.
- `cargo test` runs the analysis, M², `.wcf` round-trip and DataRay-file tests.
  `cargo test -- --ignored hw_` runs a live-camera test.

## Known limitations

- Only the BladeCam2 is supported (HR confirmed on hardware; the XHR / MT9T001 path follows the decompiled code
  but is untested). Other DataRay families (WinCamD-IR/LCM/GCM, TaperCam) use different transports.
- DataRay's firmware-update commands were deliberately left out (they could brick the camera if misused).
- The `.wcf` writer fills in the frame header fields DataRay needs to open and display a file. It does not
  reproduce every one of DataRay's stored result fields.
- Frame-header fields `+0xD8`/`+0xE0` held 13.5 % clip widths in the reference files but are 0 or
  different in the 8.0E70 captures. Their exact meaning in 8.0E70 isn't pinned down yet.
