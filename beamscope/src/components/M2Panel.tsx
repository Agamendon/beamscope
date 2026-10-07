import { useEffect, useMemo, useState } from "react";
import type uPlot from "uplot";
import { save } from "@tauri-apps/plugin-dialog";
import { api, AxisFit, CausticPoint, Config, FramePacket, M2Result } from "../api";
import { fmt } from "../format";
import { axisStyle, Plot } from "./Plot";

type Method = "d4s" | "gauss" | "clip1";

const STORE = "beamscope.m2.points";

function widths(frame: FramePacket, method: Method): [number, number] {
  const m = frame.metrics;
  if (method === "gauss") return [m.gaussX.diameterUm, m.gaussY.diameterUm];
  if (method === "clip1") return [m.clip[0]?.xUm ?? NaN, m.clip[0]?.yUm ?? NaN];
  return [m.d4sXUm, m.d4sYUm];
}

function curve(f: AxisFit, z: number) {
  return Math.sqrt(Math.max(0, f.a + f.b * z + f.c * z * z)) * 1000;
}

function FitCard({ name, f, color }: { name: string; f: AxisFit; color: string }) {
  const ok = f.m2 > 0;
  return (
    <div className="fit-card" style={{ ["--c" as string]: color }}>
      <div className="fit-title">{name}</div>
      <div className="fit-m2 mono">M² {ok ? fmt(f.m2, 3) : "—"}</div>
      <dl className="kv">
        <dt>Waist ⌀</dt>
        <dd className="mono">{ok ? `${fmt(f.waistUm, 1)} µm` : "—"}</dd>
        <dt>Waist z</dt>
        <dd className="mono">{ok ? `${fmt(f.waistZMm, 3)} mm` : "—"}</dd>
        <dt>Rayleigh</dt>
        <dd className="mono">{ok ? `${fmt(f.rayleighMm, 3)} mm` : "—"}</dd>
        <dt>Divergence</dt>
        <dd className="mono">{ok ? `${fmt(f.divergenceMrad, 3)} mrad` : "—"}</dd>
        <dt>Fit r²</dt>
        <dd className="mono">{ok ? fmt(f.r2, 5) : "—"}</dd>
      </dl>
    </div>
  );
}

export function M2Panel({ frame, config, patch, notify }: { frame: FramePacket | null; config: Config | null; patch: (p: Record<string, unknown>) => void; notify: (m: string, e?: boolean) => void }) {
  const [points, setPoints] = useState<CausticPoint[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(STORE) ?? "[]");
    } catch {
      return [];
    }
  });
  const [z, setZ] = useState(0);
  const [step, setStep] = useState(1);
  const [method, setMethod] = useState<Method>("d4s");
  const [result, setResult] = useState<M2Result | null>(null);
  const lambda = config?.wavelengthNm ?? 1064;

  useEffect(() => {
    try {
      localStorage.setItem(STORE, JSON.stringify(points));
    } catch {
      /* ignore */
    }
    if (points.filter((p) => p.enabled).length >= 3) api.fitM2(points, lambda).then(setResult);
    else setResult(null);
  }, [points, lambda]);

  const add = () => {
    if (!frame) return;
    const [dx, dy] = widths(frame, method);
    setPoints((p) => [...p, { zMm: z, dxUm: dx, dyUm: dy, enabled: true }].sort((a, b) => a.zMm - b.zMm));
    setZ(+(z + step).toFixed(6));
  };

  const options = useMemo<Omit<uPlot.Options, "width" | "height">>(
    () => ({
      legend: { show: true, live: false },
      scales: { x: { time: false } },
      axes: [
        { ...axisStyle, label: "z (mm)", labelSize: 14, size: 34 },
        { ...axisStyle, label: "diameter (µm)", labelSize: 14, size: 56 },
      ],
      series: [
        {},
        { label: "X", stroke: "#5cc8ff", width: 0, points: { show: true, size: 7, fill: "#5cc8ff" } },
        { label: "Y", stroke: "#ffb454", width: 0, points: { show: true, size: 7, fill: "#ffb454" } },
        { label: "X fit", stroke: "rgba(92,200,255,0.6)", width: 1.5, points: { show: false } },
        { label: "Y fit", stroke: "rgba(255,180,84,0.6)", width: 1.5, points: { show: false } },
      ],
    }),
    [],
  );

  const data = useMemo<uPlot.AlignedData>(() => {
    const en = points.filter((p) => p.enabled);
    if (!en.length) return [[], [], [], [], []];
    const lo = Math.min(...en.map((p) => p.zMm));
    const hi = Math.max(...en.map((p) => p.zMm));
    const pad = (hi - lo) * 0.1 || 1;
    const xs: number[] = [];
    for (let i = 0; i <= 120; i++) xs.push(lo - pad + ((hi - lo + 2 * pad) * i) / 120);
    const all = [...new Set([...xs, ...en.map((p) => p.zMm)])].sort((a, b) => a - b);
    const at = (zz: number, k: "dxUm" | "dyUm") => en.find((p) => p.zMm === zz)?.[k] ?? null;
    return [
      all,
      all.map((zz) => at(zz, "dxUm")),
      all.map((zz) => at(zz, "dyUm")),
      all.map((zz) => (result?.x.m2 ? curve(result.x, zz) : null)),
      all.map((zz) => (result?.y.m2 ? curve(result.y, zz) : null)),
    ] as uPlot.AlignedData;
  }, [points, result]);

  const exportCsv = async () => {
    const p = await save({ filters: [{ name: "CSV", extensions: ["csv"] }], defaultPath: "m2.csv" });
    if (!p) return;
    const lines = [`# wavelength_nm,${lambda}`, `# width_method,${method}`];
    if (result) {
      for (const [n, f] of [["x", result.x], ["y", result.y]] as const)
        lines.push(`# ${n}: M2=${f.m2},waist_um=${f.waistUm},waist_z_mm=${f.waistZMm},rayleigh_mm=${f.rayleighMm},divergence_mrad=${f.divergenceMrad},r2=${f.r2}`);
    }
    lines.push("z_mm,dx_um,dy_um,enabled");
    for (const q of points) lines.push(`${q.zMm},${q.dxUm},${q.dyUm},${q.enabled ? 1 : 0}`);
    await api.saveBytes(p, new TextEncoder().encode(lines.join("\n") + "\n"));
    notify(`Saved M² data to ${p}`);
  };

  return (
    <div className="m2-panel">
      <div className="m2-side">
        <div className="grid2">
          <label className="field">
            <span>z position</span>
            <span className="input-wrap">
              <input type="number" value={z} step={step} onChange={(e) => setZ(+e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
              <span className="suffix">mm</span>
            </span>
          </label>
          <label className="field">
            <span>Step</span>
            <span className="input-wrap">
              <input type="number" value={step} step={0.1} onChange={(e) => setStep(+e.target.value)} />
              <span className="suffix">mm</span>
            </span>
          </label>
        </div>
        <div className="grid2">
          <label className="field">
            <span>Width</span>
            <select value={method} onChange={(e) => setMethod(e.target.value as Method)}>
              <option value="d4s">D4σ (ISO 11146)</option>
              <option value="gauss">Gaussian 1/e²</option>
              <option value="clip1">Clip level 1</option>
            </select>
          </label>
          <label className="field">
            <span>λ</span>
            <span className="input-wrap">
              <input type="number" value={lambda} onChange={(e) => patch({ wavelengthNm: +e.target.value })} />
              <span className="suffix">nm</span>
            </span>
          </label>
        </div>
        <div className="btn-row">
          <button className="primary" disabled={!frame} onClick={add}>
            Add point at z = {z} mm
          </button>
          <button disabled={!points.length} onClick={() => confirm("Remove all points?") && setPoints([])}>
            Clear
          </button>
          <button disabled={!points.length} onClick={exportCsv}>
            Export
          </button>
        </div>
        <div className="points">
          <table className="metrics">
            <thead>
              <tr>
                <th />
                <th>z (mm)</th>
                <th>dX (µm)</th>
                <th>dY (µm)</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {points.map((p, i) => (
                <tr key={i} className={p.enabled ? "" : "off"}>
                  <td>
                    <input type="checkbox" checked={p.enabled} onChange={(e) => setPoints((ps) => ps.map((q, j) => (j === i ? { ...q, enabled: e.target.checked } : q)))} />
                  </td>
                  <td>
                    <input
                      className="cell"
                      type="number"
                      value={p.zMm}
                      onChange={(e) => setPoints((ps) => ps.map((q, j) => (j === i ? { ...q, zMm: +e.target.value } : q)))}
                    />
                  </td>
                  <td className="mono">{fmt(p.dxUm, 1)}</td>
                  <td className="mono">{fmt(p.dyUm, 1)}</td>
                  <td>
                    <button className="icon" title="Remove" onClick={() => setPoints((ps) => ps.filter((_, j) => j !== i))}>
                      ×
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
      <div className="m2-main">
        <div className="fit-cards">
          <FitCard name="X" f={result?.x ?? ({} as AxisFit)} color="#5cc8ff" />
          <FitCard name="Y" f={result?.y ?? ({} as AxisFit)} color="#ffb454" />
          <div className="warnings">
            {result?.warnings.map((w, i) => (
              <div key={i} className="warning">
                {w}
              </div>
            ))}
            {!result && <div className="hint">Add at least 3 points through the waist.</div>}
          </div>
        </div>
        <Plot options={options} data={data} optionsKey="m2" className="m2-plot" />
      </div>
    </div>
  );
}
