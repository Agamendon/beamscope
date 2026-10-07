// 256-entry RGBA lookup tables for the image view.

export type ColormapName = "turbo" | "viridis" | "inferno" | "jet" | "gray" | "hot";

export const COLORMAPS: ColormapName[] = ["turbo", "jet", "viridis", "inferno", "hot", "gray"];

type Rgb = [number, number, number];
type Poly = [Rgb, Rgb, Rgb, Rgb, Rgb, Rgb, Rgb];

// Polynomial fits (degree 6) of the perceptual maps; accurate to a few 1/255.
const VIRIDIS: Poly = [
  [0.2777273272234177, 0.005407344544966578, 0.3340998053353061],
  [0.1050930431085774, 1.404613529898575, 1.384590162594685],
  [-0.3308618287255563, 0.214847559468213, 0.09509516302823659],
  [-4.634230498983486, -5.799100973351585, -19.33244095627987],
  [6.228269936347081, 14.17993336680509, 56.69055260068105],
  [4.776384997670288, -13.74514537774601, -65.35303263337234],
  [-5.435455855934631, 4.645852612178535, 26.3124352495832],
];
const INFERNO: Poly = [
  [0.0002189403691192265, 0.001651004631001012, -0.01948089843709184],
  [0.1065134194856116, 0.5639564367884091, 3.932712388889277],
  [11.60249308247187, -3.972853965665698, -15.9423941062914],
  [-41.70399613139459, 17.43639888205313, 44.35414519872813],
  [77.162935699427, -33.40235894210092, -81.80730925738993],
  [-71.31942824499214, 32.62606426397723, 73.20951985803202],
  [25.13112622477341, -12.24266895238567, -23.07032500287172],
];

function poly(c: Poly, t: number): Rgb {
  const out: Rgb = [0, 0, 0];
  for (let k = 0; k < 3; k++) {
    let v = c[6][k];
    for (let i = 5; i >= 0; i--) v = v * t + c[i][k];
    out[k] = v;
  }
  return out;
}

function turbo(t: number): Rgb {
  // Polynomial approximation of Google's Turbo.
  const r = 0.13572138 + t * (4.6153926 + t * (-42.66032258 + t * (132.13108234 + t * (-152.94239396 + t * 59.28637943))));
  const g = 0.09140261 + t * (2.19418839 + t * (4.84296658 + t * (-14.18503333 + t * (4.27729857 + t * 2.82956604))));
  const b = 0.1066733 + t * (12.64194608 + t * (-60.58204836 + t * (110.36276771 + t * (-89.90310912 + t * 27.34824973))));
  return [r, g, b];
}

function jet(t: number): Rgb {
  const f = (x: number) => Math.min(1, Math.max(0, 1.5 - Math.abs(4 * t - x)));
  return [f(3), f(2), f(1)];
}

function hot(t: number): Rgb {
  return [Math.min(1, t * 3), Math.min(1, Math.max(0, t * 3 - 1)), Math.min(1, Math.max(0, t * 3 - 2))];
}

export function lut(name: ColormapName): Uint8Array {
  const out = new Uint8Array(256 * 4);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    let c: Rgb;
    switch (name) {
      case "turbo":
        c = turbo(t);
        break;
      case "viridis":
        c = poly(VIRIDIS, t);
        break;
      case "inferno":
        c = poly(INFERNO, t);
        break;
      case "jet":
        c = jet(t);
        break;
      case "hot":
        c = hot(t);
        break;
      default:
        c = [t, t, t];
    }
    for (let k = 0; k < 3; k++) out[i * 4 + k] = Math.round(Math.min(1, Math.max(0, c[k])) * 255);
    out[i * 4 + 3] = 255;
  }
  return out;
}

export function cssGradient(name: ColormapName): string {
  const l = lut(name);
  const stops: string[] = [];
  for (let i = 0; i <= 8; i++) {
    const j = Math.round((i / 8) * 255) * 4;
    stops.push(`rgb(${l[j]},${l[j + 1]},${l[j + 2]}) ${(i / 8) * 100}%`);
  }
  return `linear-gradient(90deg, ${stops.join(",")})`;
}
