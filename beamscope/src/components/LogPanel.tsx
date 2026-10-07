import { useEffect, useMemo, useRef, useState } from "react";
import type uPlot from "uplot";
import { save } from "@tauri-apps/plugin-dialog";
import { api, LogRow, Status } from "../api";
import { axisStyle, Plot } from "./Plot";

const METRICS: { key: keyof LogRow; label: string; color: string; unit: string }[] = [
  { key: "d4sXUm", label: "D4σ X", color: "#5cc8ff", unit: "µm" },
  { key: "d4sYUm", label: "D4σ Y", color: "#ffb454", unit: "µm" },
  { key: "cxUm", label: "Centroid X", color: "#7ee787", unit: "µm" },
  { key: "cyUm", label: "Centroid Y", color: "#f778ba", unit: "µm" },
  { key: "gaussXUm", label: "Gauss X", color: "#a5d6ff", unit: "µm" },
  { key: "gaussYUm", label: "Gauss Y", color: "#ffd8a8", unit: "µm" },
  { key: "clip1XUm", label: "Clip1 X", color: "#79c0ff", unit: "µm" },
  { key: "clip1YUm", label: "Clip1 Y", color: "#ffa657", unit: "µm" },
  { key: "ellipticity", label: "Ellipticity", color: "#d2a8ff", unit: "" },
  { key: "orientationDeg", label: "Orientation", color: "#ff7b72", unit: "°" },
  { key: "peak", label: "Peak", color: "#e3b341", unit: "cts" },
  { key: "total", label: "Total", color: "#56d4dd", unit: "" },
];

function stats(v: number[]) {
  if (!v.length) return null;
  const n = v.length;
  const mean = v.reduce((a, b) => a + b, 0) / n;
  const sd = Math.sqrt(v.reduce((a, b) => a + (b - mean) ** 2, 0) / Math.max(1, n - 1));
  return { mean, sd, min: Math.min(...v), max: Math.max(...v) };
}

export function LogPanel({ status, notify }: { status: Status | null; notify: (msg: string, err?: boolean) => void }) {
  const [rows, setRows] = useState<LogRow[]>([]);
  const [interval, setIntervalMs] = useState(0);
  const [selected, setSelected] = useState<(keyof LogRow)[]>(["d4sXUm", "d4sYUm"]);
  const rowsRef = useRef<LogRow[]>([]);
  const logging = !!status?.logging;

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const more = await api.logRows(rowsRef.current.length);
        if (more.length && alive) {
          rowsRef.current = rowsRef.current.concat(more);
          setRows(rowsRef.current);
        }
      } catch {
        /* ignore */
      }
    };
    tick();
    const t = window.setInterval(tick, logging ? 300 : 2000);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [logging]);

  const options = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      legend: { show: true, live: false },
      // Every metric gets its own y scale so e.g. centroid (mm) and width (µm) both stay
      // readable; the first two get visible axes (left / right) in their colors.
      axes: [
        { ...axisStyle, label: "time (s)", labelSize: 14, size: 34 },
        ...selected.slice(0, 2).map((k, i) => {
          const m = METRICS.find((x) => x.key === k)!;
          return { ...axisStyle, scale: String(k), side: i === 0 ? 3 : 1, size: 60, stroke: m.color, grid: { show: i === 0, ...axisStyle.grid } };
        }),
      ],
      scales: { x: { time: false }, ...Object.fromEntries(selected.map((k) => [String(k), { auto: true }])) },
      series: [{ label: "t" }, ...selected.map((k) => {
        const m = METRICS.find((x) => x.key === k)!;
        return { label: `${m.label}${m.unit ? ` (${m.unit})` : ""}`, scale: String(k), stroke: m.color, width: 1.5, points: { show: false } };
      })],
    }),
    [selected],
  );

  const data = useMemo<uPlot.AlignedData>(() => {
    const t0 = rows[0]?.tMs ?? 0;
    return [rows.map((r) => (r.tMs - t0) / 1000), ...selected.map((k) => rows.map((r) => r[k] as number))] as uPlot.AlignedData;
  }, [rows, selected]);

  const toggle = (k: keyof LogRow) => setSelected((s) => (s.includes(k) ? s.filter((x) => x !== k) : [...s, k]));

  return (
    <div className="log-panel">
      <div className="log-side">
        <div className="btn-row">
          {logging ? (
            <button className="danger" onClick={() => api.logStop()}>
              Stop logging
            </button>
          ) : (
            <button className="primary" disabled={!status?.running} onClick={() => api.logStart(interval)}>
              Start logging
            </button>
          )}
          <button
            onClick={async () => {
              await api.logClear();
              rowsRef.current = [];
              setRows([]);
            }}
          >
            Clear
          </button>
          <button
            disabled={!rows.length}
            onClick={async () => {
              const p = await save({ filters: [{ name: "CSV", extensions: ["csv"] }], defaultPath: "beam-log.csv" });
              if (p) {
                try {
                  await api.logExport(p);
                  notify(`Saved ${rows.length} rows to ${p}`);
                } catch (e) {
                  notify(String(e), true);
                }
              }
            }}
          >
            Export CSV
          </button>
        </div>
        <label className="field">
          <span>Interval</span>
          <span className="input-wrap">
            <input type="number" min={0} step={100} value={interval} disabled={logging} onChange={(e) => setIntervalMs(Math.max(0, +e.target.value))} />
            <span className="suffix">ms (0 = every frame)</span>
          </span>
        </label>
        <div className="chips">
          {METRICS.map((m) => (
            <button key={m.key} className={`chip ${selected.includes(m.key) ? "on" : ""}`} style={{ ["--c" as string]: m.color }} onClick={() => toggle(m.key)}>
              {m.label}
            </button>
          ))}
        </div>
        <table className="metrics stats">
          <thead>
            <tr>
              <th>{rows.length} rows</th>
              <th>mean</th>
              <th>σ</th>
              <th>min</th>
              <th>max</th>
            </tr>
          </thead>
          <tbody>
            {selected.map((k) => {
              const s = stats(rows.map((r) => r[k] as number).filter((v) => isFinite(v)));
              const m = METRICS.find((x) => x.key === k)!;
              const d = m.key === "ellipticity" ? 4 : 2;
              return (
                <tr key={k}>
                  <th style={{ color: m.color }}>{m.label}</th>
                  <td className="mono">{s ? s.mean.toFixed(d) : "—"}</td>
                  <td className="mono">{s ? s.sd.toFixed(d) : "—"}</td>
                  <td className="mono">{s ? s.min.toFixed(d) : "—"}</td>
                  <td className="mono">{s ? s.max.toFixed(d) : "—"}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <Plot options={options} data={data} optionsKey={selected.join(",")} className="log-plot" />
    </div>
  );
}
