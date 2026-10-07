export function fmtUm(v: number | undefined | null, digits = 1): string {
  if (v == null || !isFinite(v)) return "—";
  if (Math.abs(v) >= 10000) return `${(v / 1000).toFixed(2)} mm`;
  return `${v.toFixed(digits)} µm`;
}

export function fmt(v: number | undefined | null, digits = 2): string {
  if (v == null || !isFinite(v)) return "—";
  return v.toFixed(digits);
}

export function fmtMs(v: number | undefined | null): string {
  if (v == null || !isFinite(v)) return "—";
  if (v < 1) return `${(v * 1000).toFixed(0)} µs`;
  if (v < 100) return `${v.toFixed(2)} ms`;
  return `${v.toFixed(0)} ms`;
}
