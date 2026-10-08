import type { Orientation } from "../api";

/** Human-readable summary, e.g. "90° CW, mirrored ↔". Empty for the identity. */
export function describeOrientation(o: Orientation | undefined): string {
  if (!o) return "";
  const parts: string[] = [];
  if (o.rotate % 360) parts.push(`${o.rotate % 360}° CW`);
  if (o.flipH) parts.push("mirrored ↔");
  if (o.flipV) parts.push("mirrored ↕");
  return parts.join(", ");
}

/** CSS transform equivalent to the backend's mapping (mirror first, then rotate clockwise). */
function cssTransform(o: Orientation) {
  return `rotate(${o.rotate}deg) scale(${o.flipH ? -1 : 1}, ${o.flipV ? -1 : 1})`;
}

interface Props {
  value: Orientation;
  onChange: (o: Orientation) => void;
}

export function OrientationControl({ value, onChange }: Props) {
  const turn = (deg: number) => onChange({ ...value, rotate: (((value.rotate + deg) % 360) + 360) % 360 });
  const identity = value.rotate % 360 === 0 && !value.flipH && !value.flipV;
  return (
    <div className="orientation">
      <div className="orient-preview" title="How the sensor's top-left “F” appears on screen">
        <span style={{ transform: cssTransform(value) }}>F</span>
      </div>
      <div className="orient-body">
        <div className="btn-row">
          <button aria-label="Rotate counter-clockwise" title="Rotate 90° counter-clockwise" onClick={() => turn(-90)}>
            ↺ 90°
          </button>
          <button aria-label="Rotate clockwise" title="Rotate 90° clockwise" onClick={() => turn(90)}>
            ↻ 90°
          </button>
        </div>
        <div className="btn-row">
          <button aria-label="Mirror horizontally" className={value.flipH ? "on" : ""} onClick={() => onChange({ ...value, flipH: !value.flipH })}>
            ⇋ Mirror
          </button>
          <button aria-label="Mirror vertically" className={value.flipV ? "on" : ""} onClick={() => onChange({ ...value, flipV: !value.flipV })}>
            ⇅ Mirror
          </button>
        </div>
        <div className="orient-state">
          {identity ? (
            <span className="dim">As mounted (sensor axes)</span>
          ) : (
            <>
              <span>{describeOrientation(value)}</span>
              <button className="link" aria-label="Reset orientation" onClick={() => onChange({ rotate: 0, flipH: false, flipV: false })}>
                reset
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
