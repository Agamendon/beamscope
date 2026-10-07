//! DataRay `.wcf` capture files.
//!
//! The format is a raw dump of DataRay's in-memory structures (loader `FUN_18008ad20`,
//! writer `FUN_18008cd60`, parser `FUN_1800884e0` / `FUN_180087b40`):
//!
//! ```text
//! 0x0000  file header, 0x15D8 bytes
//!         +0x00 ".IRD"  +0x04 0x31  +0x08 header size (0x15D8)  +0x0C frame count
//!         +0x10 image bytes of frame 0   +0x14 version string (40 bytes, e.g. "8.0E70")
//!         +0x40 settings block (".IRD", clip levels at +0x238/+0x240, …)
//! 0x15D8  frame[i]: 0x3A8-byte header, width*height u16 pixels, 8 pad bytes
//!         +0x00 ".IRD"  +0x04 0x1E  +0x10 frame bytes  +0x14 width  +0x18 height
//!         +0x20/+0x28 pixel pitch x/y (µm, f64)  +0x140 exposure (ms, f64)
//!         +0x23C peak  +0x298 gain (f64)  +0x2E8 min  +0x308 camera type  +0x30C mean
//! end     zero trailer (≥ 0x15E0 bytes)
//! ```
//!
//! Older files (e.g. v5.00K5) keep the same header but may store frames without the
//! per-frame size field; we only rely on width/height at +0x14/+0x18.

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::camera::Frame;

const MAGIC: &[u8; 4] = b".IRD";
const FILE_HEADER: usize = 0x15d8;
const FRAME_HEADER: usize = 0x3a8;
const FRAME_PAD: usize = 8;
const TRAILER: usize = 0x15e0;

fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .context("truncated .wcf")
}

fn f64_at(b: &[u8], o: usize) -> f64 {
    b.get(o..o + 8)
        .map(|s| f64::from_le_bytes(s.try_into().unwrap()))
        .unwrap_or(0.0)
}

fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_f64(b: &mut [u8], o: usize, v: f64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

pub struct WcfFile {
    pub version: String,
    pub frames: Vec<Frame>,
}

pub fn read(path: &Path) -> Result<WcfFile> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&b)
}

pub fn parse(b: &[u8]) -> Result<WcfFile> {
    if b.len() < FILE_HEADER || &b[0..4] != MAGIC || u32_at(b, 4)? != 0x31 {
        bail!("not a DataRay .wcf file");
    }
    let header = u32_at(b, 8)? as usize;
    if header < 0x3c || header > b.len() {
        bail!("invalid .wcf header size {header:#x}");
    }
    let count = u32_at(b, 12)?.max(1) as usize;
    let version = {
        let v = &b[0x14..0x3c];
        let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
        String::from_utf8_lossy(&v[..end]).into_owned()
    };

    let mut frames = Vec::new();
    let mut off = header;
    for index in 0..count {
        if off + FRAME_HEADER > b.len() || &b[off..off + 4] != MAGIC {
            break;
        }
        let w = u32_at(b, off + 0x14)? as usize;
        let h = u32_at(b, off + 0x18)? as usize;
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            bail!("frame {index}: implausible size {w}x{h}");
        }
        let img = off + FRAME_HEADER;
        let n = w * h;
        if img + n * 2 > b.len() {
            bail!("frame {index}: image truncated");
        }
        let data: Vec<u16> = b[img..img + n * 2]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut px = (f64_at(b, off + 0x20), f64_at(b, off + 0x28));
        if !(px.0 > 0.0 && px.0 < 1000.0) {
            px.0 = 1.0;
        }
        if !(px.1 > 0.0 && px.1 < 1000.0) {
            px.1 = px.0;
        }
        let exposure = f64_at(b, off + 0x140);
        let gain = f64_at(b, off + 0x298);
        frames.push(Frame {
            width: w,
            height: h,
            pixel_um: px,
            data: Arc::new(data),
            exposure_ms: if exposure.is_finite() && exposure > 0.0 { exposure } else { 0.0 },
            gain: if gain.is_finite() && gain > 0.0 { gain } else { 1.0 },
            timestamp_ms: 0.0,
            index: index as u64,
        });
        off = img + n * 2 + FRAME_PAD;
    }
    if frames.is_empty() {
        bail!("no frames in .wcf file");
    }
    Ok(WcfFile { version, frames })
}

pub struct FrameStats {
    pub peak: u32,
    pub min: u32,
    pub mean: u32,
    pub centroid_px: (f64, f64),
    pub peak_px: (usize, usize),
}

/// Serialize frames into a file DataRay can open.
pub fn write(path: &Path, frames: &[(Frame, FrameStats)], camera_type: u32) -> Result<()> {
    std::fs::write(path, build(frames, camera_type))
        .with_context(|| format!("writing {}", path.display()))
}

pub fn build(frames: &[(Frame, FrameStats)], camera_type: u32) -> Vec<u8> {
    let mut out = vec![0u8; FILE_HEADER];
    out[0..4].copy_from_slice(MAGIC);
    put_u32(&mut out, 4, 0x31);
    put_u32(&mut out, 8, FILE_HEADER as u32);
    put_u32(&mut out, 12, frames.len() as u32);
    if let Some((f, _)) = frames.first() {
        put_u32(&mut out, 0x10, (f.width * f.height * 2) as u32);
    }
    let ver = b"8.0E70";
    out[0x14..0x14 + ver.len()].copy_from_slice(ver);

    // Settings block: the defaults DataRay itself writes (clip levels 13.5 % / 50 %, …).
    let s = 0x40;
    out[s..s + 4].copy_from_slice(MAGIC);
    put_f64(&mut out, 0x238, 0.1354);
    put_f64(&mut out, 0x240, 0.5);
    put_u32(&mut out, 0x11b8, 1);
    put_u32(&mut out, 0x11d4, 1);
    put_u32(&mut out, 0x11f4, 5);
    put_f64(&mut out, 0x12a0, 0.2);
    put_f64(&mut out, 0x12a8, 0.67);
    put_f64(&mut out, 0x12b0, 1.0);
    put_f64(&mut out, 0x12f8, 1.0);
    put_f64(&mut out, 0x1300, 1.0);
    put_f64(&mut out, 0x1338, 800.0);
    out[0x15d0..0x15d4].copy_from_slice(MAGIC);

    for (i, (f, st)) in frames.iter().enumerate() {
        let n = f.width * f.height;
        let mut hdr = vec![0u8; FRAME_HEADER];
        hdr[0..4].copy_from_slice(MAGIC);
        put_u32(&mut hdr, 0x04, 0x1e);
        put_u32(&mut hdr, 0x08, i as u32);
        put_u32(&mut hdr, 0x0c, frames.len() as u32);
        put_u32(&mut hdr, 0x10, (n * 2 + FRAME_HEADER + FRAME_PAD) as u32);
        put_u32(&mut hdr, 0x14, f.width as u32);
        put_u32(&mut hdr, 0x18, f.height as u32);
        put_f64(&mut hdr, 0x20, f.pixel_um.0);
        put_f64(&mut hdr, 0x28, f.pixel_um.1);
        put_u32(&mut hdr, 0x30, 16);
        put_u32(&mut hdr, 0x34, 1);
        put_u32(&mut hdr, 0x38, st.peak);
        put_u32(&mut hdr, 0x44, f.width as u32);
        put_u32(&mut hdr, 0x48, f.height as u32);
        put_f64(&mut hdr, 0x140, f.exposure_ms);
        for (o, (x, y)) in [(0x1a0, st.centroid_px), (0x1b0, st.centroid_px), (0x1d0, st.centroid_px)] {
            put_f64(&mut hdr, o, x);
            put_f64(&mut hdr, o + 8, y);
        }
        put_f64(&mut hdr, 0x1c0, st.peak_px.0 as f64);
        put_f64(&mut hdr, 0x1c8, st.peak_px.1 as f64);
        hdr[0x230..0x234].copy_from_slice(MAGIC);
        put_u32(&mut hdr, 0x23c, st.peak);
        put_f64(&mut hdr, 0x298, f.gain);
        put_u32(&mut hdr, 0x2e4, 1);
        put_u32(&mut hdr, 0x2e8, st.min);
        put_u32(&mut hdr, 0x308, camera_type);
        put_u32(&mut hdr, 0x30c, st.mean);
        put_u32(&mut hdr, 0x344, 1);
        out.extend_from_slice(&hdr);
        out.reserve(n * 2 + FRAME_PAD);
        for &p in f.data.iter() {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; FRAME_PAD]);
    }
    out.extend_from_slice(&[0u8; TRAILER]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Option<Vec<u8>> {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(name);
        std::fs::read(p).ok()
    }

    #[test]
    fn reads_datarays_capture() {
        let Some(b) = sample("capture.wcf") else { return };
        let f = parse(&b).unwrap();
        assert_eq!(f.version, "8.0E70");
        let fr = &f.frames[0];
        assert_eq!((fr.width, fr.height), (640, 512));
        assert!((fr.pixel_um.0 - 10.4).abs() < 1e-9);
        assert_eq!(*fr.data.iter().max().unwrap(), 65408);
    }

    /// DataRay's own results stored in the user's captures must be reproduced.
    #[test]
    fn matches_dataray_stored_results() {
        for name in ["capture.wcf", "capture_again.wcf"] {
            let Some(b) = sample(name) else { continue };
            let fr = &parse(&b).unwrap().frames[0];
            let f = FILE_HEADER;
            let stored_diam = f64_at(&b, f + 0x260);
            let stored_c = (f64_at(&b, f + 0x1a0), f64_at(&b, f + 0x1a8));
            let m = crate::analysis::analyze(fr, &Default::default());
            // Our border baseline differs from DataRay's by a few counts; results still agree
            // to the last printed digit on both captures, so keep the tolerances tight.
            assert!((m.area_diameter_um - stored_diam).abs() / stored_diam < 1e-4, "{name}: {} vs {stored_diam}", m.area_diameter_um);
            assert!((m.dataray_centroid_px.0 - stored_c.0).abs() < 1e-3, "{name}: {:?} vs {stored_c:?}", m.dataray_centroid_px);
            assert!((m.dataray_centroid_px.1 - stored_c.1).abs() < 1e-3, "{name}: {:?} vs {stored_c:?}", m.dataray_centroid_px);
        }
    }

    #[test]
    fn round_trip() {
        let fr = Frame {
            width: 8,
            height: 4,
            pixel_um: (5.2, 5.2),
            data: Arc::new((0..32u16).map(|v| v << 6).collect()),
            exposure_ms: 1.5,
            gain: 2.0,
            timestamp_ms: 0.0,
            index: 0,
        };
        let st = FrameStats { peak: 31 << 6, min: 0, mean: 15 << 6, centroid_px: (4.0, 2.0), peak_px: (7, 3) };
        let b = build(&[(fr.clone(), st)], 0x2a);
        let back = parse(&b).unwrap();
        assert_eq!(back.frames[0].data, fr.data);
        assert_eq!(back.frames[0].exposure_ms, 1.5);
        assert_eq!(back.frames[0].gain, 2.0);
    }
}

#[cfg(test)]
mod compare {
    /// Prints our metrics for ../../capture.wcf next to the numbers DataRay stored in it.
    #[test]
    #[ignore]
    fn compare_capture() {
        let name = std::env::var("WCF").unwrap_or_else(|_| "../../capture.wcf".into());
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
        let b = std::fs::read(p).unwrap();
        let f = super::parse(&b).unwrap();
        let fr0 = &f.frames[0];
        println!("{} {}x{} px {:?} exp {}", f.version, fr0.width, fr0.height, fr0.pixel_um, fr0.exposure_ms);
        let m = crate::analysis::analyze(fr0, &Default::default());
        println!("ours: area ⌀ {:.3} µm  DataRay-centroid ({:.4}, {:.4})  baseline {:.2}", m.area_diameter_um, m.dataray_centroid_px.0, m.dataray_centroid_px.1, m.baseline);
        println!("ours: centroid {:?} px  D4σ {:.2} x {:.2} µm  maj/min {:.2}/{:.2}  clip {:?}  gauss {:.2} / {:.2}  peak {} min {} mean {:.1} baseline {:.1}",
            m.centroid_px, m.d4s_x_um, m.d4s_y_um, m.d4s_major_um, m.d4s_minor_um,
            m.clip.iter().map(|c| (c.x_um, c.y_um)).collect::<Vec<_>>(), m.gauss_x.diameter_um, m.gauss_y.diameter_um,
            m.peak, m.min, m.mean, m.baseline);
        let fr = 0x15d8;
        let d = |o: usize| f64::from_le_bytes(b[fr + o..fr + o + 8].try_into().unwrap());
        println!("DataRay doubles: {}", [0x70, 0x78, 0x80, 0x88, 0x90, 0xc8, 0xd0, 0xd8, 0xe0, 0xe8, 0x148, 0x1a0, 0x1a8, 0x1b0, 0x1b8, 0x260, 0x278]
            .iter().map(|&o| format!("+{o:#x}={:.4}", d(o))).collect::<Vec<_>>().join("  "));
    }
}
