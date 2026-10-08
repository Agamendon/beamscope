import { invoke } from "@tauri-apps/api/core";

export type Binning = "full" | "bin2" | "bin4";

export interface Roi {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface AnalysisSettings {
  autoBaseline: boolean;
  isoAperture: boolean;
  noiseClip: number;
  clipLevels: [number, number];
  profilesThroughCentroid: boolean;
  profileBand: number;
  principalAxes: boolean;
}

/** Camera mounting: mirror (in sensor coordinates) first, then rotate clockwise. */
export interface Orientation {
  rotate: number;
  flipH: boolean;
  flipV: boolean;
}

export interface Config {
  exposureMs: number;
  autoExposure: boolean;
  gainStep: number;
  blackLevel: number;
  autoBlack: boolean;
  binning: Binning;
  roi: Roi | null;
  average: number;
  subtractBackground: boolean;
  analysis: AnalysisSettings;
  wavelengthNm: number;
  orientation: Orientation;
}

export interface CameraInfo {
  model: string;
  serial: string;
  sensor: string;
  sensorWidth: number;
  sensorHeight: number;
  pixelUm: number;
  firmware: string;
  link: string;
  exposureMinMs: number;
  exposureMaxMs: number;
  gainSteps: number;
  blackLevelMax: number;
  simulated: boolean;
}

export interface Status {
  connected: boolean;
  running: boolean;
  source: string;
  camera: CameraInfo | null;
  fps: number;
  frames: number;
  dropped: number;
  lastError: string | null;
  hasBackground: boolean;
  logging: boolean;
  logRows: number;
  config: Config;
}

export interface GaussFit {
  diameterUm: number;
  centerUm: number;
  amplitude: number;
  offset: number;
  r2: number;
}

/** Sample i is at pixel (x0 + i·dx, y0 + i·dy) and at originUm + i·stepUm on the plot axis. */
export interface ProfileLine {
  x0: number;
  y0: number;
  dx: number;
  dy: number;
  stepUm: number;
  originUm: number;
  len: number;
}

/** ISO 11146-3 integration area (rectangle, 3× D4σ) used for the final moments. */
export interface Aperture {
  cx: number;
  cy: number;
  halfMajorPx: number;
  halfMinorPx: number;
  angleRad: number;
}

export interface Metrics {
  width: number;
  height: number;
  pixelUm: [number, number];
  peak: number;
  peakSmoothed: number;
  peakPx: [number, number];
  min: number;
  mean: number;
  baseline: number;
  total: number;
  saturatedPixels: number;
  centroidPx: [number, number];
  centroidUm: [number, number];
  d4sXUm: number;
  d4sYUm: number;
  d4sMajorUm: number;
  d4sMinorUm: number;
  orientationDeg: number;
  ellipticity: number;
  effectiveDiameterUm: number;
  areaDiameterUm: number;
  datarayCentroidPx: [number, number];
  apertureIterations: number;
  aperture: Aperture | null;
  /** Profiles, clip and Gaussian widths are along u (major) / v (minor) instead of X / Y. */
  principalAxes: boolean;
  clip: { level: number; xUm: number; yUm: number }[];
  gaussX: GaussFit;
  gaussY: GaussFit;
  profileRow: number;
  profileCol: number;
  profileLineX: ProfileLine;
  profileLineY: ProfileLine;
}

export interface FramePacket {
  index: number;
  width: number;
  height: number;
  dispWidth: number;
  dispHeight: number;
  dispScale: number;
  pixelUm: [number, number];
  exposureMs: number;
  gain: number;
  timestampMs: number;
  fps: number;
  source: string;
  metrics: Metrics;
  image: Uint16Array;
  profileX: Float32Array;
  profileY: Float32Array;
}

export interface LogRow {
  tMs: number;
  frame: number;
  exposureMs: number;
  peak: number;
  total: number;
  cxUm: number;
  cyUm: number;
  d4sXUm: number;
  d4sYUm: number;
  d4sMajorUm: number;
  d4sMinorUm: number;
  orientationDeg: number;
  ellipticity: number;
  clip1XUm: number;
  clip1YUm: number;
  clip2XUm: number;
  clip2YUm: number;
  gaussXUm: number;
  gaussYUm: number;
}

export interface CausticPoint {
  zMm: number;
  dxUm: number;
  dyUm: number;
  enabled: boolean;
}

export interface AxisFit {
  m2: number;
  waistUm: number;
  waistZMm: number;
  rayleighMm: number;
  divergenceMrad: number;
  a: number;
  b: number;
  c: number;
  r2: number;
}

export interface M2Result {
  x: AxisFit;
  y: AxisFit;
  warnings: string[];
}

const dec = new TextDecoder();

export function parsePacket(buf: ArrayBuffer): FramePacket | null {
  if (buf.byteLength < 8) return null;
  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== 0x31465342) return null; // "BSF1"
  const jlen = dv.getUint32(4, true);
  const meta = JSON.parse(dec.decode(new Uint8Array(buf, 8, jlen)));
  let off = 8 + jlen;
  off = (off + 3) & ~3;
  const n = meta.dispWidth * meta.dispHeight;
  const image = new Uint16Array(buf, off, n);
  off += n * 2;
  off = (off + 3) & ~3;
  const nx = meta.profileXLen ?? meta.width;
  const ny = meta.profileYLen ?? meta.height;
  const profileX = new Float32Array(buf, off, nx);
  off += nx * 4;
  const profileY = new Float32Array(buf, off, ny);
  return { ...meta, image, profileX, profileY };
}

export const api = {
  connect: (simulate: boolean) => invoke<CameraInfo>("connect", { simulate }),
  disconnect: () => invoke<void>("disconnect"),
  status: () => invoke<Status>("status"),
  updateConfig: (patch: Partial<Config> | Record<string, unknown>) =>
    invoke<Config>("update_config", { patch }),
  start: () => invoke<void>("start"),
  stop: () => invoke<void>("stop"),
  latestFrame: async (since?: number) => {
    const buf = await invoke<ArrayBuffer>("latest_frame", { since: since ?? null });
    return parsePacket(buf);
  },
  captureBackground: (frames: number) => invoke<void>("capture_background", { frames }),
  clearBackground: () => invoke<void>("clear_background"),
  openWcf: (path: string) => invoke<string>("open_wcf", { path }),
  saveWcf: (path: string) => invoke<void>("save_wcf", { path }),
  exportImageCsv: (path: string) => invoke<void>("export_image_csv", { path }),
  exportProfilesCsv: (path: string) => invoke<void>("export_profiles_csv", { path }),
  logStart: (intervalMs: number) => invoke<void>("log_start", { intervalMs }),
  logStop: () => invoke<void>("log_stop"),
  logClear: () => invoke<void>("log_clear"),
  logRows: (from: number) => invoke<LogRow[]>("log_rows", { from }),
  logExport: (path: string) => invoke<void>("log_export", { path }),
  fitM2: (points: CausticPoint[], wavelengthNm: number) =>
    invoke<M2Result>("fit_m2", { points, wavelengthNm }),
  saveBytes: (path: string, data: Uint8Array) =>
    invoke<void>("save_bytes", { path, data: Array.from(data) }),
};
