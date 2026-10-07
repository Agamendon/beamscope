import { useCallback, useEffect, useRef, useState } from "react";
import type { FramePacket } from "../api";
import { ColormapName, lut } from "../colormaps";
import { fmtUm } from "../format";

export interface DisplayOptions {
  colormap: ColormapName;
  log: boolean;
  autoContrast: boolean;
  showSaturation: boolean;
  showCentroid: boolean;
  showEllipse: boolean;
  showProfiles: boolean;
  zoomProfiles: boolean;
  /** ISO 11146-3 integration area (3× D4σ rectangle) used for the moments. */
  showAperture: boolean;
}

const VS = `#version 300 es
in vec2 a_pos;
uniform vec2 u_canvas;
uniform float u_scale;
uniform vec2 u_t;
uniform vec2 u_size;
out vec2 v_uv;
void main() {
  vec2 s = a_pos * u_scale + u_t;
  vec2 c = s / u_canvas * 2.0 - 1.0;
  gl_Position = vec4(c.x, -c.y, 0.0, 1.0);
  v_uv = a_pos / u_size;
}`;

const FS = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_img;
uniform sampler2D u_lut;
uniform float u_lo;
uniform float u_hi;
uniform bool u_log;
uniform bool u_sat;
out vec4 o;
void main() {
  vec4 t = texture(u_img, v_uv);
  float v = (t.g * 255.0 * 256.0 + t.r * 255.0) / 65535.0;
  if (u_sat && v >= 0.9918) { o = vec4(1.0, 0.0, 1.0, 1.0); return; }
  float n = clamp((v - u_lo) / max(u_hi - u_lo, 1e-6), 0.0, 1.0);
  if (u_log) n = log(1.0 + 1000.0 * n) / log(1001.0);
  o = texture(u_lut, vec2(n * (255.0 / 256.0) + 0.5 / 256.0, 0.5));
}`;

interface View {
  scale: number;
  tx: number;
  ty: number;
}

interface GlState {
  gl: WebGL2RenderingContext;
  prog: WebGLProgram;
  img: WebGLTexture;
  lut: WebGLTexture;
  buf: WebGLBuffer;
  texSize: [number, number];
}

function compile(gl: WebGL2RenderingContext, type: number, src: string) {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
  return s;
}

function initGl(canvas: HTMLCanvasElement): GlState {
  const gl = canvas.getContext("webgl2", { antialias: false, preserveDrawingBuffer: true })!;
  const prog = gl.createProgram()!;
  gl.attachShader(prog, compile(gl, gl.VERTEX_SHADER, VS));
  gl.attachShader(prog, compile(gl, gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(prog);
  const mk = () => {
    const t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  };
  return { gl, prog, img: mk(), lut: mk(), buf: gl.createBuffer()!, texSize: [0, 0] };
}

interface Props {
  frame: FramePacket | null;
  opts: DisplayOptions;
}

export function ImageView({ frame, opts }: Props) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const gls = useRef<GlState | null>(null);
  const [size, setSize] = useState<[number, number]>([1, 1]);
  const [view, setView] = useState<View | null>(null);
  const [hover, setHover] = useState<{ x: number; y: number; v: number } | null>(null);
  const drag = useRef<{ x: number; y: number; v: View } | null>(null);
  const imgSize = useRef<[number, number]>([0, 0]);

  // Size tracking.
  useEffect(() => {
    const el = wrap.current!;
    const ro = new ResizeObserver(() => setSize([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const fit = useCallback((): View | null => {
    const [iw, ih] = imgSize.current;
    if (!iw) return null;
    const s = Math.min(size[0] / iw, size[1] / ih) * 0.98;
    return { scale: s, tx: (size[0] - iw * s) / 2, ty: (size[1] - ih * s) / 2 };
  }, [size]);

  // Refit when the image size or the panel size changes.
  useEffect(() => {
    if (!frame) return;
    const [iw, ih] = imgSize.current;
    if (iw !== frame.width || ih !== frame.height) {
      imgSize.current = [frame.width, frame.height];
      setView(fit());
    }
  }, [frame, fit]);
  useEffect(() => setView(fit()), [size, fit]);

  // LUT upload.
  useEffect(() => {
    if (!canvas.current) return;
    if (!gls.current) gls.current = initGl(canvas.current);
    const { gl, lut: t } = gls.current;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, lut(opts.colormap));
  }, [opts.colormap]);

  // Draw.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    if (!gls.current) gls.current = initGl(c);
    const st = gls.current;
    const { gl, prog } = st;
    const dpr = window.devicePixelRatio || 1;
    const W = Math.max(1, Math.round(size[0] * dpr));
    const H = Math.max(1, Math.round(size[1] * dpr));
    if (c.width !== W || c.height !== H) {
      c.width = W;
      c.height = H;
    }
    gl.viewport(0, 0, W, H);
    gl.clearColor(0.043, 0.051, 0.063, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!frame || !view) return;

    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.bindTexture(gl.TEXTURE_2D, st.img);
    const bytes = new Uint8Array(frame.image.buffer, frame.image.byteOffset, frame.image.length * 2);
    if (st.texSize[0] !== frame.dispWidth || st.texSize[1] !== frame.dispHeight) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG8, frame.dispWidth, frame.dispHeight, 0, gl.RG, gl.UNSIGNED_BYTE, bytes);
      st.texSize = [frame.dispWidth, frame.dispHeight];
    } else {
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, frame.dispWidth, frame.dispHeight, gl.RG, gl.UNSIGNED_BYTE, bytes);
    }

    gl.useProgram(prog);
    const w = frame.width;
    const h = frame.height;
    gl.bindBuffer(gl.ARRAY_BUFFER, st.buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, w, 0, 0, h, w, h]), gl.DYNAMIC_DRAW);
    const loc = gl.getAttribLocation(prog, "a_pos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const u = (n: string) => gl.getUniformLocation(prog, n);
    gl.uniform2f(u("u_canvas"), W, H);
    gl.uniform1f(u("u_scale"), view.scale * dpr);
    gl.uniform2f(u("u_t"), view.tx * dpr, view.ty * dpr);
    gl.uniform2f(u("u_size"), w, h);
    const m = frame.metrics;
    const lo = opts.autoContrast ? Math.max(0, m.baseline) / 65535 : 0;
    const hi = opts.autoContrast ? Math.max(m.peak, m.baseline + 64) / 65535 : 1;
    gl.uniform1f(u("u_lo"), lo);
    gl.uniform1f(u("u_hi"), hi);
    gl.uniform1i(u("u_log"), opts.log ? 1 : 0);
    gl.uniform1i(u("u_sat"), opts.showSaturation ? 1 : 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, st.img);
    gl.uniform1i(u("u_img"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, st.lut);
    gl.uniform1i(u("u_lut"), 1);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.activeTexture(gl.TEXTURE0);
  }, [frame, view, size, opts]);

  const toImg = (e: React.MouseEvent) => {
    const r = wrap.current!.getBoundingClientRect();
    if (!view) return null;
    return { x: (e.clientX - r.left - view.tx) / view.scale, y: (e.clientY - r.top - view.ty) / view.scale };
  };

  const onWheel = (e: React.WheelEvent) => {
    if (!view) return;
    const r = wrap.current!.getBoundingClientRect();
    const mx = e.clientX - r.left;
    const my = e.clientY - r.top;
    const k = Math.exp(-e.deltaY * 0.0015);
    const scale = Math.min(64, Math.max(0.05, view.scale * k));
    const f = scale / view.scale;
    setView({ scale, tx: mx - (mx - view.tx) * f, ty: my - (my - view.ty) * f });
  };

  const onMove = (e: React.MouseEvent) => {
    if (drag.current && view) {
      const d = drag.current;
      setView({ ...d.v, tx: d.v.tx + e.clientX - d.x, ty: d.v.ty + e.clientY - d.y });
      return;
    }
    const p = toImg(e);
    if (!p || !frame || p.x < 0 || p.y < 0 || p.x >= frame.width || p.y >= frame.height) {
      setHover(null);
      return;
    }
    const dx = Math.floor(p.x / frame.dispScale);
    const dy = Math.floor(p.y / frame.dispScale);
    setHover({ x: p.x, y: p.y, v: frame.image[dy * frame.dispWidth + dx] ?? 0 });
  };

  const m = frame?.metrics;
  const px = m ? m.pixelUm[0] : 1;
  const py = m ? m.pixelUm[1] : 1;
  const T = (x: number, y: number) => (view ? [x * view.scale + view.tx, y * view.scale + view.ty] : [0, 0]);

  return (
    <div
      ref={wrap}
      className="image-view"
      onWheel={onWheel}
      onMouseDown={(e) => view && (drag.current = { x: e.clientX, y: e.clientY, v: view })}
      onMouseUp={() => (drag.current = null)}
      onMouseLeave={() => {
        drag.current = null;
        setHover(null);
      }}
      onMouseMove={onMove}
      onDoubleClick={() => setView(fit())}
    >
      <canvas ref={canvas} style={{ width: size[0], height: size[1] }} />
      {frame && view && m && (
        <svg className="overlay" width={size[0]} height={size[1]}>
          {opts.showProfiles &&
            ([
              [m.profileLineX, m.principalAxes ? "u" : null],
              [m.profileLineY, m.principalAxes ? "v" : null],
            ] as const).map(([l, label], i) => {
              // Pixel centres sit at integer coordinates; the canvas maps pixel k to [k, k+1).
              const [x1, y1] = T(l.x0 + 0.5, l.y0 + 0.5);
              const [x2, y2] = T(l.x0 + (l.len - 1) * l.dx + 0.5, l.y0 + (l.len - 1) * l.dy + 0.5);
              return (
                <g key={i} className="ov-profile">
                  <line x1={x1} y1={y1} x2={x2} y2={y2} />
                  {label && (
                    <text x={x2 - l.dx * 14} y={y2 - l.dy * 14} className="ov-axis-label">
                      {label}
                    </text>
                  )}
                </g>
              );
            })}
          {opts.showAperture && m.aperture && (
            <polygon
              className="ov-aperture"
              points={(() => {
                const a = m.aperture!;
                const [c, s] = [Math.cos(a.angleRad), Math.sin(a.angleRad)];
                return [
                  [1, 1],
                  [-1, 1],
                  [-1, -1],
                  [1, -1],
                ]
                  .map(([i, j]) => T(a.cx + 0.5 + i * a.halfMajorPx * c - j * a.halfMinorPx * s, a.cy + 0.5 + i * a.halfMajorPx * s + j * a.halfMinorPx * c).join(","))
                  .join(" ");
              })()}
            />
          )}
          {opts.showEllipse && m.d4sMajorUm > 0 && (
            <ellipse
              className="ov-ellipse"
              cx={T(m.centroidPx[0] + 0.5, 0)[0]}
              cy={T(0, m.centroidPx[1] + 0.5)[1]}
              rx={(m.d4sMajorUm / 2 / px) * view.scale}
              ry={(m.d4sMinorUm / 2 / py) * view.scale}
              transform={`rotate(${-m.orientationDeg} ${T(m.centroidPx[0] + 0.5, 0)[0]} ${T(0, m.centroidPx[1] + 0.5)[1]})`}
            />
          )}
          {opts.showCentroid && (
            <g className="ov-centroid" transform={`translate(${T(m.centroidPx[0] + 0.5, m.centroidPx[1] + 0.5).join(",")})`}>
              <line x1={-9} y1={0} x2={9} y2={0} />
              <line x1={0} y1={-9} x2={0} y2={9} />
            </g>
          )}
        </svg>
      )}
      {hover && frame && (
        <div className="hover-readout">
          x {fmtUm(hover.x * px)} · y {fmtUm(hover.y * py)} · {((hover.v / 65535) * 100).toFixed(1)}%
        </div>
      )}
      {!frame && <div className="empty-hint">Connect a camera or open a .wcf file</div>}
    </div>
  );
}
