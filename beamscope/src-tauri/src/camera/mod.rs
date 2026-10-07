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
