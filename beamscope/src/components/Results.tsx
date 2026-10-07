import type { FramePacket } from "../api";
import { fmt, fmtMs, fmtUm } from "../format";

function Big({ label, value, sub, warn }: { label: string; value: string; sub?: string; warn?: boolean }) {
  return (
    <div className={`big ${warn ? "warn" : ""}`}>
      <div className="big-label">{label}</div>
      <div className="big-value mono">{value}</div>
      {sub && <div className="big-sub">{sub}</div>}
    </div>
  );
}

function Row({ k, x, y }: { k: string; x: string; y?: string }) {
  return (
    <tr>
      <th>{k}</th>
      <td className="mono">{x}</td>
      {y !== undefined && <td className="mono">{y}</td>}
    </tr>
  );
}

export function Results({ frame }: { frame: FramePacket | null }) {
  const m = frame?.metrics;
  const peakPct = m ? (m.peak / 65535) * 100 : 0;
  const saturated = !!m && m.saturatedPixels > 0;
  // Clip / Gaussian widths come from the two line profiles: X/Y, or u/v along the beam axes.
  const principal = !!m?.principalAxes;
  const profileRows = (
    <>
      {m?.clip.map((c) => <Row key={c.level} k={`Clip ${(c.level * 100).toFixed(1)}%`} x={fmtUm(c.xUm)} y={fmtUm(c.yUm)} />)}
      <Row k="Gauss 1/e²" x={fmtUm(m?.gaussX.diameterUm)} y={fmtUm(m?.gaussY.diameterUm)} />
      <Row k="Gauss fit r²" x={fmt(m?.gaussX.r2, 4)} y={fmt(m?.gaussY.r2, 4)} />
    </>
  );
  return (
    <aside className="results">
      <div className="bigs">
        <Big label="D4σ X" value={fmtUm(m?.d4sXUm)} />
        <Big label="D4σ Y" value={fmtUm(m?.d4sYUm)} />
        <Big
          label="Peak"
          value={m ? `${peakPct.toFixed(1)}%` : "—"}
          sub={saturated ? `${m!.saturatedPixels} px saturated` : m && peakPct < 25 ? "low signal" : undefined}
          warn={saturated || (!!m && peakPct < 25)}
        />
        <Big label="Ellipticity" value={fmt(m?.ellipticity, 3)} sub={m ? `${fmt(m.orientationDeg, 1)}°` : undefined} />
      </div>

      <table className="metrics">
        <thead>
          <tr>
            <th />
            <th>X</th>
            <th>Y</th>
          </tr>
        </thead>
        <tbody>
          <Row k="Centroid" x={fmtUm(m?.centroidUm[0])} y={fmtUm(m?.centroidUm[1])} />
          <Row k="D4σ (ISO)" x={fmtUm(m?.d4sXUm)} y={fmtUm(m?.d4sYUm)} />
          {!principal && profileRows}
        </tbody>
      </table>

      {principal && (
        <table className="metrics">
          <thead>
            <tr>
              <th className="axes-note">along beam axes ({fmt(m?.orientationDeg, 1)}°)</th>
              <th>u (major)</th>
              <th>v (minor)</th>
            </tr>
          </thead>
          <tbody>
            <Row k="D4σ (ISO)" x={fmtUm(m?.d4sMajorUm)} y={fmtUm(m?.d4sMinorUm)} />
            {profileRows}
          </tbody>
        </table>
      )}

      <table className="metrics">
        <tbody>
          <Row k="Major / minor" x={m ? `${fmtUm(m.d4sMajorUm)} / ${fmtUm(m.d4sMinorUm)}` : "—"} />
          <Row k="Effective ⌀ (D4σ)" x={fmtUm(m?.effectiveDiameterUm)} />
          <Row k="Effective ⌀ (area, DataRay)" x={fmtUm(m?.areaDiameterUm)} />
          <Row
            k="Centroid (DataRay)"
            x={m ? `${fmtUm(m.datarayCentroidPx[0] * m.pixelUm[0])}, ${fmtUm(m.datarayCentroidPx[1] * m.pixelUm[1])}` : "—"}
          />
          <Row k="Orientation" x={m ? `${fmt(m.orientationDeg, 1)}°` : "—"} />
          <Row k="Baseline" x={m ? `${((m.baseline / 65535) * 100).toFixed(2)}%` : "—"} />
          <Row k="Total (rel.)" x={m ? m.total.toExponential(3) : "—"} />
          <Row k="ISO iterations" x={m ? String(m.apertureIterations) : "—"} />
        </tbody>
      </table>

      <table className="metrics">
        <tbody>
          <Row k="Image" x={frame ? `${frame.width}×${frame.height} @ ${fmt(frame.pixelUm[0], 2)} µm` : "—"} />
          <Row k="Exposure" x={fmtMs(frame?.exposureMs)} />
          <Row k="Gain" x={frame ? `${fmt(frame.gain, 2)}×` : "—"} />
          <Row k="Frame rate" x={frame && frame.source === "live" ? `${fmt(frame.fps, 1)} fps` : "—"} />
        </tbody>
      </table>
    </aside>
  );
}
