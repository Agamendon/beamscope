//! DataRay BladeCam2 driver (FT60x + FPGA + Aptina MT9M001 / MT9T001).
//!
//! Protocol recovered from DataRayOcx.ocx; see `docs/bladecam2-protocol.md` for the source
//! function behind every step here.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use super::ftd3xx::{DeviceInfo, Ftd3xx, Handle, FT_FLAGS_SUPERSPEED};
use super::{now_ms, Binning, Camera, CameraInfo, Frame, Roi};

const DEVICE_ID: u32 = 0x1632_2101;
const PIPE_CMD_OUT: u8 = 0x02;
const PIPE_AUX_OUT: u8 = 0x03;
const PIPE_REPLY_IN: u8 = 0x82;
const PIPE_VIDEO_IN: u8 = 0x83;

/// Pixel clock period in ms (48 MHz).
const T_PIX_MS: f64 = 2.083_333_333_333_333_6e-5;

// FPGA commands (byte 2 of a register transaction).
mod cmd {
    pub const FW_VERSION: u8 = 0x01;
    pub const SCRATCH_WRITE: u8 = 0x02;
    pub const SCRATCH_READ: u8 = 0x03;
    pub const STREAM: u8 = 0x04;
    pub const SENSOR_WRITE: u8 = 0x06;
    pub const SENSOR_READ: u8 = 0x07;
    pub const CALIB_READ: u8 = 0x09;
    pub const FRAME_REQUEST: u8 = 0x0A;
    pub const APPLY_SENSOR: u8 = 0x0B;
    pub const SENSOR_RESET: u8 = 0x0C;
    pub const OUT_WIDTH: u8 = 0x0D;
    pub const OUT_HEIGHT: u8 = 0x0E;
    pub const SPECIAL_FRAME: u8 = 0x0F;
    pub const SENSOR_STANDBY: u8 = 0x11;
    pub const PRE_RECONFIGURE: u8 = 0x95;
}

const SCRATCH_PATTERN: u16 = 0x4243;

/// Returned (as an error) for frames that should be silently dropped.
#[derive(Debug)]
pub struct SkipFrame;

impl std::fmt::Display for SkipFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("frame dropped (no start marker)")
    }
}

impl std::error::Error for SkipFrame {}

/// Global-gain register codes, indexed by gain step - 1 (tables at 0x180394000 / 0x180394060).
const GAIN_M001: [u16; 44] = [
    0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a,
    0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67,
];
const GAIN_T001: [u16; 44] = [
    0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a,
    0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x160, 0x260, 0x360, 0x460, 0x560, 0x660, 0x760,
];

/// Aptina global gain code → multiplier: bits 0..5 in 1/8 steps, bit 6 = 2× analog stage,
/// bits 8..14 = digital gain (1 + d/8).
fn gain_multiplier(code: u16) -> f64 {
    let analog = (code & 0x3f) as f64 / 8.0 * if code & 0x40 != 0 { 2.0 } else { 1.0 };
    let digital = 1.0 + ((code >> 8) & 0x7f) as f64 / 8.0;
    analog * digital
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sensor {
    /// 1280×1024, 5.2 µm — BladeCam2-HR.
    Mt9m001,
    /// 2048×1536, 3.2 µm — BladeCam2-XHR.
    Mt9t001,
}

impl Sensor {
    fn full_size(self) -> (usize, usize) {
        match self {
            Sensor::Mt9m001 => (1280, 1024),
            Sensor::Mt9t001 => (2048, 1536),
        }
    }
    fn pixel_um(self) -> f64 {
        match self {
            Sensor::Mt9m001 => 5.2,
            Sensor::Mt9t001 => 3.2,
        }
    }
    fn default_hblank(self) -> u16 {
        match self {
            Sensor::Mt9m001 => 0x13,
            Sensor::Mt9t001 => 0x15,
        }
    }
    fn black_level_max(self) -> u32 {
        match self {
            Sensor::Mt9m001 => 10,
            Sensor::Mt9t001 => 0x400,
        }
    }
}

pub struct BladeCam2 {
    ft: Ftd3xx,
    h: Handle,
    dev: DeviceInfo,
    sensor: Sensor,
    firmware: u16,
    calibration: Vec<u8>,
    /// Shadow of sensor registers, so unchanged values are not rewritten (FUN_18003db90).
    regs: HashMap<u8, u16>,
    roi: Roi,
    binning: Binning,
    exposure_ms: f64,
    shutter_width: u32,
    hblank: u16,
    gain_step: u32,
    black_level: u32,
    streaming: bool,
    frame_index: u64,
    buf: Vec<u8>,
}

// The FTD3XX handle is only ever used from behind the camera mutex.
unsafe impl Send for BladeCam2 {}

impl BladeCam2 {
    pub fn open() -> Result<Self> {
        let ft = Ftd3xx::load()?;
        let devs = ft.devices()?;
        let dev = devs
            .into_iter()
            .find(|d| d.id == DEVICE_ID)
            .ok_or_else(|| anyhow!("no BladeCam2 found (is it plugged in, and is DataRay closed?)"))?;
        let h = ft.open_by_index(dev.index).context("opening BladeCam2")?;
        let mut cam = Self {
            ft,
            h,
            dev,
            sensor: Sensor::Mt9m001,
            firmware: 0,
            calibration: Vec::new(),
            regs: HashMap::new(),
            roi: Roi { x: 0, y: 0, w: 1280, h: 1024 },
            binning: Binning::Full,
            exposure_ms: 1.0,
            shutter_width: 100,
            hblank: 0x13,
            gain_step: 1,
            black_level: 0,
            streaming: false,
            frame_index: 0,
            buf: Vec::new(),
        };
        if let Err(e) = cam.handshake() {
            cam.ft.close(cam.h);
            return Err(e);
        }
        Ok(cam)
    }

    /// FUN_1800394a0 + tail of the CBladeCam constructor.
    fn handshake(&mut self) -> Result<()> {
        // A previous session may have died mid-stream; flush whatever is still queued.
        self.recover();
        for pipe in [PIPE_REPLY_IN, PIPE_CMD_OUT, PIPE_AUX_OUT, PIPE_VIDEO_IN] {
            self.ft.set_pipe_timeout(self.h, pipe, 500)?;
        }
        self.ft.enable_gpio(self.h, 3, 3)?;
        self.ft.write_gpio(self.h, 3, 0)?;

        // First contact, then have the FPGA reload its application image, then test again.
        let _ = self.reg(cmd::SCRATCH_WRITE, SCRATCH_PATTERN, 0);
        let _ = self.reg(cmd::SCRATCH_READ, 0, 0);
        let _ = self.reg(cmd::PRE_RECONFIGURE, 0, 0);
        self.reconfigure_fpga()?;
        self.reg(cmd::SCRATCH_WRITE, SCRATCH_PATTERN, 0)?;
        let echo = self.reg(cmd::SCRATCH_READ, 0, 0)?;
        if echo != SCRATCH_PATTERN {
            bail!("BladeCam2 FPGA self-test failed: expected {SCRATCH_PATTERN:#x}, got {echo:#x}");
        }
        self.firmware = self.reg(cmd::FW_VERSION, 0, 0)?;

        self.reg(cmd::SENSOR_STANDBY, 2, 0)?;
        self.reg(cmd::SENSOR_RESET, 1, 0)?;
        sleep(Duration::from_millis(100));
        self.reg(cmd::SENSOR_RESET, 0, 0)?;
        sleep(Duration::from_millis(100));
        self.reg(cmd::SENSOR_STANDBY, 1, 0)?;

        let chip = self.sensor_read(0x00)?;
        self.sensor = if chip == 0x1621 { Sensor::Mt9t001 } else { Sensor::Mt9m001 };
        let (w, h) = self.sensor.full_size();
        self.roi = Roi { x: 0, y: 0, w, h };
        self.hblank = self.sensor.default_hblank();

        self.calibration = (0..56u16)
            .map(|i| self.reg(cmd::CALIB_READ, i, 0).map(|v| v as u8))
            .collect::<Result<_>>()
            .unwrap_or_default();
        Ok(())
    }

    /// 8-byte register transaction on pipe 0x02 with a 4-byte reply on 0x82 (FUN_18003be30).
    fn reg(&mut self, c: u8, arg1: u16, arg2: u16) -> Result<u16> {
        let [a1l, a1h] = arg1.to_le_bytes();
        let [a2l, a2h] = arg2.to_le_bytes();
        let pkt = [0x02, 0x03, c, a1l, a1h, a2l, a2h, 0x00];
        let res = (|| -> Result<u16> {
            if self.ft.write_pipe(self.h, PIPE_CMD_OUT, &pkt)? != 8 {
                bail!("short command write");
            }
            let mut reply = [0u8; 4];
            if self.ft.read_pipe(self.h, PIPE_REPLY_IN, &mut reply)? != 4 {
                bail!("short command reply");
            }
            Ok(u16::from_le_bytes([reply[2], reply[3]]))
        })();
        if res.is_err() {
            self.recover();
        }
        res.with_context(|| format!("BladeCam2 command {c:#04x}"))
    }

    /// `02 01 93 …`: FPGA reloads its application image (FUN_18003aac0).
    fn reconfigure_fpga(&mut self) -> Result<()> {
        let pkt = [0x02, 0x01, 0x93, 0, 0, 0, 0, 0];
        let n = self.ft.write_pipe(self.h, PIPE_CMD_OUT, &pkt);
        if !matches!(n, Ok(8)) {
            self.recover();
            bail!("FPGA reconfigure command failed");
        }
        sleep(Duration::from_millis(500));
        Ok(())
    }

    /// GPIO pulse + flush of both IN pipes (FUN_18003abc0).
    fn recover(&mut self) {
        let _ = self.ft.write_gpio(self.h, 3, 3);
        sleep(Duration::from_millis(10));
        let _ = self.ft.write_gpio(self.h, 3, 0);
        for pipe in [PIPE_VIDEO_IN, PIPE_REPLY_IN] {
            self.ft.abort_pipe(self.h, pipe);
            self.ft.clear_stream_pipe(self.h, pipe);
        }
    }

    fn sensor_read(&mut self, r: u8) -> Result<u16> {
        let v = self.reg(cmd::SENSOR_READ, r as u16, 0)?;
        self.regs.insert(r, v);
        Ok(v)
    }

    fn sensor_write(&mut self, r: u8, v: u16) -> Result<()> {
        if self.regs.get(&r) != Some(&v) {
            self.reg(cmd::SENSOR_WRITE, r as u16, v)?;
            self.regs.insert(r, v);
        }
        Ok(())
    }

    fn out_size(&self) -> (usize, usize) {
        let f = self.binning.factor();
        (self.roi.w / f, self.roi.h / f)
    }

    /// Window + readout mode registers (FUN_18003acd0 / FUN_18003ae10), then cmd 0x0B.
    fn write_window(&mut self) -> Result<()> {
        let Roi { x, y, w, h } = self.roi;
        let (x, y, w, h) = (x as u16, y as u16, w as u16, h as u16);
        match self.sensor {
            Sensor::Mt9m001 => {
                self.sensor_write(0x1e, 0x8000)?;
                self.sensor_write(0x01, y + 0x0c)?;
                self.sensor_write(0x02, x + 0x14)?;
                self.sensor_write(0x03, (h - 1) | 1)?;
                self.sensor_write(0x04, (w - 1) | 1)?;
                self.sensor_write(0x05, self.hblank)?;
                self.sensor_write(0x62, 0x498)?;
                let (r1e, r20) = match self.binning {
                    Binning::Bin2 => (0x8000, 0x111d),
                    Binning::Full => (0x8000, 0x1105),
                    Binning::Bin4 => (0x800c, 0x1105),
                };
                self.sensor_write(0x1e, r1e)?;
                self.sensor_write(0x20, r20)?;
            }
            Sensor::Mt9t001 => {
                self.sensor_write(0x1e, 0xc040)?;
                self.sensor_write(0x01, y + 0x14)?;
                self.sensor_write(0x02, x + 0x20)?;
                self.sensor_write(0x03, (h - 1) | 1)?;
                self.sensor_write(0x04, (w - 1) | 1)?;
                let mode = match self.binning {
                    Binning::Bin2 => 1,
                    Binning::Full => 0,
                    Binning::Bin4 => 3,
                };
                self.sensor_write(0x23, mode)?;
                self.sensor_write(0x22, mode)?;
                self.sensor_write(0x62, 0)?;
            }
        }
        self.reg(cmd::APPLY_SENSOR, 0, 0)?;
        Ok(())
    }

    /// Exposure time → shutter width / horizontal blank (FUN_18003b0e0).
    fn compute_exposure(&self, ms: f64) -> (u32, u16, f64) {
        let ticks = ms / T_PIX_MS;
        match self.sensor {
            Sensor::Mt9m001 => {
                let w = self.roi.w as f64;
                let row = w + 0xf5 as f64;
                let sw = (ticks / row).floor().clamp(1.0, 0x3fff as f64) as u32;
                let left = ticks as i64 - (sw as f64 * row) as i64;
                let mut hb = 0x13i64;
                if left >= sw as i64 {
                    hb = left / sw as i64 + 0x13;
                }
                let hb = hb.clamp(0x13, 0x7ff) as u16;
                let actual = (sw as f64 * (hb as f64 + 0xe2 as f64 + w) - 180.0) * T_PIX_MS;
                (sw, hb, actual)
            }
            Sensor::Mt9t001 => {
                let w_out = self.out_size().0 as f64;
                let hb = 0x15u16;
                let row = (w_out + 0x186 as f64).max(0x287 as f64);
                let sw = (ticks / row).floor().clamp(1.0, u32::MAX as f64 / 2.0) as u32;
                let actual = (row * sw as f64 - 199.0) * T_PIX_MS;
                (sw, hb, actual)
            }
        }
    }

    fn write_exposure(&mut self) -> Result<()> {
        let (sw, hb, actual) = self.compute_exposure(self.exposure_ms);
        self.shutter_width = sw;
        self.hblank = hb;
        match self.sensor {
            Sensor::Mt9m001 => {
                self.sensor_write(0x09, (sw & 0x3fff) as u16)?;
            }
            Sensor::Mt9t001 => {
                self.sensor_write(0x09, (sw & 0xffff) as u16)?;
                self.sensor_write(0x08, (sw >> 16) as u16)?;
            }
        }
        self.sensor_write(0x05, hb & 0x7ff)?;
        self.exposure_ms = actual.max(T_PIX_MS);
        let timeout = (self.exposure_ms * 4.0).max(500.0) as u32;
        for pipe in [PIPE_REPLY_IN, PIPE_CMD_OUT, PIPE_AUX_OUT, PIPE_VIDEO_IN] {
            self.ft.set_pipe_timeout(self.h, pipe, timeout)?;
        }
        Ok(())
    }

    fn gain_code(&self) -> u16 {
        let table = match self.sensor {
            Sensor::Mt9m001 => &GAIN_M001,
            Sensor::Mt9t001 => &GAIN_T001,
        };
        table[(self.gain_step.clamp(1, 44) - 1) as usize]
    }

    fn write_gain(&mut self) -> Result<()> {
        let code = self.gain_code();
        self.sensor_write(0x35, code)
    }

    /// FUN_18003b8c0: write the black-level target and read it back until it sticks.
    fn write_black_level(&mut self) -> Result<()> {
        let v = self.black_level as u16;
        for _ in 0..10 {
            let back = match self.sensor {
                Sensor::Mt9m001 => {
                    self.sensor_write(0x5f, v | (v + 8) << 8 | 0x8000)?;
                    self.sensor_read(0x5f)? & 0x3f
                }
                Sensor::Mt9t001 => {
                    self.sensor_write(0x49, (v & 0x3ff) << 2)?;
                    self.sensor_read(0x49)? >> 2
                }
            };
            if back as u32 == self.black_level {
                break;
            }
            self.regs.remove(&(if self.sensor == Sensor::Mt9m001 { 0x5f } else { 0x49 }));
        }
        Ok(())
    }

    /// Full stream (re)start, mirroring the "re-init" branch of FUN_18003c100.
    fn arm(&mut self) -> Result<()> {
        self.recover();
        self.reg(cmd::STREAM, 0, 0)?;
        self.write_black_level()?;
        self.write_gain()?;
        self.write_window()?;
        self.write_exposure()?;
        self.reg(cmd::SPECIAL_FRAME, 0, 0)?;
        let (w, h) = self.out_size();
        self.reg(cmd::OUT_WIDTH, w as u16, 0)?;
        self.reg(cmd::OUT_HEIGHT, h as u16, 0)?;
        self.reg(cmd::STREAM, 1, 0)?;
        self.buf.resize(w * h * 2, 0);
        self.streaming = true;
        Ok(())
    }
}

impl Camera for BladeCam2 {
    fn info(&self) -> CameraInfo {
        let (w, h) = self.sensor.full_size();
        let (model, sensor) = match self.sensor {
            Sensor::Mt9m001 => ("BladeCam2-HR", "Aptina MT9M001"),
            Sensor::Mt9t001 => ("BladeCam2-XHR", "Aptina MT9T001"),
        };
        CameraInfo {
            model: model.into(),
            serial: self.dev.serial.clone(),
            sensor: sensor.into(),
            sensor_width: w,
            sensor_height: h,
            pixel_um: self.sensor.pixel_um(),
            firmware: format!("{} (raw {:#x})", self.firmware as i32 - 0x66, self.firmware),
            link: if self.dev.flags & FT_FLAGS_SUPERSPEED != 0 { "USB 3".into() } else { "USB 2".into() },
            exposure_min_ms: 0.03,
            exposure_max_ms: match self.sensor {
                Sensor::Mt9m001 => 1000.0,
                Sensor::Mt9t001 => 2000.0,
            },
            gain_steps: 44,
            black_level_max: self.sensor.black_level_max(),
            simulated: false,
        }
    }

    fn set_readout(&mut self, roi: Roi, binning: Binning) -> Result<()> {
        let (fw, fh) = self.sensor.full_size();
        let f = binning.factor();
        // Keep the window aligned to the binning factor and inside the sensor.
        let w = (roi.w.clamp(16, fw) / (2 * f)) * 2 * f;
        let h = (roi.h.clamp(16, fh) / (2 * f)) * 2 * f;
        let x = roi.x.min(fw - w) & !1;
        let y = roi.y.min(fh - h) & !1;
        self.roi = Roi { x, y, w, h };
        self.binning = binning;
        self.streaming = false;
        Ok(())
    }

    fn set_exposure(&mut self, ms: f64) -> Result<f64> {
        self.exposure_ms = ms;
        if self.streaming {
            self.write_exposure()?;
        } else {
            self.exposure_ms = self.compute_exposure(ms).2;
        }
        Ok(self.exposure_ms)
    }

    fn set_gain(&mut self, step: u32) -> Result<f64> {
        self.gain_step = step.clamp(1, 44);
        if self.streaming {
            self.write_gain()?;
        }
        Ok(gain_multiplier(self.gain_code()) / gain_multiplier(0x0c))
    }

    fn set_black_level(&mut self, level: u32) -> Result<()> {
        self.black_level = level.min(self.sensor.black_level_max());
        if self.streaming {
            self.write_black_level()?;
        }
        Ok(())
    }

    fn grab(&mut self) -> Result<Frame> {
        if !self.streaming {
            self.arm()?;
            // The first frame after arming is stale (seen on hardware: ~500 ms, garbage data).
            let _ = self.reg(cmd::FRAME_REQUEST, 1, 0);
            let _ = self.ft.read_pipe(self.h, PIPE_VIDEO_IN, &mut self.buf);
        }
        let (w, h) = self.out_size();
        let res = (|| -> Result<()> {
            self.reg(cmd::FRAME_REQUEST, 1, 0)?;
            let n = self.ft.read_pipe(self.h, PIPE_VIDEO_IN, &mut self.buf)? as usize;
            if n != self.buf.len() {
                bail!("short frame: {n} of {} bytes", self.buf.len());
            }
            Ok(())
        })();
        if let Err(e) = res {
            self.streaming = false; // USB-level failure: re-arm on the next grab
            return Err(e);
        }
        // Bit 15 of the first word marks the start of a frame. The first frame after arming
        // can be stale and lacks it; drop it without re-arming.
        if self.buf[1] & 0x80 == 0 {
            bail!(SkipFrame);
        }
        let data: Vec<u16> = self
            .buf
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) << 6)
            .collect();
        self.frame_index += 1;
        let pitch = self.sensor.pixel_um() * self.binning.factor() as f64;
        Ok(Frame {
            width: w,
            height: h,
            pixel_um: (pitch, pitch),
            data: Arc::new(data),
            exposure_ms: self.exposure_ms,
            gain: gain_multiplier(self.gain_code()) / gain_multiplier(0x0c),
            timestamp_ms: now_ms(),
            index: self.frame_index,
        })
    }

    fn stop(&mut self) {
        if self.streaming {
            let _ = self.reg(cmd::STREAM, 0, 0);
            self.streaming = false;
        }
    }
}

impl Drop for BladeCam2 {
    fn drop(&mut self) {
        self.stop();
        self.ft.close(self.h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_codes() {
        assert_eq!(gain_multiplier(0x08), 1.0);
        assert_eq!(gain_multiplier(0x0c), 1.5);
        assert_eq!(gain_multiplier(0x60), 8.0);
        assert_eq!(gain_multiplier(0x160), 9.0);
    }
}

#[cfg(test)]
mod hw {
    use super::*;

    /// Needs a BladeCam2 plugged in and DataRay closed: `cargo test --release -- --ignored hw_`
    #[test]
    #[ignore]
    fn hw_grab() {
        let mut cam = BladeCam2::open().expect("open");
        println!("{:#?}", cam.info());
        println!("calibration: {:02x?}", cam.calibration);
        for binning in [Binning::Bin2, Binning::Full] {
            let (w, h) = cam.sensor.full_size();
            cam.set_readout(Roi { x: 0, y: 0, w, h }, binning).unwrap();
            let mut exp = cam.set_exposure(1.0).unwrap();
            for i in 0..12 {
                let t0 = std::time::Instant::now();
                match cam.grab() {
                    Ok(f) => {
                        let m = crate::analysis::analyze(&f, &Default::default());
                        println!(
                            "{binning:?} #{i}: {}x{} {:.1} ms  exp {:.4} ms  peak {:.0}  centroid ({:.1},{:.1})  D4σ {:.1} × {:.1} µm",
                            f.width, f.height, t0.elapsed().as_secs_f64() * 1e3, f.exposure_ms, m.peak,
                            m.centroid_px.0, m.centroid_px.1, m.d4s_x_um, m.d4s_y_um
                        );
                        let k = ((57343.125 / m.peak_smoothed.max(1.0) + 1.0) * 0.5).clamp(0.15, 7.0);
                        exp = cam.set_exposure(exp * if m.peak < 62258.25 { k.sqrt() } else { k }).unwrap();
                    }
                    Err(e) => println!("{binning:?} #{i}: {e:#}"),
                }
            }
        }
    }
}
