import { useMemo } from "react";
import type uPlot from "uplot";
import type { FramePacket } from "../api";
import { axisStyle, Plot } from "./Plot";

interface Props {
  frame: FramePacket | null;
  /** First ("x": X or u) or second ("y": Y or v) profile. */
  axis: "x" | "y";
  /** Show only ±1.5 beam diameters around the centroid instead of the whole line. */
  zoom: boolean;
}

export function ProfilePlot({ frame, axis, zoom }: Props) {
  const principal = !!frame?.metrics.principalAxes;
  const name = principal ? (axis === "x" ? "u, major" : "v, minor") : axis;

  const options = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      legend: { show: false },
      cursor: { drag: { x: true, y: false } },
      scales: { x: { time: false } },
      axes: [
        { ...axisStyle, label: principal ? `${name} (µm from centroid)` : `${name} (µm)`, labelSize: 14, size: 34 },
        { ...axisStyle, size: 44, values: (_u, v) => v.map((t) => `${Math.round(t * 100)}%`) },
      ],
      series: [
        {},
        { stroke: axis === "x" ? "#5cc8ff" : "#ffb454", width: 1.5, fill: axis === "x" ? "rgba(92,200,255,0.08)" : "rgba(255,180,84,0.08)" },
        { stroke: "rgba(255,255,255,0.55)", width: 1, dash: [4, 4] },
      ],
    }),
    [axis, principal, name],
  );

  const data = useMemo<uPlot.AlignedData>(() => {
    if (!frame) return [[], [], []];
    const m = frame.metrics;
    const p = axis === "x" ? frame.profileX : frame.profileY;
    const line = axis === "x" ? m.profileLineX : m.profileLineY;
    const g = axis === "x" ? m.gaussX : m.gaussY;
    const pos = (i: number) => line.originUm + i * line.stepUm;
    // Plot as fraction of full scale above baseline so both profiles share units.
    const norm = 65535 - m.baseline;
    let lo = 0;
    let hi = p.length;
    if (zoom && line.stepUm > 0) {
      // Centroid position on this axis: 0 for u/v lines, absolute µm for X/Y lines.
      const c = principal ? 0 : axis === "x" ? m.centroidUm[0] : m.centroidUm[1];
      const d = Math.max(m.d4sMajorUm, g.diameterUm, 10 * line.stepUm);
      if (isFinite(c) && isFinite(d)) {
        lo = Math.max(0, Math.floor((c - 1.5 * d - line.originUm) / line.stepUm));
        hi = Math.min(p.length, Math.ceil((c + 1.5 * d - line.originUm) / line.stepUm) + 1);
        if (hi <= lo) [lo, hi] = [0, p.length];
      }
    }
    const n = hi - lo;
    const xs = new Float64Array(n);
    const ys = new Float64Array(n);
    const fit = new Float64Array(n);
    const w = g.diameterUm / 2;
    for (let k = 0; k < n; k++) {
      const i = lo + k;
      const x = pos(i);
      xs[k] = x;
      ys[k] = p[i] / norm;
      fit[k] = w > 0 ? (g.amplitude * Math.exp((-2 * (x - g.centerUm) ** 2) / (w * w)) + g.offset) / norm : NaN;
    }
    return [xs, ys, fit] as unknown as uPlot.AlignedData;
  }, [frame, axis, zoom, principal]);

  return <Plot options={options} data={data} optionsKey={`${axis}-${principal}`} className="profile-plot" />;
}
