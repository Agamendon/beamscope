import { useCallback, useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, Config, FramePacket, Status } from "./api";
import { Controls } from "./components/Controls";
import { DisplayOptions, ImageView } from "./components/ImageView";
import { LogPanel } from "./components/LogPanel";
import { M2Panel } from "./components/M2Panel";
import { ProfilePlot } from "./components/ProfilePlot";
import { Results } from "./components/Results";
import { fmtMs } from "./format";
import "./App.css";

type Tab = "profiles" | "log" | "m2";

const DISPLAY_KEY = "beamscope.display";

function deepMerge<T>(a: T, b: Record<string, unknown>): T {
  const out: Record<string, unknown> = { ...(a as Record<string, unknown>) };
  for (const [k, v] of Object.entries(b)) {
    const cur = out[k];
    out[k] = v && typeof v === "object" && !Array.isArray(v) && cur && typeof cur === "object" ? deepMerge(cur, v as Record<string, unknown>) : v;
  }
  return out as T;
}

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [frame, setFrame] = useState<FramePacket | null>(null);
  const [tab, setTab] = useState<Tab>("profiles");
  const [busy, setBusy] = useState<string | null>(null);
  const [toast, setToast] = useState<{ msg: string; err: boolean } | null>(null);
  const [display, setDisplayState] = useState<DisplayOptions>(() => {
    const d: DisplayOptions = { colormap: "turbo", log: false, autoContrast: false, showSaturation: true, showCentroid: true, showEllipse: true, showProfiles: true, zoomProfiles: true, showAperture: true };
    try {
      return { ...d, ...JSON.parse(localStorage.getItem(DISPLAY_KEY) ?? "{}") };
    } catch {
      return d;
    }
  });
  const setDisplay = (d: DisplayOptions) => {
    setDisplayState(d);
    try {
      localStorage.setItem(DISPLAY_KEY, JSON.stringify(d));
    } catch {
      /* ignore */
    }
  };
  const lastIndex = useRef<number | undefined>(undefined);
  const refreshStill = useRef(false);

  const notify = useCallback((msg: string, err = false) => {
    setToast({ msg, err });
    window.setTimeout(() => setToast((t) => (t?.msg === msg ? null : t)), err ? 7000 : 3500);
  }, []);

  // Status polling.
  const pollStatus = useCallback(async () => {
    try {
      const s = await api.status();
      setStatus(s);
      setConfig(s.config);
    } catch (e) {
      console.error(e);
    }
  }, []);
  useEffect(() => {
    pollStatus();
    const t = window.setInterval(pollStatus, 400);
    return () => window.clearInterval(t);
  }, [pollStatus]);

  // Frame pull loop: one request in flight, paced by the display refresh.
  useEffect(() => {
    let alive = true;
    let inFlight = false;
    let raf = 0;
    const loop = async () => {
      if (!alive) return;
      if (!inFlight) {
        inFlight = true;
        try {
          const since = refreshStill.current ? undefined : lastIndex.current;
          refreshStill.current = false;
          const p = await api.latestFrame(since);
          if (p && alive) {
            lastIndex.current = p.index;
            setFrame(p);
          }
        } catch (e) {
          console.error(e);
        } finally {
          inFlight = false;
        }
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      alive = false;
      cancelAnimationFrame(raf);
    };
  }, []);

  const patch = useCallback(
    async (p: Record<string, unknown>) => {
      setConfig((c) => (c ? deepMerge(c, p) : c));
      try {
        const cfg = await api.updateConfig(p);
        setConfig(cfg);
        refreshStill.current = true; // re-analyzed still frames need a re-fetch
      } catch (e) {
        notify(String(e), true);
      }
    },
    [notify],
  );

  const connect = async (simulate: boolean) => {
    setBusy("connect");
    try {
      const info = await api.connect(simulate);
      await api.start();
      notify(`Connected: ${info.model} (${info.link})`);
    } catch (e) {
      notify(String(e), true);
    } finally {
      setBusy(null);
      pollStatus();
    }
  };

  const toggleRun = async () => {
    try {
      if (status?.running) await api.stop();
      else await api.start();
    } catch (e) {
      notify(String(e), true);
    }
    pollStatus();
  };

  const openFile = async () => {
    const p = await open({ filters: [{ name: "DataRay capture", extensions: ["wcf"] }] });
    if (!p || Array.isArray(p)) return;
    try {
      const ver = await api.openWcf(p);
      refreshStill.current = true;
      notify(`Opened ${p.split(/[\\/]/).pop()} (DataRay ${ver || "unknown version"})`);
    } catch (e) {
      notify(String(e), true);
    }
    pollStatus();
  };

  const stamp = () => new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");

  const saveFile = async () => {
    if (!frame) return;
    const p = await save({ filters: [{ name: "DataRay capture", extensions: ["wcf"] }], defaultPath: `beam-${stamp()}.wcf` });
    if (!p) return;
    try {
      await api.saveWcf(p);
      notify(`Saved ${p}`);
    } catch (e) {
      notify(String(e), true);
    }
  };

  const exportAs = async (kind: "png" | "image" | "profiles") => {
    if (!frame) return;
    const ext = kind === "png" ? "png" : "csv";
    const p = await save({ filters: [{ name: ext.toUpperCase(), extensions: [ext] }], defaultPath: `beam-${kind}-${stamp()}.${ext}` });
    if (!p) return;
    try {
      if (kind === "png") {
        const c = document.querySelector<HTMLCanvasElement>(".image-view canvas");
        const blob = await new Promise<Blob | null>((r) => c?.toBlob(r, "image/png"));
        if (!blob) throw new Error("could not capture the image");
        await api.saveBytes(p, new Uint8Array(await blob.arrayBuffer()));
      } else if (kind === "image") await api.exportImageCsv(p);
      else await api.exportProfilesCsv(p);
      notify(`Saved ${p}`);
    } catch (e) {
      notify(String(e), true);
    }
  };

  const captureBackground = async (n: number) => {
    setBusy("background");
    try {
      await api.captureBackground(n);
      await patch({ subtractBackground: true });
      notify(`Background captured from ${n} frames`);
    } catch (e) {
      notify(String(e), true);
    } finally {
      setBusy(null);
    }
  };

  // Keyboard shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      if (e.ctrlKey && e.key === "o") {
        e.preventDefault();
        openFile();
      } else if (e.ctrlKey && e.key === "s") {
        e.preventDefault();
        saveFile();
      } else if (e.key === " " && status?.connected) {
        e.preventDefault();
        toggleRun();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const cam = status?.camera;
  const src = frame?.source ?? status?.source ?? "none";
  return (
    <div className="app">
      <header className="toolbar">
        <div className="brand">
          <span className="logo" />
          BeamScope
        </div>
        <div className="tb-group">
          {status?.connected ? (
            <>
              <button className={status.running ? "danger" : "primary"} onClick={toggleRun} title="Space">
                {status.running ? "■ Stop" : "▶ Live"}
              </button>
              <button onClick={() => api.disconnect().then(pollStatus)}>Disconnect</button>
            </>
          ) : (
            <>
              <button className="primary" disabled={!!busy} onClick={() => connect(false)}>
                {busy === "connect" ? "Connecting…" : "Connect camera"}
              </button>
              <button disabled={!!busy} onClick={() => connect(true)}>
                Simulator
              </button>
            </>
          )}
        </div>
        <div className="tb-group">
          <button onClick={openFile} title="Ctrl+O">
            Open .wcf
          </button>
          <button onClick={saveFile} disabled={!frame} title="Ctrl+S">
            Save .wcf
          </button>
          <div className="menu">
            <button disabled={!frame}>Export ▾</button>
            <div className="menu-items">
              <button onClick={() => exportAs("png")}>Image (PNG)</button>
              <button onClick={() => exportAs("image")}>Pixel data (CSV)</button>
              <button onClick={() => exportAs("profiles")}>Profiles (CSV)</button>
            </div>
          </div>
        </div>
        <div className="tb-status">
          {cam && <span className="pill">{cam.model}</span>}
          {src.startsWith("file:") && <span className="pill file">{src.slice(5).split(/[\\/]/).pop()}</span>}
          {status?.running && (
            <span className="mono">
              {status.fps.toFixed(1)} fps · {fmtMs(config?.exposureMs)}
              {config?.autoExposure ? " (auto)" : ""}
            </span>
          )}
          {status?.lastError && status.running && <span className="err" title={status.lastError}>⚠ {status.lastError.slice(0, 60)}</span>}
        </div>
      </header>

      <Controls
        status={status}
        config={config}
        patch={patch}
        display={display}
        setDisplay={setDisplay}
        onCaptureBackground={captureBackground}
        onClearBackground={() => api.clearBackground().then(pollStatus)}
        busy={busy}
      />

      <main className="main">
        <ImageView frame={frame} opts={display} />
        <div className="bottom">
          <nav className="tabs">
            <button className={tab === "profiles" ? "on" : ""} onClick={() => setTab("profiles")}>
              Profiles
            </button>
            <button className={tab === "log" ? "on" : ""} onClick={() => setTab("log")}>
              Logging{status?.logging ? " ●" : ""}
            </button>
            <button className={tab === "m2" ? "on" : ""} onClick={() => setTab("m2")}>
              M²
            </button>
          </nav>
          <div className="tab-body">
            {tab === "profiles" && (
              <div className="profiles">
                <ProfilePlot frame={frame} axis="x" zoom={display.zoomProfiles} />
                <ProfilePlot frame={frame} axis="y" zoom={display.zoomProfiles} />
              </div>
            )}
            {tab === "log" && <LogPanel status={status} notify={notify} />}
            {tab === "m2" && <M2Panel frame={frame} config={config} patch={patch} notify={notify} />}
          </div>
        </div>
      </main>

      <Results frame={frame} />

      {toast && <div className={`toast ${toast.err ? "err" : ""}`}>{toast.msg}</div>}
    </div>
  );
}
