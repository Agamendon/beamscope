//! Acquisition engine: owns the camera, runs the capture thread, applies averaging /
//! background subtraction / auto-exposure, analyzes frames and keeps the log.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::analysis::{self, AnalysisSettings, Metrics};
use crate::camera::{self, bladecam2::SkipFrame, Binning, Camera, CameraInfo, Frame, Roi};
use crate::wcf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub exposure_ms: f64,
    pub auto_exposure: bool,
    pub gain_step: u32,
    pub black_level: u32,
    pub auto_black: bool,
    pub binning: Binning,
    pub roi: Option<Roi>,
    /// Boxcar average over this many frames (1 = off).
    pub average: usize,
    pub subtract_background: bool,
    pub analysis: AnalysisSettings,
    pub wavelength_nm: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exposure_ms: 1.0,
            auto_exposure: true,
            gain_step: 1,
            black_level: 0,
            auto_black: true,
            binning: Binning::Bin2,
            roi: None,
            average: 1,
            subtract_background: false,
            analysis: AnalysisSettings::default(),
            wavelength_nm: 1064.0,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub running: bool,
    pub source: String,
    pub camera: Option<CameraInfo>,
    pub fps: f64,
    pub frames: u64,
    pub dropped: u64,
    pub last_error: Option<String>,
    pub has_background: bool,
    pub logging: bool,
    pub log_rows: usize,
    pub config: Config,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRow {
    pub t_ms: f64,
    pub frame: u64,
    pub exposure_ms: f64,
    pub peak: f64,
    pub total: f64,
    pub cx_um: f64,
    pub cy_um: f64,
    pub d4s_x_um: f64,
    pub d4s_y_um: f64,
    pub d4s_major_um: f64,
    pub d4s_minor_um: f64,
    pub orientation_deg: f64,
    pub ellipticity: f64,
    pub clip1_x_um: f64,
    pub clip1_y_um: f64,
    pub clip2_x_um: f64,
    pub clip2_y_um: f64,
    pub gauss_x_um: f64,
    pub gauss_y_um: f64,
}

impl LogRow {
    fn from(f: &Frame, m: &Metrics) -> Self {
        let c = |i: usize| m.clip.get(i).cloned().unwrap_or_default();
        Self {
            t_ms: f.timestamp_ms,
            frame: f.index,
            exposure_ms: f.exposure_ms,
            peak: m.peak,
            total: m.total,
            cx_um: m.centroid_um.0,
            cy_um: m.centroid_um.1,
            d4s_x_um: m.d4s_x_um,
            d4s_y_um: m.d4s_y_um,
            d4s_major_um: m.d4s_major_um,
            d4s_minor_um: m.d4s_minor_um,
            orientation_deg: m.orientation_deg,
            ellipticity: m.ellipticity,
            clip1_x_um: c(0).x_um,
            clip1_y_um: c(0).y_um,
            clip2_x_um: c(1).x_um,
            clip2_y_um: c(1).y_um,
            gauss_x_um: m.gauss_x.diameter_um,
            gauss_y_um: m.gauss_y.diameter_um,
        }
    }

    pub const CSV_HEADER: &'static str = "time_ms,frame,exposure_ms,peak,total,centroid_x_um,centroid_y_um,d4s_x_um,d4s_y_um,d4s_major_um,d4s_minor_um,orientation_deg,ellipticity,clip1_x_um,clip1_y_um,clip2_x_um,clip2_y_um,gauss_x_um,gauss_y_um";

    pub fn csv(&self) -> String {
        format!(
            "{:.1},{},{:.4},{:.0},{:.0},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.2},{:.4},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
            self.t_ms, self.frame, self.exposure_ms, self.peak, self.total, self.cx_um, self.cy_um,
            self.d4s_x_um, self.d4s_y_um, self.d4s_major_um, self.d4s_minor_um, self.orientation_deg,
            self.ellipticity, self.clip1_x_um, self.clip1_y_um, self.clip2_x_um, self.clip2_y_um,
            self.gauss_x_um, self.gauss_y_um
        )
    }
}

struct Latest {
    raw: Frame,
    processed: Frame,
    metrics: Metrics,
}

#[derive(Default)]
struct Log {
    enabled: bool,
    interval_ms: f64,
    last_ms: f64,
    rows: Vec<LogRow>,
}

struct Shared {
    cam: Mutex<Option<Box<dyn Camera>>>,
    info: Mutex<Option<CameraInfo>>,
    cfg: Mutex<Config>,
    running: AtomicBool,
    latest: Mutex<Option<Arc<Latest>>>,
    source: Mutex<String>,
    background: Mutex<Option<Vec<f32>>>,
    bg_request: Mutex<Option<(usize, Vec<f64>, usize)>>,
    stats: Mutex<(f64, u64, u64, Option<String>)>,
    log: Mutex<Log>,
}

pub struct Engine {
    s: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            s: Arc::new(Shared {
                cam: Mutex::new(None),
                info: Mutex::new(None),
                cfg: Mutex::new(Config::default()),
                running: AtomicBool::new(false),
                latest: Mutex::new(None),
                source: Mutex::new("none".into()),
                background: Mutex::new(None),
                bg_request: Mutex::new(None),
                stats: Mutex::new((0.0, 0, 0, None)),
                log: Mutex::new(Log { interval_ms: 0.0, ..Default::default() }),
            }),
            thread: Mutex::new(None),
        }
    }
}

fn apply_camera_config(cam: &mut dyn Camera, info: &CameraInfo, cfg: &mut Config) -> Result<()> {
    let roi = cfg.roi.unwrap_or(Roi { x: 0, y: 0, w: info.sensor_width, h: info.sensor_height });
    cam.set_readout(roi, cfg.binning)?;
    cfg.exposure_ms = cam.set_exposure(cfg.exposure_ms.clamp(info.exposure_min_ms, info.exposure_max_ms))?;
    cam.set_gain(cfg.gain_step)?;
    cam.set_black_level(cfg.black_level)?;
    Ok(())
}

impl Engine {
    pub fn connect(&self, simulate: bool) -> Result<CameraInfo> {
        self.stop();
        *self.s.cam.lock() = None;
        let mut cam = camera::open(simulate)?;
        let info = cam.info();
        {
            let mut cfg = self.s.cfg.lock();
            cfg.roi = None;
            apply_camera_config(cam.as_mut(), &info, &mut cfg)?;
        }
        *self.s.cam.lock() = Some(cam);
        *self.s.info.lock() = Some(info.clone());
        *self.s.background.lock() = None;
        Ok(info)
    }

    pub fn disconnect(&self) {
        self.stop();
        *self.s.cam.lock() = None;
        *self.s.info.lock() = None;
    }

    pub fn status(&self) -> Status {
        // One lock per statement: temporaries in a struct literal live until the end of the
        // whole expression, so locking the same mutex twice inside it deadlocks.
        let (fps, frames, dropped, err) = self.s.stats.lock().clone();
        let (logging, log_rows) = {
            let log = self.s.log.lock();
            (log.enabled, log.rows.len())
        };
        // `info` is set/cleared together with the camera and, unlike `cam`, is never held
        // across a frame read, so polling status can't stall behind the capture thread.
        let camera = self.s.info.lock().clone();
        let source = self.s.source.lock().clone();
        let has_background = self.s.background.lock().is_some();
        let config = self.s.cfg.lock().clone();
        Status {
            connected: camera.is_some(),
            running: self.s.running.load(Ordering::SeqCst),
            source,
            camera,
            fps,
            frames,
            dropped,
            last_error: err,
            has_background,
            logging,
            log_rows,
            config,
        }
    }

    pub fn set_config(&self, mut new: Config) -> Result<Config> {
        let old = self.s.cfg.lock().clone();
        if let (Some(cam), Some(info)) = (self.s.cam.lock().as_mut(), self.s.info.lock().as_ref()) {
            if new.roi != old.roi || new.binning != old.binning {
                let roi = new.roi.unwrap_or(Roi { x: 0, y: 0, w: info.sensor_width, h: info.sensor_height });
                cam.set_readout(roi, new.binning)?;
                *self.s.background.lock() = None;
            }
            if (new.exposure_ms - old.exposure_ms).abs() > 1e-9 {
                new.exposure_ms = cam.set_exposure(new.exposure_ms.clamp(info.exposure_min_ms, info.exposure_max_ms))?;
            }
            if new.gain_step != old.gain_step {
                cam.set_gain(new.gain_step)?;
            }
            if new.black_level != old.black_level {
                cam.set_black_level(new.black_level.min(info.black_level_max))?;
            }
        }
        new.average = new.average.clamp(1, 100);
        *self.s.cfg.lock() = new.clone();
        // Re-analyze a still (file) frame with the new settings.
        if !self.s.running.load(Ordering::SeqCst) {
            let latest = self.s.latest.lock().clone();
            if let Some(l) = latest {
                let metrics = analysis::analyze(&l.processed, &new.analysis);
                *self.s.latest.lock() = Some(Arc::new(Latest { raw: l.raw.clone(), processed: l.processed.clone(), metrics }));
            }
        }
        Ok(new)
    }

    pub fn start(&self) -> Result<()> {
        if self.s.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        if self.s.cam.lock().is_none() {
            bail!("no camera connected");
        }
        if let Some(t) = self.thread.lock().take() {
            let _ = t.join();
        }
        self.s.running.store(true, Ordering::SeqCst);
        *self.s.source.lock() = "live".into();
        let s = self.s.clone();
        *self.thread.lock() = Some(std::thread::Builder::new().name("capture".into()).spawn(move || run(s))?);
        Ok(())
    }

    pub fn stop(&self) {
        self.s.running.store(false, Ordering::SeqCst);
        if let Some(t) = self.thread.lock().take() {
            let _ = t.join();
        }
        if let Some(cam) = self.s.cam.lock().as_mut() {
            cam.stop();
        }
    }

    /// Binary packet for the UI: see `build_packet`. Empty when nothing newer than `since`.
    pub fn latest_packet(&self, since: u64) -> Vec<u8> {
        let latest = self.s.latest.lock().clone();
        match latest {
            Some(l) if l.processed.index != since || since == u64::MAX => {
                let fps = self.s.stats.lock().0;
                let source = self.s.source.lock().clone();
                build_packet(&l, fps, &source)
            }
            _ => Vec::new(),
        }
    }

    pub fn capture_background(&self, frames: usize) -> Result<()> {
        if !self.s.running.load(Ordering::SeqCst) {
            bail!("start the live view first");
        }
        *self.s.bg_request.lock() = Some((frames.clamp(1, 100), Vec::new(), 0));
        let t0 = Instant::now();
        while self.s.bg_request.lock().is_some() {
            if t0.elapsed() > Duration::from_secs(60) || !self.s.running.load(Ordering::SeqCst) {
                *self.s.bg_request.lock() = None;
                bail!("background capture timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    pub fn clear_background(&self) {
        *self.s.background.lock() = None;
    }

    pub fn open_wcf(&self, path: &Path) -> Result<String> {
        self.stop();
        let file = wcf::read(path)?;
        let frame = file.frames.into_iter().next().ok_or_else(|| anyhow!("empty file"))?;
        let cfg = self.s.cfg.lock().clone();
        let metrics = analysis::analyze(&frame, &cfg.analysis);
        *self.s.latest.lock() = Some(Arc::new(Latest { raw: frame.clone(), processed: frame, metrics }));
        *self.s.source.lock() = format!("file:{}", path.display());
        Ok(file.version)
    }

    fn latest(&self) -> Result<Arc<Latest>> {
        self.s.latest.lock().clone().ok_or_else(|| anyhow!("no frame yet"))
    }

    pub fn save_wcf(&self, path: &Path) -> Result<()> {
        let l = self.latest()?;
        let m = &l.metrics;
        let st = wcf::FrameStats {
            peak: m.peak as u32,
            min: m.min as u32,
            mean: m.mean as u32,
            centroid_px: m.centroid_px,
            peak_px: m.peak_px,
        };
        let ty = match self.s.info.lock().as_ref().map(|i| i.model.as_str()) {
            Some(m) if m.contains("XHR") => 0x2b,
            _ => 0x2a,
        };
        wcf::write(path, &[(l.processed.clone(), st)], ty)
    }

    pub fn export_image_csv(&self, path: &Path) -> Result<()> {
        let l = self.latest()?;
        let f = &l.processed;
        let mut s = String::with_capacity(f.width * f.height * 6);
        for y in 0..f.height {
            let row = &f.data[y * f.width..(y + 1) * f.width];
            let line: Vec<String> = row.iter().map(|v| v.to_string()).collect();
            s.push_str(&line.join(","));
            s.push('\n');
        }
        std::fs::write(path, s).context("writing image CSV")
    }

    pub fn export_profiles_csv(&self, path: &Path) -> Result<()> {
        let l = self.latest()?;
        let m = &l.metrics;
        let (lx, ly) = (&m.profile_line_x, &m.profile_line_y);
        let (a, b) = if m.principal_axes { ("u", "v") } else { ("x", "y") };
        let mut s = if m.principal_axes {
            format!(
                "# profiles along the principal axes through the centroid (u = major at {:.2} deg), positions from the centroid; baseline {:.1} subtracted\n",
                m.orientation_deg, m.baseline
            )
        } else {
            format!("# profiles through row {} / column {} (baseline {:.1} subtracted)\n", m.profile_row, m.profile_col, m.baseline)
        };
        s.push_str(&format!("{a}_um,{a}_value,{b}_um,{b}_value\n"));
        for i in 0..m.profile_x.len().max(m.profile_y.len()) {
            let xs = m.profile_x.get(i).map(|v| format!("{:.3},{:.1}", lx.origin_um + i as f64 * lx.step_um, v)).unwrap_or(",".into());
            let ys = m.profile_y.get(i).map(|v| format!("{:.3},{:.1}", ly.origin_um + i as f64 * ly.step_um, v)).unwrap_or(",".into());
            s.push_str(&format!("{xs},{ys}\n"));
        }
        std::fs::write(path, s).context("writing profile CSV")
    }

    pub fn metrics(&self) -> Result<Metrics> {
        Ok(self.latest()?.metrics.clone())
    }

    // ---- logging ------------------------------------------------------------------------

    pub fn log_start(&self, interval_ms: f64) {
        let mut l = self.s.log.lock();
        l.enabled = true;
        l.interval_ms = interval_ms.max(0.0);
        l.last_ms = 0.0;
    }

    pub fn log_stop(&self) {
        self.s.log.lock().enabled = false;
    }

    pub fn log_clear(&self) {
        self.s.log.lock().rows.clear();
    }

    pub fn log_rows(&self, from: usize) -> Vec<LogRow> {
        let l = self.s.log.lock();
        l.rows.get(from.min(l.rows.len())..).map(|r| r.to_vec()).unwrap_or_default()
    }

    pub fn log_export(&self, path: &Path) -> Result<()> {
        let l = self.s.log.lock();
        let mut s = String::from(LogRow::CSV_HEADER);
        s.push('\n');
        for r in &l.rows {
            s.push_str(&r.csv());
            s.push('\n');
        }
        std::fs::write(path, s).context("writing log CSV")
    }
}

/// Capture loop.
fn run(s: Arc<Shared>) {
    let mut avg: VecDeque<Arc<Vec<u16>>> = VecDeque::new();
    let mut sum: Vec<u32> = Vec::new();
    let mut times: VecDeque<Instant> = VecDeque::new();
    let mut errors = 0u32;
    let mut frames = 0u64;
    let mut dropped = 0u64;

    while s.running.load(Ordering::SeqCst) {
        let grabbed = {
            let mut guard = s.cam.lock();
            match guard.as_mut() {
                Some(cam) => cam.grab(),
                None => break,
            }
        };
        let raw = match grabbed {
            Ok(f) => {
                errors = 0;
                f
            }
            Err(e) if e.downcast_ref::<SkipFrame>().is_some() => {
                dropped += 1;
                continue;
            }
            Err(e) => {
                errors += 1;
                dropped += 1;
                s.stats.lock().3 = Some(format!("{e:#}"));
                if errors > 20 {
                    s.stats.lock().3 = Some(format!("camera stopped responding: {e:#}"));
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        frames += 1;
        let cfg = s.cfg.lock().clone();

        // Background capture request: average N raw frames.
        {
            let mut req = s.bg_request.lock();
            if let Some((n, acc, got)) = req.as_mut() {
                if acc.len() != raw.data.len() {
                    acc.clear();
                    acc.resize(raw.data.len(), 0.0);
                    *got = 0;
                }
                for (a, &v) in acc.iter_mut().zip(raw.data.iter()) {
                    *a += v as f64;
                }
                *got += 1;
                if *got >= *n {
                    let k = *got as f64;
                    *s.background.lock() = Some(acc.iter().map(|&a| (a / k) as f32).collect());
                    *req = None;
                }
            }
        }

        // Boxcar averaging.
        let mut data = raw.data.clone();
        if cfg.average > 1 {
            if sum.len() != raw.data.len() {
                sum = vec![0; raw.data.len()];
                avg.clear();
            }
            for (a, &v) in sum.iter_mut().zip(raw.data.iter()) {
                *a += v as u32;
            }
            avg.push_back(raw.data.clone());
            while avg.len() > cfg.average {
                let old = avg.pop_front().unwrap();
                for (a, &v) in sum.iter_mut().zip(old.iter()) {
                    *a -= v as u32;
                }
            }
            let k = avg.len() as u32;
            data = Arc::new(sum.iter().map(|&a| (a / k) as u16).collect());
        } else if !avg.is_empty() {
            avg.clear();
            sum.clear();
        }

        // Background subtraction.
        if cfg.subtract_background {
            if let Some(bg) = s.background.lock().as_ref() {
                if bg.len() == data.len() {
                    data = Arc::new(
                        data.iter().zip(bg.iter()).map(|(&v, &b)| (v as f32 - b).clamp(0.0, 65535.0) as u16).collect(),
                    );
                }
            }
        }

        let processed = Frame { data, ..raw.clone() };
        let metrics = analysis::analyze(&processed, &cfg.analysis);

        // Auto-exposure on the raw frame (DataRay's rule, FUN_180039920) and auto black level.
        if cfg.auto_exposure || cfg.auto_black {
            let raw_m = if cfg.average > 1 || cfg.subtract_background {
                analysis::analyze(&raw, &AnalysisSettings { iso_aperture: false, ..cfg.analysis.clone() })
            } else {
                metrics.clone()
            };
            adjust(&s, &cfg, &raw_m);
        }

        // FPS over the last second.
        let now = Instant::now();
        times.push_back(now);
        while times.len() > 2 && now.duration_since(times[0]) > Duration::from_secs(2) {
            times.pop_front();
        }
        let fps = if times.len() > 1 {
            (times.len() - 1) as f64 / now.duration_since(times[0]).as_secs_f64()
        } else {
            0.0
        };
        *s.stats.lock() = (fps, frames, dropped, None);

        {
            let mut log = s.log.lock();
            if log.enabled && processed.timestamp_ms - log.last_ms >= log.interval_ms {
                log.last_ms = processed.timestamp_ms;
                log.rows.push(LogRow::from(&processed, &metrics));
            }
        }
        *s.latest.lock() = Some(Arc::new(Latest { raw, processed, metrics }));
    }
    s.running.store(false, Ordering::SeqCst);
    if let Some(cam) = s.cam.lock().as_mut() {
        cam.stop();
    }
}

fn adjust(s: &Shared, cfg: &Config, m: &Metrics) {
    let info = match s.info.lock().clone() {
        Some(i) => i,
        None => return,
    };
    let mut next = cfg.clone();
    if cfg.auto_exposure {
        let peak = m.peak_smoothed.max(1.0);
        let mut k = ((57343.125 / peak + 1.0) * 0.5).clamp(0.15, 7.0);
        if k > 1.05 || k < 0.95 || peak > 60292.2 {
            if m.saturated_pixels > 0 {
                // Saturated: the peak says nothing about how far over we are; back off fast.
                k = 0.5;
            } else if peak < 62258.25 {
                k = k.sqrt();
            }
            next.exposure_ms = (cfg.exposure_ms * k).clamp(info.exposure_min_ms, info.exposure_max_ms.min(2000.0));
        }
    }
    if cfg.auto_black {
        // DataRay keeps the darkest pixel between target/2 and 2·target (target 0x200).
        let target = 512.0;
        if m.min_smoothed < target / 2.0 && cfg.black_level < info.black_level_max {
            next.black_level = cfg.black_level + 1;
        } else if m.min_smoothed >= 2.0 * target && cfg.black_level > 0 {
            next.black_level = cfg.black_level - 1;
        }
    }
    if (next.exposure_ms - cfg.exposure_ms).abs() > 1e-9 || next.black_level != cfg.black_level {
        if let Some(cam) = s.cam.lock().as_mut() {
            if next.exposure_ms != cfg.exposure_ms {
                match cam.set_exposure(next.exposure_ms) {
                    Ok(v) => next.exposure_ms = v,
                    Err(_) => next.exposure_ms = cfg.exposure_ms,
                }
            }
            if next.black_level != cfg.black_level && cam.set_black_level(next.black_level).is_err() {
                next.black_level = cfg.black_level;
            }
        }
        let mut c = s.cfg.lock();
        c.exposure_ms = next.exposure_ms;
        c.black_level = next.black_level;
    }
}

/// Packet layout (little endian):
/// `"BSF1"` · u32 json_len · json · pad→4 · u16 display[dw·dh] · pad→4 · f32 profile_x[w] · f32 profile_y[h]
fn build_packet(l: &Latest, fps: f64, source: &str) -> Vec<u8> {
    let f = &l.processed;
    let m = &l.metrics;
    let scale = ((f.width.max(f.height) + 1279) / 1280).max(1);
    let (dw, dh) = (f.width / scale, f.height / scale);
    let json = serde_json::json!({
        "index": f.index,
        "width": f.width,
        "height": f.height,
        "dispWidth": dw,
        "dispHeight": dh,
        "dispScale": scale,
        "pixelUm": [f.pixel_um.0, f.pixel_um.1],
        "exposureMs": f.exposure_ms,
        "gain": f.gain,
        "timestampMs": f.timestamp_ms,
        "fps": fps,
        "source": source,
        "metrics": m,
        "profileXLen": m.profile_x.len(),
        "profileYLen": m.profile_y.len(),
    })
    .to_string();
    let mut out = Vec::with_capacity(16 + json.len() + dw * dh * 2 + (f.width + f.height) * 4);
    out.extend_from_slice(b"BSF1");
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    while out.len() % 4 != 0 {
        out.push(0);
    }
    if scale == 1 {
        for &v in f.data.iter() {
            out.extend_from_slice(&v.to_le_bytes());
        }
    } else {
        let n = (scale * scale) as u32;
        for y in 0..dh {
            for x in 0..dw {
                let mut acc = 0u32;
                for yy in 0..scale {
                    let row = (y * scale + yy) * f.width + x * scale;
                    for xx in 0..scale {
                        acc += f.data[row + xx] as u32;
                    }
                }
                out.extend_from_slice(&((acc / n) as u16).to_le_bytes());
            }
        }
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    for v in m.profile_x.iter().chain(m.profile_y.iter()) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` on a thread and fails if it doesn't finish quickly (catches lock deadlocks).
    fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| panic!("{what} deadlocked or hung"))
    }

    /// Exercise the engine API with the simulator, the way the UI does.
    #[test]
    fn simulator_session_does_not_deadlock() {
        let e = Arc::new(Engine::default());
        let st = within("status (idle)", { let e = e.clone(); move || e.status() });
        assert!(!st.connected);
        let info = within("connect", { let e = e.clone(); move || e.connect(true) }).unwrap();
        assert!(info.simulated);
        within("start", { let e = e.clone(); move || e.start() }).unwrap();
        std::thread::sleep(Duration::from_millis(600));
        for _ in 0..5 {
            let st = within("status (running)", { let e = e.clone(); move || e.status() });
            assert!(st.connected && st.running);
            let p = within("latest_packet", { let e = e.clone(); move || e.latest_packet(u64::MAX) });
            assert!(!p.is_empty());
        }
        let mut cfg = e.status().config;
        cfg.auto_exposure = false;
        cfg.exposure_ms = 0.1;
        cfg.analysis.principal_axes = true;
        let c = within("set_config", { let e = e.clone(); move || e.set_config(cfg) }).unwrap();
        assert!(c.analysis.principal_axes);
        std::thread::sleep(Duration::from_millis(500));
        let m = within("metrics", { let e = e.clone(); move || e.metrics() }).unwrap();
        assert!(m.principal_axes && m.aperture.is_some());
        within("log", { let e = e.clone(); move || { e.log_start(0.0); std::thread::sleep(Duration::from_millis(300)); e.log_stop(); e.log_rows(0).len() } });
        within("stop", { let e = e.clone(); move || e.stop() });
        within("disconnect", { let e = e.clone(); move || e.disconnect() });
        assert!(!e.status().connected);
    }
}

#[cfg(test)]
mod hw {
    use super::*;

    /// Needs the camera: `cargo test --release -- --ignored hw_engine --nocapture`
    #[test]
    #[ignore]
    fn hw_engine_manual_exposure_sweep() {
        let e = Engine::default();
        println!("{:?}", e.connect(false).map(|i| i.model));
        e.start().unwrap();
        std::thread::sleep(Duration::from_millis(800));
        let mut cfg = e.status().config;
        cfg.auto_exposure = false;
        for ms in [1.0, 0.5, 0.2, 0.1, 0.05, 0.03, 0.02, 0.01, 0.0] {
            cfg.exposure_ms = ms;
            let r = e.set_config(cfg.clone());
            std::thread::sleep(Duration::from_millis(400));
            let st = e.status();
            let m = e.metrics().ok();
            println!(
                "set {ms} -> {:?}  running={} fps={:.1} err={:?} peak={:?} d4s={:?}",
                r.map(|c| c.exposure_ms),
                st.running,
                st.fps,
                st.last_error,
                m.as_ref().map(|m| m.peak),
                m.as_ref().map(|m| (m.d4s_x_um, m.d4s_y_um))
            );
            let _ = e.latest_packet(u64::MAX);
        }
        e.stop();
    }
}
