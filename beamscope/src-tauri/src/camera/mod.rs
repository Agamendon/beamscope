pub mod bladecam2;
pub mod ftd3xx;
pub mod sim;

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// One captured image. Pixel values are 16-bit full scale (10-bit sensors are shifted left
/// by 6, the same convention DataRay uses, so `.wcf` files round-trip).
#[derive(Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixel_um: (f64, f64),
    pub data: Arc<Vec<u16>>,
    pub exposure_ms: f64,
    pub gain: f64,
    /// Milliseconds since the Unix epoch.
    pub timestamp_ms: f64,
    pub index: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub model: String,
    pub serial: String,
    pub sensor: String,
    pub sensor_width: usize,
    pub sensor_height: usize,
    pub pixel_um: f64,
    pub firmware: String,
    pub link: String,
    pub exposure_min_ms: f64,
    pub exposure_max_ms: f64,
    pub gain_steps: u32,
    pub black_level_max: u32,
    pub simulated: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Binning {
    Full,
    Bin2,
    Bin4,
}

impl Binning {
    pub fn factor(self) -> usize {
        match self {
            Binning::Full => 1,
            Binning::Bin2 => 2,
            Binning::Bin4 => 4,
        }
    }
}

/// Region of interest in full-sensor pixel coordinates.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Roi {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

pub trait Camera: Send {
    fn info(&self) -> CameraInfo;
    /// Apply ROI + binning. Takes effect on the next `grab`.
    fn set_readout(&mut self, roi: Roi, binning: Binning) -> Result<()>;
    /// Returns the exposure actually achieved by the sensor timing.
    fn set_exposure(&mut self, ms: f64) -> Result<f64>;
    /// `step` is 1-based, up to `info().gain_steps`. Returns the nominal gain multiplier.
    fn set_gain(&mut self, step: u32) -> Result<f64>;
    fn set_black_level(&mut self, level: u32) -> Result<()>;
    fn grab(&mut self) -> Result<Frame>;
    fn stop(&mut self);
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Find and open the first available camera. Falls back to the simulator when asked.
pub fn open(simulate: bool) -> Result<Box<dyn Camera>> {
    if simulate {
        return Ok(Box::new(sim::SimCamera::new()));
    }
    Ok(Box::new(bladecam2::BladeCam2::open()?))
}

/// How the camera is mounted relative to the lab. Applied to every frame before analysis, so
/// the image, X/Y profiles and all X/Y metrics follow the lab axes. Mirroring is applied first
/// (in sensor coordinates), then the clockwise rotation.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Orientation {
    /// Clockwise rotation of the displayed image: 0, 90, 180 or 270 degrees.
    pub rotate: u16,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Orientation {
    pub fn is_identity(&self) -> bool {
        self.rotate % 360 == 0 && !self.flip_h && !self.flip_v
    }

    /// Re-map `f` into the lab orientation.
    pub fn apply(&self, f: &Frame) -> Frame {
        if self.is_identity() {
            return f.clone();
        }
        let (w, h) = (f.width, f.height);
        let rot = (self.rotate / 90) % 4;
        let (ow, oh, pitch) = if rot % 2 == 1 { (h, w, (f.pixel_um.1, f.pixel_um.0)) } else { (w, h, f.pixel_um) };
        let src = &f.data;
        let mut out = Vec::with_capacity(ow * oh);
        for oy in 0..oh {
            for ox in 0..ow {
                // Undo the clockwise rotation: output (ox, oy) comes from mirrored-image (x, y).
                let (x, y) = match rot {
                    0 => (ox, oy),
                    1 => (oy, h - 1 - ox),
                    2 => (w - 1 - ox, h - 1 - oy),
                    _ => (w - 1 - oy, ox),
                };
                // Undo the mirror.
                let x = if self.flip_h { w - 1 - x } else { x };
                let y = if self.flip_v { h - 1 - y } else { y };
                out.push(src[y * w + x]);
            }
        }
        Frame { width: ow, height: oh, pixel_um: pitch, data: Arc::new(out), ..f.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize) -> Frame {
        // Pixel value encodes its sensor position: 100·y + x.
        let d = (0..h).flat_map(|y| (0..w).map(move |x| (100 * y + x) as u16)).collect();
        Frame { width: w, height: h, pixel_um: (5.0, 7.0), data: Arc::new(d), exposure_ms: 1.0, gain: 1.0, timestamp_ms: 0.0, index: 3 }
    }

    fn at(f: &Frame, x: usize, y: usize) -> u16 {
        f.data[y * f.width + x]
    }

    #[test]
    fn rotations_and_mirrors() {
        let f = frame(4, 3); // sensor top-left = 0, top-right = 3, bottom-left = 200
        let o = |rotate, flip_h, flip_v| Orientation { rotate, flip_h, flip_v }.apply(&f);

        let r90 = o(90, false, false);
        assert_eq!((r90.width, r90.height, r90.pixel_um), (3, 4, (7.0, 5.0)));
        // Clockwise: the sensor's bottom-left corner ends up top-left, top-left ends up top-right.
        assert_eq!(at(&r90, 0, 0), 200);
        assert_eq!(at(&r90, 2, 0), 0);
        assert_eq!(at(&r90, 2, 3), 3);

        let r180 = o(180, false, false);
        assert_eq!(at(&r180, 0, 0), 203);
        assert_eq!(at(&r180, 3, 2), 0);

        let r270 = o(270, false, false);
        assert_eq!((r270.width, r270.height), (3, 4));
        assert_eq!(at(&r270, 0, 0), 3);
        assert_eq!(at(&r270, 0, 3), 0);

        assert_eq!(at(&o(0, true, false), 0, 0), 3);
        assert_eq!(at(&o(0, false, true), 0, 0), 200);
        // Mirror first, then rotate.
        assert_eq!(at(&o(90, true, false), 0, 0), 203);

        // Four quarter turns are the identity; metadata is preserved.
        let mut g = f.clone();
        for _ in 0..4 {
            g = Orientation { rotate: 90, ..Default::default() }.apply(&g);
        }
        assert_eq!(g.data, f.data);
        assert_eq!((g.index, g.pixel_um), (3, (5.0, 7.0)));
    }
}
