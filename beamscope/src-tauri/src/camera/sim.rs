//! Simulated BladeCam2-HR: an elliptical Gaussian beam with shot/read noise and slow
//! pointing drift. Brightness follows exposure × gain so auto-exposure can be exercised.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::{now_ms, Binning, Camera, CameraInfo, Frame, Roi};

pub struct SimCamera {
    roi: Roi,
    binning: Binning,
    exposure_ms: f64,
    gain: f64,
    black: u32,
    index: u64,
    t0: Instant,
    rng: u64,
}

const W: usize = 1280;
const H: usize = 1024;
const PITCH: f64 = 5.2;

impl SimCamera {
    pub fn new() -> Self {
        Self {
            roi: Roi { x: 0, y: 0, w: W, h: H },
            binning: Binning::Full,
            exposure_ms: 1.0,
            gain: 1.0,
            black: 0,
            index: 0,
            t0: Instant::now(),
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn rand(&mut self) -> f64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn gauss(&mut self) -> f64 {
        let u = self.rand().max(1e-12);
        let v = self.rand();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

impl Camera for SimCamera {
    fn info(&self) -> CameraInfo {
        CameraInfo {
            model: "Simulated BladeCam2-HR".into(),
            serial: "SIM-0001".into(),
            sensor: "virtual MT9M001".into(),
            sensor_width: W,
            sensor_height: H,
            pixel_um: PITCH,
            firmware: "sim".into(),
            link: "none".into(),
            exposure_min_ms: 0.05,
            exposure_max_ms: 1000.0,
            gain_steps: 44,
            black_level_max: 10,
            simulated: true,
        }
    }

    fn set_readout(&mut self, roi: Roi, binning: Binning) -> Result<()> {
        let f = binning.factor();
        let w = (roi.w.clamp(16, W) / (2 * f)) * 2 * f;
        let h = (roi.h.clamp(16, H) / (2 * f)) * 2 * f;
        self.roi = Roi { x: roi.x.min(W - w) & !1, y: roi.y.min(H - h) & !1, w, h };
        self.binning = binning;
        Ok(())
    }

    fn set_exposure(&mut self, ms: f64) -> Result<f64> {
        self.exposure_ms = ms.clamp(0.05, 1000.0);
        Ok(self.exposure_ms)
    }

    fn set_gain(&mut self, step: u32) -> Result<f64> {
        self.gain = 1.0 + (step.clamp(1, 44) - 1) as f64 * 0.125;
        Ok(self.gain)
    }

    fn set_black_level(&mut self, level: u32) -> Result<()> {
        self.black = level;
        Ok(())
    }

    fn grab(&mut self) -> Result<Frame> {
        let f = self.binning.factor();
        let (ow, oh) = (self.roi.w / f, self.roi.h / f);
        // Readout time roughly like the real HR camera (≈30 ms binned, ≈110 ms full).
        let readout = 8.0 + (ow * oh) as f64 * 7.5e-5;
        std::thread::sleep(Duration::from_secs_f64((self.exposure_ms + readout) / 1000.0));

        let t = self.t0.elapsed().as_secs_f64();
        let cx = 640.0 + 25.0 * (t * 0.31).sin();
        let cy = 512.0 + 18.0 * (t * 0.23).cos();
        let (wx, wy) = (110.0, 80.0); // 1/e² radii in full-sensor pixels
        let (s, c) = (0.35f64.sin(), 0.35f64.cos());
        // Photons per ms at gain 1 at the beam center.
        let amp = 1800.0 * self.exposure_ms * self.gain;
        let dark = 12.0 + 4.0 * self.black as f64;
        let mut data = vec![0u16; ow * oh];
        for oy in 0..oh {
            for ox in 0..ow {
                let x = (self.roi.x + ox * f) as f64 + (f as f64 - 1.0) / 2.0;
                let y = (self.roi.y + oy * f) as f64 + (f as f64 - 1.0) / 2.0;
                let (dx, dy) = (x - cx, y - cy);
                let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
                let sig = amp * (-2.0 * (u * u / (wx * wx) + v * v / (wy * wy))).exp();
                let noise = self.gauss() * (1.5 + sig.sqrt() * 0.3);
                let counts = (dark + sig + noise).clamp(0.0, 1023.0) as u16;
                data[oy * ow + ox] = counts << 6;
            }
        }
        self.index += 1;
        Ok(Frame {
            width: ow,
            height: oh,
            pixel_um: (PITCH * f as f64, PITCH * f as f64),
            data: Arc::new(data),
            exposure_ms: self.exposure_ms,
            gain: self.gain,
            timestamp_ms: now_ms(),
            index: self.index,
        })
    }

    fn stop(&mut self) {}
}
