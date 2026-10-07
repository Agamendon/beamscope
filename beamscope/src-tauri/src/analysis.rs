//! Beam metrics: ISO 11146 second moments (with iterative integration area), clip-level
//! widths, 1-D Gaussian fits and the M² caustic fit.

use serde::{Deserialize, Serialize};

use crate::camera::Frame;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSettings {
    /// Subtract a baseline estimated from the image border before computing moments.
    pub auto_baseline: bool,
    /// ISO 11146-3 iterative rectangular integration area (3× the beam widths).
    pub iso_aperture: bool,
    /// Pixels below this fraction of peak (after baseline) are zeroed. 0 disables.
    pub noise_clip: f64,
    /// Clip levels for the knife-free "clip" widths, as fractions of the profile peak.
    pub clip_levels: [f64; 2],
    /// Take the X/Y profiles through the centroid (true) or through the peak pixel (false).
    pub profiles_through_centroid: bool,
    /// Average this many rows/columns for each line profile (odd numbers are centered).
    pub profile_band: usize,
    /// Take the two profiles along the beam's principal axes (u = major, v = minor) through
    /// the centroid, like DataRay's rotated crosshair, instead of along sensor X/Y.
    #[serde(default)]
    pub principal_axes: bool,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            auto_baseline: true,
            iso_aperture: true,
            noise_clip: 0.0,
            clip_levels: [0.135, 0.5],
            profiles_through_centroid: true,
            profile_band: 1,
            principal_axes: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GaussFit {
    /// 1/e² diameter in µm.
    pub diameter_um: f64,
    /// Center in µm, in the same coordinate as the profile's position axis.
    pub center_um: f64,
    pub amplitude: f64,
    pub offset: f64,
    /// Coefficient of determination, 0..1.
    pub r2: f64,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClipWidth {
    pub level: f64,
    pub x_um: f64,
    pub y_um: f64,
}

/// A straight sampling line through the image. Sample `i` sits at pixel
/// `(x0 + i·dx, y0 + i·dy)` and at position `origin_um + i·step_um` on the plot axis.
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLine {
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub step_um: f64,
    pub origin_um: f64,
    pub len: usize,
}

/// The ISO 11146-3 integration area actually used for the final moments: a rectangle
/// centred on the centroid, aligned with the principal axes, 3× the D4σ widths.
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aperture {
    pub cx: f64,
    pub cy: f64,
    /// Half-sizes along the major / minor axes, in pixels.
    pub half_major_px: f64,
    pub half_minor_px: f64,
    /// Major-axis direction in image coordinates (y down), radians.
    pub angle_rad: f64,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub width: usize,
    pub height: usize,
    pub pixel_um: (f64, f64),
    pub peak: f64,
    /// Peak of the 3-tap horizontally smoothed image (used for auto-exposure, like DataRay).
    pub peak_smoothed: f64,
    /// Minimum of the same smoothed image (dead pixels ignored), used for auto black level.
    pub min_smoothed: f64,
    pub peak_px: (usize, usize),
    pub min: f64,
    pub mean: f64,
    pub baseline: f64,
    pub total: f64,
    pub saturated_pixels: usize,
    pub centroid_px: (f64, f64),
    pub centroid_um: (f64, f64),
    /// ISO 11146 D4σ widths along the lab X/Y axes, µm.
    pub d4s_x_um: f64,
    pub d4s_y_um: f64,
    /// D4σ along the principal axes, µm (major ≥ minor).
    pub d4s_major_um: f64,
    pub d4s_minor_um: f64,
    /// Azimuth of the major axis, degrees, counter-clockwise from +X (image y points down).
    pub orientation_deg: f64,
    pub ellipticity: f64,
    pub effective_diameter_um: f64,
    /// DataRay's "effective diameter": 2·√(A/π) of the area above clip level 1 (FUN_180195090).
    pub area_diameter_um: f64,
    /// DataRay's centroid: intensity-weighted over pixels ≥ clip level 1 (FUN_180195090).
    pub dataray_centroid_px: (f64, f64),
    pub aperture_iterations: u32,
    pub aperture: Option<Aperture>,
    /// True when the two profiles (and the clip / Gaussian widths) are along u/v, not X/Y.
    pub principal_axes: bool,
    /// Clip widths along the two profiles (`x_um` = X or u, `y_um` = Y or v).
    pub clip: Vec<ClipWidth>,
    pub gauss_x: GaussFit,
    pub gauss_y: GaussFit,
    pub profile_row: usize,
    pub profile_col: usize,
    pub profile_line_x: ProfileLine,
    pub profile_line_y: ProfileLine,
    #[serde(skip)]
    pub profile_x: Vec<f32>,
    #[serde(skip)]
    pub profile_y: Vec<f32>,
}

/// Mean of the outer border (≈4 % of each side) — the dark level for auto baseline.
fn border_mean(f: &Frame) -> f64 {
    let (w, h) = (f.width, f.height);
    let bx = (w / 25).max(1);
    let by = (h / 25).max(1);
    let mut s = 0.0f64;
    let mut n = 0usize;
    for y in 0..h {
        let row = &f.data[y * w..(y + 1) * w];
        if y < by || y >= h - by {
            s += row.iter().map(|&v| v as f64).sum::<f64>();
            n += w;
        } else {
            s += row[..bx].iter().chain(&row[w - bx..]).map(|&v| v as f64).sum::<f64>();
            n += 2 * bx;
        }
    }
    s / n.max(1) as f64
}

struct Moments {
    total: f64,
    cx: f64,
    cy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

/// Moments of `img` (baseline-subtracted, f32) over pixels inside an optional rotated box.
fn moments(img: &[f32], w: usize, h: usize, sx: f64, sy: f64, aperture: Option<(f64, f64, f64, f64, f64)>) -> Option<Moments> {
    let (mut x0, mut x1, mut y0, mut y1) = (0usize, w, 0usize, h);
    let mut inside: Option<Box<dyn Fn(f64, f64) -> bool>> = None;
    if let Some((cx, cy, hw, hh, phi)) = aperture {
        // Bounding box of the rotated rectangle (in pixel units).
        let (c, s) = (phi.cos(), phi.sin());
        let ex = (hw * c).abs() + (hh * s).abs();
        let ey = (hw * s).abs() + (hh * c).abs();
        x0 = ((cx - ex).floor().max(0.0)) as usize;
        x1 = ((cx + ex).ceil().min(w as f64)) as usize;
        y0 = ((cy - ey).floor().max(0.0)) as usize;
        y1 = ((cy + ey).ceil().min(h as f64)) as usize;
        inside = Some(Box::new(move |x: f64, y: f64| {
            let (dx, dy) = (x - cx, y - cy);
            (dx * c + dy * s).abs() <= hw && (-dx * s + dy * c).abs() <= hh
        }));
    }
    let (mut m0, mut mx, mut my) = (0.0f64, 0.0f64, 0.0f64);
    let (mut mxx, mut myy, mut mxy) = (0.0f64, 0.0f64, 0.0f64);
    for y in y0..y1 {
        let row = &img[y * w..(y + 1) * w];
        let fy = y as f64;
        let (mut r0, mut rx, mut rxx) = (0.0f64, 0.0f64, 0.0f64);
        for x in x0..x1 {
            let v = row[x] as f64;
            if v == 0.0 {
                continue;
            }
            let fx = x as f64;
            if let Some(f) = &inside {
                if !f(fx, fy) {
                    continue;
                }
            }
            r0 += v;
            rx += v * fx;
            rxx += v * fx * fx;
        }
        m0 += r0;
        mx += rx;
        my += r0 * fy;
        mxx += rxx;
        myy += r0 * fy * fy;
        mxy += rx * fy;
    }
    if m0 <= 0.0 {
        return None;
    }
    let cx = mx / m0;
    let cy = my / m0;
    Some(Moments {
        total: m0,
        cx,
        cy,
        sxx: (mxx / m0 - cx * cx) * sx * sx,
        syy: (myy / m0 - cy * cy) * sy * sy,
        sxy: (mxy / m0 - cx * cy) * sx * sy,
    })
}

/// ISO 11146-1 principal-axis widths from second moments (µm²). Returns (dx, dy, dmaj, dmin, phi).
fn iso_widths(m: &Moments) -> (f64, f64, f64, f64, f64) {
    let (a, b, c) = (m.sxx.max(0.0), m.syy.max(0.0), m.sxy);
    let dx = 4.0 * a.sqrt();
    let dy = 4.0 * b.sqrt();
    let root = ((a - b).powi(2) + 4.0 * c * c).sqrt();
    let dmaj = 2.0 * std::f64::consts::SQRT_2 * (a + b + root).max(0.0).sqrt();
    let dmin = 2.0 * std::f64::consts::SQRT_2 * (a + b - root).max(0.0).sqrt();
    // Angle of the major axis.
    let phi = 0.5 * (2.0 * c).atan2(a - b);
    (dx, dy, dmaj, dmin, phi)
}

/// Width of the contiguous region around the profile maximum that stays above `level`
/// (fraction of that maximum), with linear interpolation at both edges. In samples.
fn clip_width(p: &[f32], level: f64) -> f64 {
    let Some((imax, &pk)) = p.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()) else {
        return 0.0;
    };
    if pk <= 0.0 {
        return 0.0;
    }
    let t = pk * level as f32;
    let mut i = imax;
    while i > 0 && p[i - 1] >= t {
        i -= 1;
    }
    let mut j = imax;
    while j + 1 < p.len() && p[j + 1] >= t {
        j += 1;
    }
    let left = if i == 0 { 0.0 } else { (i - 1) as f64 + ((t - p[i - 1]) / (p[i] - p[i - 1])) as f64 };
    let right = if j + 1 >= p.len() { j as f64 } else { j as f64 + ((p[j] - t) / (p[j] - p[j + 1])) as f64 };
    (right - left).max(0.0)
}

/// Levenberg–Marquardt fit of `A·exp(-2(x-x0)²/w²) + B`. Returns (A, x0, w, B, r²) in samples.
pub fn fit_gaussian(p: &[f32]) -> Option<(f64, f64, f64, f64, f64)> {
    let n = p.len();
    if n < 5 {
        return None;
    }
    let ys: Vec<f64> = p.iter().map(|&v| v as f64).collect();
    let mut sorted = ys.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let b0 = sorted[n / 2];
    // Seed from the FWHM around the maximum (robust to hot pixels elsewhere).
    let (imax, &ymax) = ys.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap())?;
    if ymax <= b0 {
        return None;
    }
    let fwhm = clip_width(&p.iter().map(|&v| v - b0 as f32).collect::<Vec<_>>(), 0.5).max(1.0);
    let w0 = fwhm / 1.1774;
    let mut q = [ymax - b0, imax as f64, w0.max(1.0), b0];

    // Fit only ±2.5 seed widths around the peak: the wings are mostly noise, and it is fast.
    let lo = ((imax as f64 - 2.5 * w0).floor().max(0.0)) as usize;
    let hi = ((imax as f64 + 2.5 * w0).ceil().min((n - 1) as f64)) as usize;
    let idx: Vec<usize> = if hi >= lo + 6 { (lo..=hi).collect() } else { (lo.saturating_sub(3)..=(hi + 3).min(n - 1)).collect() };

    let model = |q: &[f64; 4], x: f64| q[0] * (-2.0 * (x - q[1]).powi(2) / (q[2] * q[2])).exp() + q[3];
    let sse = |q: &[f64; 4]| idx.iter().map(|&i| (ys[i] - model(q, i as f64)).powi(2)).sum::<f64>();
    let mut lambda = 1e-3;
    let mut err = sse(&q);
    for _ in 0..60 {
        let mut jtj = [[0.0f64; 4]; 4];
        let mut jtr = [0.0f64; 4];
        for &i in &idx {
            let x = i as f64;
            let e = (-2.0 * (x - q[1]).powi(2) / (q[2] * q[2])).exp();
            let j = [
                e,
                q[0] * e * 4.0 * (x - q[1]) / (q[2] * q[2]),
                q[0] * e * 4.0 * (x - q[1]).powi(2) / q[2].powi(3),
                1.0,
            ];
            let r = ys[i] - (q[0] * e + q[3]);
            for a in 0..4 {
                jtr[a] += j[a] * r;
                for b in 0..4 {
                    jtj[a][b] += j[a] * j[b];
                }
            }
        }
        let mut improved = false;
        for _ in 0..8 {
            let mut m = jtj;
            for a in 0..4 {
                m[a][a] *= 1.0 + lambda;
            }
            if let Some(d) = solve4(m, jtr) {
                let cand = [q[0] + d[0], q[1] + d[1], (q[2] + d[2]).abs().max(1e-3), q[3] + d[3]];
                let e2 = sse(&cand);
                if e2 < err {
                    let rel = (err - e2) / err.max(1e-12);
                    q = cand;
                    err = e2;
                    lambda = (lambda * 0.3).max(1e-9);
                    improved = true;
                    if rel < 1e-9 {
                        break;
                    }
                    break;
                }
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    let mean = idx.iter().map(|&i| ys[i]).sum::<f64>() / idx.len() as f64;
    let sst = idx.iter().map(|&i| (ys[i] - mean).powi(2)).sum::<f64>();
    let r2 = if sst > 0.0 { 1.0 - err / sst } else { 0.0 };
    Some((q[0], q[1], q[2], q[3], r2))
}

fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for col in 0..4 {
        let piv = (col..4).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap())?;
        if a[piv][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in 0..4 {
            if r != col {
                let f = a[r][col] / a[col][col];
                for c in col..4 {
                    a[r][c] -= f * a[col][c];
                }
                b[r] -= f * b[col];
            }
        }
    }
    Some([b[0] / a[0][0], b[1] / a[1][1], b[2] / a[2][2], b[3] / a[3][3]])
}

/// Bilinear sample of `img` at a fractional pixel position (pixel centres at integers).
fn bilinear(img: &[f32], w: usize, h: usize, x: f64, y: f64) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f64);
    let y = y.clamp(0.0, (h - 1) as f64);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let top = img[y0 * w + x0] * (1.0 - fx) + img[y0 * w + x1] * fx;
    let bot = img[y1 * w + x0] * (1.0 - fx) + img[y1 * w + x1] * fx;
    top * (1.0 - fy) + bot * fy
}

/// Profile along direction `dir` (unit vector, pixel coordinates) through `center`, spanning
/// the whole image in 1-pixel steps, averaged over ±`half` pixels across the line. The
/// position axis is in µm from `center`.
fn sample_line(img: &[f32], w: usize, h: usize, pitch: (f64, f64), center: (f64, f64), dir: (f64, f64), half: f64) -> (ProfileLine, Vec<f32>) {
    let (cx, cy) = center;
    let (dx, dy) = dir;
    // Parameter range that keeps the line inside [0, w-1] × [0, h-1].
    let range = |c: f64, d: f64, hi: f64| -> (f64, f64) {
        if d.abs() < 1e-12 {
            (f64::NEG_INFINITY, f64::INFINITY)
        } else {
            let (a, b) = ((0.0 - c) / d, (hi - c) / d);
            (a.min(b), a.max(b))
        }
    };
    let (ax, bx) = range(cx, dx, (w - 1) as f64);
    let (ay, by) = range(cy, dy, (h - 1) as f64);
    let (t0, t1) = (ax.max(ay).ceil(), bx.min(by).floor());
    let (t0, n) = if t1 >= t0 { (t0, (t1 - t0) as usize + 1) } else { (0.0, 1) };
    let step_um = ((dx * pitch.0).powi(2) + (dy * pitch.1).powi(2)).sqrt();
    let k = half.round() as i64;
    let prof = (0..n)
        .map(|i| {
            let t = t0 + i as f64;
            let (x, y) = (cx + t * dx, cy + t * dy);
            let mut acc = 0.0f32;
            for j in -k..=k {
                acc += bilinear(img, w, h, x - j as f64 * dy, y + j as f64 * dx);
            }
            acc / (2 * k + 1) as f32
        })
        .collect();
    (ProfileLine { x0: cx + t0 * dx, y0: cy + t0 * dy, dx, dy, step_um, origin_um: t0 * step_um, len: n }, prof)
}

pub fn analyze(f: &Frame, s: &AnalysisSettings) -> Metrics {
    let (w, h) = (f.width, f.height);
    let (sx, sy) = f.pixel_um;
    let mut m = Metrics { width: w, height: h, pixel_um: f.pixel_um, ..Default::default() };

    // Raw statistics.
    let (mut pk, mut pki, mut mn, mut sum, mut sat) = (0u16, 0usize, u16::MAX, 0u64, 0usize);
    for (i, &v) in f.data.iter().enumerate() {
        if v > pk {
            pk = v;
            pki = i;
        }
        mn = mn.min(v);
        sum += v as u64;
        if v >= 65000 {
            sat += 1;
        }
    }
    m.peak = pk as f64;
    m.peak_px = (pki % w, pki / w);
    m.min = mn as f64;
    m.mean = sum as f64 / (w * h) as f64;
    m.saturated_pixels = sat;
    let (mut psm, mut msm) = (0u32, u32::MAX);
    for y in 1..h.saturating_sub(1) {
        let r = &f.data[y * w..(y + 1) * w];
        for x in 1..w.saturating_sub(1) {
            let v = (r[x - 1] as u32 + 2 * r[x] as u32 + r[x + 1] as u32) >> 2;
            psm = psm.max(v);
            msm = msm.min(v);
        }
    }
    m.peak_smoothed = psm as f64;
    m.min_smoothed = if msm == u32::MAX { m.min } else { msm as f64 };

    // Baseline-subtracted working image.
    m.baseline = if s.auto_baseline { border_mean(f) } else { 0.0 };
    let base = m.baseline as f32;
    let clip = ((m.peak - m.baseline) * s.noise_clip).max(0.0) as f32;
    let img: Vec<f32> = f
        .data
        .iter()
        .map(|&v| {
            let d = v as f32 - base;
            if s.noise_clip > 0.0 && d < clip {
                0.0
            } else {
                d
            }
        })
        .collect();

    // Seed from pixels above 10 % of peak: on a small beam in a large frame the summed
    // baseline noise otherwise outweighs the beam and drags the estimate to the center.
    let seed_t = ((m.peak - m.baseline) * 0.1).max(1.0) as f32;
    let pos: Vec<f32> = img.iter().map(|&v| if v >= seed_t { v } else { 0.0 }).collect();
    let seed = moments(&pos, w, h, sx, sy, None);
    let mut mom = if s.iso_aperture { None } else { moments(&img, w, h, sx, sy, None).or(seed.as_ref().map(|m| Moments { ..*m })) };
    if s.iso_aperture {
        if let Some(mut cur) = seed {
            for it in 0..30 {
                let (_, _, dmaj, dmin, phi) = iso_widths(&cur);
                let ap = (cur.cx, cur.cy, 1.5 * dmaj / sx.min(sy), 1.5 * dmin.max(sx) / sx.min(sy), phi);
                m.aperture_iterations = it + 1;
                m.aperture = Some(Aperture { cx: ap.0, cy: ap.1, half_major_px: ap.2, half_minor_px: ap.3, angle_rad: ap.4 });
                match moments(&img, w, h, sx, sy, Some(ap)) {
                    Some(next) => {
                        let (_, _, nmaj, nmin, _) = iso_widths(&next);
                        // Pixel-quantized aperture edges make the widths wobble by a hair on
                        // small beams; accept 0.2 % or 0.05 px, whichever is looser.
                        let tol = |d: f64| (2e-3 * d).max(0.05 * sx.min(sy));
                        let conv = (nmaj - dmaj).abs() <= tol(dmaj)
                            && (nmin - dmin).abs() <= tol(dmin)
                            && (next.cx - cur.cx).abs() < 0.05
                            && (next.cy - cur.cy).abs() < 0.05;
                        cur = next;
                        if conv {
                            break;
                        }
                    }
                    None => break,
                }
            }
            mom = Some(cur);
        }
    }
    if let Some(mo) = &mom {
        let (dx, dy, dmaj, dmin, phi) = iso_widths(mo);
        m.total = mo.total;
        m.centroid_px = (mo.cx, mo.cy);
        m.centroid_um = (mo.cx * sx, mo.cy * sy);
        m.d4s_x_um = dx;
        m.d4s_y_um = dy;
        m.d4s_major_um = dmaj;
        m.d4s_minor_um = dmin;
        // Image rows grow downward; report the angle in the usual math orientation.
        m.orientation_deg = -phi.to_degrees();
        m.ellipticity = if dmaj > 0.0 { dmin / dmaj } else { 0.0 };
        m.effective_diameter_um = (dmaj * dmin).sqrt();
    }

    // DataRay-compatible metrics (FUN_180195090): 3-pixel border excluded, threshold at clip
    // level 1 of (peak − baseline).
    {
        let t = ((m.peak - m.baseline) * s.clip_levels[0]) as f32;
        let (mut n, mut sv, mut sx_, mut sy_) = (0usize, 0.0f64, 0.0f64, 0.0f64);
        for y in 3..h.saturating_sub(3) {
            let row = &img[y * w..(y + 1) * w];
            for x in 3..w.saturating_sub(3) {
                let v = row[x];
                if v > t {
                    n += 1;
                }
                if v >= t {
                    sv += v as f64;
                    sx_ += v as f64 * x as f64;
                    sy_ += v as f64 * y as f64;
                }
            }
        }
        m.area_diameter_um = 2.0 * (sx * sy * n as f64 / std::f64::consts::PI).sqrt();
        if sv > 0.0 {
            m.dataray_centroid_px = (sx_ / sv, sy_ / sv);
        }
    }

    // Line profiles: along X/Y through the centroid (or peak), or along the principal axes.
    let band = s.profile_band.max(1);
    let phi_major = mom.as_ref().map(|mo| iso_widths(mo).4);
    if let (true, Some(phi)) = (s.principal_axes, phi_major) {
        let (cx, cy) = m.centroid_px;
        let (c, sn) = (phi.cos(), phi.sin());
        m.principal_axes = true;
        m.profile_row = cy.round().clamp(0.0, (h - 1) as f64) as usize;
        m.profile_col = cx.round().clamp(0.0, (w - 1) as f64) as usize;
        let half = (band / 2) as f64;
        let (line_u, prof_u) = sample_line(&img, w, h, (sx, sy), (cx, cy), (c, sn), half);
        let (line_v, prof_v) = sample_line(&img, w, h, (sx, sy), (cx, cy), (-sn, c), half);
        m.profile_line_x = line_u;
        m.profile_line_y = line_v;
        m.profile_x = prof_u;
        m.profile_y = prof_v;
    } else {
        let (px, py) = if s.profiles_through_centroid && mom.is_some() {
            (m.centroid_px.0.round().max(0.0) as usize, m.centroid_px.1.round().max(0.0) as usize)
        } else {
            m.peak_px
        };
        let (px, py) = (px.min(w - 1), py.min(h - 1));
        m.profile_row = py;
        m.profile_col = px;
        let half = band / 2;
        let rows: Vec<usize> = (py.saturating_sub(half)..=(py + half).min(h - 1)).collect();
        let cols: Vec<usize> = (px.saturating_sub(half)..=(px + half).min(w - 1)).collect();
        m.profile_x = (0..w)
            .map(|x| rows.iter().map(|&y| img[y * w + x]).sum::<f32>() / rows.len() as f32)
            .collect();
        m.profile_y = (0..h)
            .map(|y| cols.iter().map(|&x| img[y * w + x]).sum::<f32>() / cols.len() as f32)
            .collect();
        m.profile_line_x = ProfileLine { x0: 0.0, y0: py as f64, dx: 1.0, dy: 0.0, step_um: sx, origin_um: 0.0, len: w };
        m.profile_line_y = ProfileLine { x0: px as f64, y0: 0.0, dx: 0.0, dy: 1.0, step_um: sy, origin_um: 0.0, len: h };
    }

    let (lx, ly) = (m.profile_line_x.clone(), m.profile_line_y.clone());
    m.clip = s
        .clip_levels
        .iter()
        .map(|&lv| ClipWidth {
            level: lv,
            x_um: clip_width(&m.profile_x, lv) * lx.step_um,
            y_um: clip_width(&m.profile_y, lv) * ly.step_um,
        })
        .collect();
    let gf = |p: &[f32], l: &ProfileLine| {
        fit_gaussian(p)
            .map(|(a, c, wd, b, r2)| GaussFit {
                diameter_um: 2.0 * wd * l.step_um,
                center_um: l.origin_um + c * l.step_um,
                amplitude: a,
                offset: b,
                r2,
            })
            .unwrap_or_default()
    };
    m.gauss_x = gf(&m.profile_x, &lx);
    m.gauss_y = gf(&m.profile_y, &ly);
    m
}

// ---------------------------------------------------------------------------------------
// M² (ISO 11146-1 hyperbolic fit)

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CausticPoint {
    pub z_mm: f64,
    pub dx_um: f64,
    pub dy_um: f64,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AxisFit {
    pub m2: f64,
    pub waist_um: f64,
    pub waist_z_mm: f64,
    pub rayleigh_mm: f64,
    /// Full far-field divergence angle, mrad.
    pub divergence_mrad: f64,
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub r2: f64,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct M2Result {
    pub x: AxisFit,
    pub y: AxisFit,
    pub warnings: Vec<String>,
}

/// Least-squares fit of d²(z) = A + Bz + Cz² (d in mm, z in mm).
fn fit_axis(z: &[f64], d_um: &[f64], lambda_nm: f64) -> Option<AxisFit> {
    let n = z.len();
    if n < 3 {
        return None;
    }
    let mut ata = [[0.0f64; 3]; 3];
    let mut atb = [0.0f64; 3];
    for i in 0..n {
        let d = d_um[i] * 1e-3;
        let row = [1.0, z[i], z[i] * z[i]];
        for a in 0..3 {
            atb[a] += row[a] * d * d;
            for b in 0..3 {
                ata[a][b] += row[a] * row[b];
            }
        }
    }
    let [a, b, c] = solve3(ata, atb)?;
    let lambda_mm = lambda_nm * 1e-6;
    let disc = 4.0 * a * c - b * b;
    if c <= 0.0 || disc <= 0.0 {
        return None;
    }
    let z0 = -b / (2.0 * c);
    let d0 = disc.sqrt() / (2.0 * c.sqrt());
    let theta = c.sqrt(); // full divergence, rad
    let m2 = std::f64::consts::PI / (8.0 * lambda_mm) * disc.sqrt();
    let mean = d_um.iter().map(|d| (d * 1e-3).powi(2)).sum::<f64>() / n as f64;
    let (mut ssr, mut sst) = (0.0, 0.0);
    for i in 0..n {
        let y = (d_um[i] * 1e-3).powi(2);
        let f = a + b * z[i] + c * z[i] * z[i];
        ssr += (y - f).powi(2);
        sst += (y - mean).powi(2);
    }
    Some(AxisFit {
        m2,
        waist_um: d0 * 1e3,
        waist_z_mm: z0,
        rayleigh_mm: d0 / theta,
        divergence_mrad: theta * 1e3,
        a,
        b,
        c,
        r2: if sst > 0.0 { 1.0 - ssr / sst } else { 0.0 },
    })
}

fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let piv = (col..3).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap())?;
        if a[piv][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in 0..3 {
            if r != col {
                let f = a[r][col] / a[col][col];
                for c in col..3 {
                    a[r][c] -= f * a[col][c];
                }
                b[r] -= f * b[col];
            }
        }
    }
    Some([b[0] / a[0][0], b[1] / a[1][1], b[2] / a[2][2]])
}

pub fn fit_m2(points: &[CausticPoint], lambda_nm: f64) -> M2Result {
    let pts: Vec<&CausticPoint> = points.iter().filter(|p| p.enabled).collect();
    let z: Vec<f64> = pts.iter().map(|p| p.z_mm).collect();
    let dx: Vec<f64> = pts.iter().map(|p| p.dx_um).collect();
    let dy: Vec<f64> = pts.iter().map(|p| p.dy_um).collect();
    let mut r = M2Result::default();
    if pts.len() < 3 {
        r.warnings.push("Need at least 3 points (ISO 11146 asks for 10).".into());
        return r;
    }
    match fit_axis(&z, &dx, lambda_nm) {
        Some(f) => r.x = f,
        None => r.warnings.push("X fit failed: data is not a caustic (check z positions).".into()),
    }
    match fit_axis(&z, &dy, lambda_nm) {
        Some(f) => r.y = f,
        None => r.warnings.push("Y fit failed: data is not a caustic (check z positions).".into()),
    }
    if pts.len() < 10 {
        r.warnings.push(format!("{} points; ISO 11146 recommends at least 10.", pts.len()));
    }
    for (name, f) in [("X", &r.x), ("Y", &r.y)] {
        if f.rayleigh_mm > 0.0 {
            let near = z.iter().filter(|&&zz| (zz - f.waist_z_mm).abs() <= f.rayleigh_mm).count();
            let far = z.iter().filter(|&&zz| (zz - f.waist_z_mm).abs() >= 2.0 * f.rayleigh_mm).count();
            if near < pts.len() / 2 || far < pts.len() / 2 {
                r.warnings.push(format!(
                    "{name}: ISO wants ≥ half the points within one Rayleigh length of the waist and ≥ half beyond two ({near} near, {far} far)."
                ));
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn gaussian_frame(w: usize, h: usize, cx: f64, cy: f64, wx: f64, wy: f64, bg: f64) -> Frame {
        let mut d = vec![0u16; w * h];
        for y in 0..h {
            for x in 0..w {
                let v = bg
                    + 50000.0
                        * (-2.0 * ((x as f64 - cx).powi(2) / (wx * wx) + (y as f64 - cy).powi(2) / (wy * wy))).exp();
                d[y * w + x] = v as u16;
            }
        }
        Frame { width: w, height: h, pixel_um: (5.2, 5.2), data: Arc::new(d), exposure_ms: 1.0, gain: 1.0, timestamp_ms: 0.0, index: 0 }
    }

    #[test]
    fn d4sigma_matches_1e2_for_gaussian() {
        // For a TEM00 beam, D4σ equals the 1/e² diameter 2w.
        let f = gaussian_frame(400, 300, 210.3, 140.7, 40.0, 25.0, 800.0);
        let m = analyze(&f, &AnalysisSettings::default());
        assert!((m.centroid_px.0 - 210.3).abs() < 0.05, "{:?}", m.centroid_px);
        assert!((m.centroid_px.1 - 140.7).abs() < 0.05);
        assert!((m.d4s_x_um / 5.2 - 80.0).abs() < 0.8, "dx {}", m.d4s_x_um / 5.2);
        assert!((m.d4s_y_um / 5.2 - 50.0).abs() < 0.5, "dy {}", m.d4s_y_um / 5.2);
        assert!((m.gauss_x.diameter_um / 5.2 - 80.0).abs() < 0.5, "gx {}", m.gauss_x.diameter_um / 5.2);
        assert!((m.clip[0].x_um / 5.2 - 80.0).abs() < 1.0, "clip {}", m.clip[0].x_um / 5.2);
        assert!(m.orientation_deg.abs() < 0.5);
    }

    /// Rotated elliptical beam: principal-axis profiles must recover the true 1/e² diameters
    /// along the axes; X/Y profiles can't.
    #[test]
    fn principal_axis_profiles_on_rotated_beam() {
        let (w, h, cx, cy) = (500usize, 400usize, 251.3, 198.6);
        let (wu, wv, phi) = (60.0f64, 30.0f64, 0.6f64); // 1/e² radii in px, major-axis angle (y down)
        let mut d = vec![0u16; w * h];
        for y in 0..h {
            for x in 0..w {
                let (ddx, ddy) = (x as f64 - cx, y as f64 - cy);
                let u = ddx * phi.cos() + ddy * phi.sin();
                let v = -ddx * phi.sin() + ddy * phi.cos();
                d[y * w + x] = (500.0 + 50000.0 * (-2.0 * (u * u / (wu * wu) + v * v / (wv * wv))).exp()) as u16;
            }
        }
        let f = Frame { width: w, height: h, pixel_um: (5.2, 5.2), data: Arc::new(d), exposure_ms: 1.0, gain: 1.0, timestamp_ms: 0.0, index: 0 };
        let s = AnalysisSettings { principal_axes: true, ..Default::default() };
        let m = analyze(&f, &s);
        assert!(m.principal_axes);
        assert!((m.orientation_deg + phi.to_degrees()).abs() < 0.2, "orientation {}", m.orientation_deg);
        assert!((m.gauss_x.diameter_um / 5.2 - 2.0 * wu).abs() < 1.0, "u gauss {}", m.gauss_x.diameter_um / 5.2);
        assert!((m.gauss_y.diameter_um / 5.2 - 2.0 * wv).abs() < 0.5, "v gauss {}", m.gauss_y.diameter_um / 5.2);
        assert!((m.clip[0].x_um / 5.2 - 2.0 * wu).abs() < 1.5, "u clip {}", m.clip[0].x_um / 5.2);
        assert!((m.clip[0].y_um / 5.2 - 2.0 * wv).abs() < 1.0, "v clip {}", m.clip[0].y_um / 5.2);
        // The Gaussian centre is at the centroid, i.e. 0 on the u/v axes.
        assert!(m.gauss_x.center_um.abs() < 1.0 && m.gauss_y.center_um.abs() < 1.0);
        // The ISO area is 3× the D4σ widths (half-size 1.5 d).
        let ap = m.aperture.unwrap();
        assert!((ap.half_major_px * 5.2 - 1.5 * m.d4s_major_um).abs() < 1e-6);
        // Same frame along X/Y: the widths mix both axes and differ from 2w.
        let mxy = analyze(&f, &AnalysisSettings::default());
        assert!((mxy.gauss_x.diameter_um / 5.2 - 2.0 * wu).abs() > 5.0);
    }

    #[test]
    fn small_beam_in_large_frame() {
        // Tiny beam, noisy-ish flat baseline slightly below the border mean.
        let mut f = gaussian_frame(640, 512, 330.5, 257.2, 8.0, 8.0, 800.0);
        let d = Arc::make_mut(&mut f.data);
        for (i, v) in d.iter_mut().enumerate() {
            if i % 7 == 0 {
                *v = v.saturating_sub(30);
            }
        }
        let m = analyze(&f, &AnalysisSettings::default());
        assert!((m.centroid_px.0 - 330.5).abs() < 0.1, "{:?}", m.centroid_px);
        assert!((m.d4s_x_um / 5.2 - 16.0).abs() < 1.0, "dx {}", m.d4s_x_um / 5.2);
        assert!(m.aperture_iterations > 0);
    }

    #[test]
    fn m2_of_ideal_gaussian_is_one() {
        let lambda_nm = 1064.0;
        let w0_mm = 0.1;
        let zr = std::f64::consts::PI * w0_mm * w0_mm / (lambda_nm * 1e-6);
        let pts: Vec<CausticPoint> = (-10..=10)
            .map(|i| {
                let z = i as f64 * zr / 3.0 + 5.0;
                let d = 2.0 * w0_mm * (1.0 + ((z - 5.0) / zr).powi(2)).sqrt() * 1e3;
                CausticPoint { z_mm: z, dx_um: d, dy_um: d * 1.5, enabled: true }
            })
            .collect();
        let r = fit_m2(&pts, lambda_nm);
        assert!((r.x.m2 - 1.0).abs() < 1e-6, "{}", r.x.m2);
        assert!((r.x.waist_um - 200.0).abs() < 1e-3);
        assert!((r.x.waist_z_mm - 5.0).abs() < 1e-6);
        assert!((r.y.m2 - 1.5 * 1.5).abs() < 1e-6, "{}", r.y.m2);
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use std::sync::Arc;

    fn frame(w: usize, h: usize, d: Vec<u16>, px: f64) -> Frame {
        Frame { width: w, height: h, pixel_um: (px, px), data: Arc::new(d), exposure_ms: 1.0, gain: 1.0, timestamp_ms: 0.0, index: 0 }
    }

    /// Scales a real capture down to mimic short exposures, plus synthetic edge cases.
    #[test]
    fn analyze_never_panics() {
        let mut rng = 12345u64;
        let mut rnd = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng % 10_000) as f64 / 10_000.0
        };
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../capture_again.wcf");
        let mut cases: Vec<(String, Frame)> = Vec::new();
        if let Ok(b) = std::fs::read(p) {
            let f = crate::wcf::parse(&b).unwrap().frames.remove(0);
            for k in [1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01, 0.005, 0.002, 0.0] {
                let d: Vec<u16> = f.data.iter().map(|&v| ((v as f64 - 755.0) * k + 755.0 + (rnd() - 0.5) * 256.0).clamp(0.0, 65535.0) as u16 & 0xffc0).collect();
                cases.push((format!("capture x{k}"), frame(f.width, f.height, d, 5.2)));
            }
        }
        for (w, h) in [(640, 512), (16, 16), (7, 5), (1280, 1024)] {
            cases.push((format!("zeros {w}x{h}"), frame(w, h, vec![0; w * h], 10.4)));
            cases.push((format!("flat {w}x{h}"), frame(w, h, vec![768; w * h], 10.4)));
            cases.push((format!("sat {w}x{h}"), frame(w, h, vec![65472; w * h], 10.4)));
            let noise: Vec<u16> = (0..w * h).map(|_| ((rnd() * 40.0) as u16 + 700) & 0xffc0).collect();
            cases.push((format!("noise {w}x{h}"), frame(w, h, noise, 10.4)));
            let mut hot = vec![768u16; w * h];
            hot[w * h / 3] = 65472;
            cases.push((format!("hot {w}x{h}"), frame(w, h, hot, 10.4)));
        }
        for (name, f) in cases {
            for s in [
                AnalysisSettings::default(),
                AnalysisSettings { iso_aperture: false, auto_baseline: false, noise_clip: 0.05, ..Default::default() },
                AnalysisSettings { principal_axes: true, profile_band: 5, ..Default::default() },
            ] {
                let r = std::panic::catch_unwind(|| analyze(&f, &s));
                match r {
                    Ok(m) => println!("{name:24} ok  peak {:6.0}  d4s {:9.1} x {:9.1}  gauss {:9.1}", m.peak, m.d4s_x_um, m.d4s_y_um, m.gauss_x.diameter_um),
                    Err(_) => panic!("analyze panicked on {name} with {s:?}"),
                }
            }
        }
    }
}
