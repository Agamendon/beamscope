import { useEffect, useState } from "react";
import type { Binning, Config, Status } from "../api";
import { COLORMAPS, ColormapName, cssGradient } from "../colormaps";
import { fmtMs } from "../format";
import type { DisplayOptions } from "./ImageView";
import { OrientationControl } from "./OrientationControl";

export const ORIENTATION_KEY = "beamscope.orientation";

interface Props {
  status: Status | null;
  config: Config | null;
  patch: (p: Record<string, unknown>) => void;
  display: DisplayOptions;
  setDisplay: (d: DisplayOptions) => void;
  onCaptureBackground: (frames: number) => void;
  onClearBackground: () => void;
  busy: string | null;
}

function Section({ title, children, defaultOpen = true }: { title: string; children: React.ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className={`section ${open ? "open" : ""}`}>
      <button className="section-title" onClick={() => setOpen(!open)}>
        <span>{title}</span>
        <span className="chev">{open ? "▾" : "▸"}</span>
      </button>
      {open && <div className="section-body">{children}</div>}
    </section>
  );
}

function Toggle({ label, value, onChange, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <label className={`toggle ${disabled ? "disabled" : ""}`}>
      <input type="checkbox" aria-label={label} checked={value} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="track">
        <span className="thumb" />
      </span>
      <span>{label}</span>
    </label>
  );
}

function NumberField({ label, value, onCommit, step = 1, min, max, suffix }: { label: string; value: number; onCommit: (v: number) => void; step?: number; min?: number; max?: number; suffix?: string }) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  const commit = () => {
    const v = parseFloat(text);
    if (isFinite(v)) onCommit(Math.min(max ?? Infinity, Math.max(min ?? -Infinity, v)));
    else setText(String(value));
  };
  return (
    <label className="field">
      <span>{label}</span>
      <span className="input-wrap">
        <input
          type="number"
          value={text}
          step={step}
          min={min}
          max={max}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
        {suffix && <span className="suffix">{suffix}</span>}
      </span>
    </label>
  );
}

export function Controls({ status, config, patch, display, setDisplay, onCaptureBackground, onClearBackground, busy }: Props) {
  const cam = status?.camera;
  const live = !!status?.connected;
  const [bgFrames, setBgFrames] = useState(10);
  if (!config) return <aside className="controls" />;

  const minE = cam?.exposureMinMs ?? 0.03;
  const maxE = cam?.exposureMaxMs ?? 1000;
  const toSlider = (ms: number) => (Math.log(ms / minE) / Math.log(maxE / minE)) * 1000;
  const fromSlider = (v: number) => minE * Math.pow(maxE / minE, v / 1000);

  return (
    <aside className="controls">
      <Section title="Acquisition">
        <div className="field-row">
          <span className="label">Exposure</span>
          <span className="value mono">{fmtMs(config.exposureMs)}</span>
        </div>
        <input
          type="range"
          aria-label="Exposure"
          min={0}
          max={1000}
          value={toSlider(config.exposureMs)}
          disabled={!live || config.autoExposure}
          onChange={(e) => patch({ exposureMs: fromSlider(+e.target.value) })}
        />
        <Toggle label="Auto exposure" value={config.autoExposure} disabled={!live} onChange={(v) => patch({ autoExposure: v })} />

        <div className="field-row">
          <span className="label">Gain</span>
          <span className="value mono">step {config.gainStep}</span>
        </div>
        <input type="range" min={1} max={cam?.gainSteps ?? 44} value={config.gainStep} disabled={!live} onChange={(e) => patch({ gainStep: +e.target.value })} />

        <div className="field-row">
          <span className="label">Black level</span>
          <span className="value mono">{config.blackLevel}</span>
        </div>
        <input
          type="range"
          min={0}
          max={cam?.blackLevelMax ?? 10}
          value={config.blackLevel}
          disabled={!live || config.autoBlack}
          onChange={(e) => patch({ blackLevel: +e.target.value })}
        />
        <Toggle label="Auto black level" value={config.autoBlack} disabled={!live} onChange={(v) => patch({ autoBlack: v })} />

        <label className="field">
          <span>Resolution</span>
          <select value={config.binning} disabled={!live} onChange={(e) => patch({ binning: e.target.value as Binning })}>
            <option value="full">Full {cam ? `${cam.sensorWidth}×${cam.sensorHeight}` : ""}</option>
            <option value="bin2">2×2 binned (fast)</option>
            <option value="bin4">4×4 binned</option>
          </select>
        </label>
        <NumberField label="Average" value={config.average} min={1} max={100} suffix="frames" onCommit={(v) => patch({ average: Math.round(v) })} />
      </Section>

      <Section title="Camera orientation">
        <p className="hint">Match the image to how the camera is mounted. X/Y profiles and all X/Y numbers follow.</p>
        <OrientationControl
          value={config.orientation ?? { rotate: 0, flipH: false, flipV: false }}
          onChange={(o) => {
            patch({ orientation: o });
            try {
              localStorage.setItem(ORIENTATION_KEY, JSON.stringify(o));
            } catch {
              /* ignore */
            }
          }}
        />
      </Section>

      <Section title="Background">
        <p className="hint">Block the beam, then capture. Subtracted from every frame.</p>
        <div className="btn-row">
          <NumberField label="Frames" value={bgFrames} min={1} max={100} onCommit={(v) => setBgFrames(Math.round(v))} />
        </div>
        <div className="btn-row">
          <button disabled={!status?.running || !!busy} onClick={() => onCaptureBackground(bgFrames)}>
            {busy === "background" ? "Capturing…" : "Capture"}
          </button>
          <button disabled={!status?.hasBackground} onClick={onClearBackground}>
            Clear
          </button>
        </div>
        <Toggle label={`Subtract${status?.hasBackground ? "" : " (none captured)"}`} value={config.subtractBackground} onChange={(v) => patch({ subtractBackground: v })} />
      </Section>

      <Section title="Analysis">
        <Toggle label="Auto baseline (border)" value={config.analysis.autoBaseline} onChange={(v) => patch({ analysis: { autoBaseline: v } })} />
        <Toggle label="ISO 11146 aperture" value={config.analysis.isoAperture} onChange={(v) => patch({ analysis: { isoAperture: v } })} />
        <Toggle
          label="Profiles along beam axes (u/v)"
          value={!!config.analysis.principalAxes}
          onChange={(v) => patch({ analysis: { principalAxes: v } })}
        />
        <Toggle
          label="Profiles through centroid"
          value={config.analysis.profilesThroughCentroid || !!config.analysis.principalAxes}
          disabled={!!config.analysis.principalAxes}
          onChange={(v) => patch({ analysis: { profilesThroughCentroid: v } })}
        />
        <div className="grid2">
          <NumberField
            label="Clip 1"
            value={+(config.analysis.clipLevels[0] * 100).toFixed(2)}
            min={1}
            max={99}
            step={0.1}
            suffix="%"
            onCommit={(v) => patch({ analysis: { clipLevels: [v / 100, config.analysis.clipLevels[1]] } })}
          />
          <NumberField
            label="Clip 2"
            value={+(config.analysis.clipLevels[1] * 100).toFixed(2)}
            min={1}
            max={99}
            step={0.1}
            suffix="%"
            onCommit={(v) => patch({ analysis: { clipLevels: [config.analysis.clipLevels[0], v / 100] } })}
          />
        </div>
        <NumberField
          label="Noise clip"
          value={+(config.analysis.noiseClip * 100).toFixed(2)}
          min={0}
          max={50}
          step={0.5}
          suffix="%"
          onCommit={(v) => patch({ analysis: { noiseClip: v / 100 } })}
        />
        <NumberField label="Profile band" value={config.analysis.profileBand} min={1} max={51} step={2} suffix="px" onCommit={(v) => patch({ analysis: { profileBand: Math.round(v) } })} />
        <NumberField label="Wavelength" value={config.wavelengthNm} min={150} max={20000} step={1} suffix="nm" onCommit={(v) => patch({ wavelengthNm: v })} />
      </Section>

      <Section title="Display">
        <div className="colormaps">
          {COLORMAPS.map((c) => (
            <button
              key={c}
              title={c}
              className={`cmap ${display.colormap === c ? "active" : ""}`}
              style={{ background: cssGradient(c as ColormapName) }}
              onClick={() => setDisplay({ ...display, colormap: c })}
            />
          ))}
        </div>
        <Toggle label="Log scale" value={display.log} onChange={(v) => setDisplay({ ...display, log: v })} />
        <Toggle label="Auto contrast" value={display.autoContrast} onChange={(v) => setDisplay({ ...display, autoContrast: v })} />
        <Toggle label="Highlight saturation" value={display.showSaturation} onChange={(v) => setDisplay({ ...display, showSaturation: v })} />
        <Toggle label="Centroid" value={display.showCentroid} onChange={(v) => setDisplay({ ...display, showCentroid: v })} />
        <Toggle label="D4σ ellipse" value={display.showEllipse} onChange={(v) => setDisplay({ ...display, showEllipse: v })} />
        <Toggle label="Profile lines" value={display.showProfiles} onChange={(v) => setDisplay({ ...display, showProfiles: v })} />
        <Toggle label="Zoom profiles to beam" value={display.zoomProfiles} onChange={(v) => setDisplay({ ...display, zoomProfiles: v })} />
        <Toggle label="ISO integration area (3×)" value={display.showAperture} onChange={(v) => setDisplay({ ...display, showAperture: v })} />
      </Section>

      {cam && (
        <Section title="Camera" defaultOpen={false}>
          <dl className="kv">
            <dt>Model</dt>
            <dd>{cam.model}</dd>
            <dt>Serial</dt>
            <dd className="mono">{cam.serial}</dd>
            <dt>Sensor</dt>
            <dd>
              {cam.sensor}, {cam.sensorWidth}×{cam.sensorHeight}, {cam.pixelUm} µm
            </dd>
            <dt>Link</dt>
            <dd>{cam.link}</dd>
            <dt>Firmware</dt>
            <dd className="mono">{cam.firmware}</dd>
          </dl>
        </Section>
      )}
    </aside>
  );
}
