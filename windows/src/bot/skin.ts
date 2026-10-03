// Iskra's look: silhouette, palette, eyes; motion lives in engine.ts.

export type RGB = readonly [number, number, number];

export interface Skin {
  /** Body outline in R units; `morph` 0 = body, 1 = the mail-box form. */
  hull(a: number, morph?: number): readonly [number, number];
  /** Half axes of the outline, for placing eyes and blush. */
  extents(morph?: number): readonly [number, number];
  top: RGB;
  bottom: RGB;
  ink: string;
  accent: RGB;
  /** Eye spacing, height and size, in R units. */
  eyes: { sp: number; pitch: number; w: number; h: number };
  /** Hands, for a character that waves. Iskra hops instead. */
  waves: boolean;
  /** Star shards orbiting the body. */
  shards: boolean;
}

const BODY_N = 1.7;
const BOX = { rx: 1.0, ry: 0.94, r: 0.42 };

const sup = (v: number, n: number) => Math.sign(v) * Math.pow(Math.abs(v), 2 / n);

/** Ray ∩ rounded rect, so the body can morph into a box. */
function rrPoint(ca: number, sa: number, w: number, h: number, cr: number) {
  const eps = 1e-6;
  const kx = ca >= 0 ? 1 : -1;
  const ky = sa >= 0 ? 1 : -1;
  const cx = kx * (w - cr);
  const cy = ky * (h - cr);

  const dot = ca * cx + sa * cy;
  const disc = dot * dot - (cx * cx + cy * cy - cr * cr);
  if (disc >= 0) {
    const t = dot + Math.sqrt(disc);
    if (t > eps) {
      const px = ca * t;
      const py = sa * t;
      if (Math.abs(px) >= w - cr - eps && Math.abs(py) >= h - cr - eps) return { x: px, y: py };
    }
  }
  if (Math.abs(sa) > eps) {
    const t = (ky * h) / sa;
    if (t > eps) {
      const px = ca * t;
      if (Math.abs(px) <= w - cr + eps) return { x: px, y: ky * h };
    }
  }
  if (Math.abs(ca) > eps) {
    const t = (kx * w) / ca;
    if (t > eps) {
      const py = sa * t;
      if (Math.abs(py) <= h - cr + eps) return { x: kx * w, y: py };
    }
  }
  return { x: kx * w, y: ky * h };
}

export const ISKRA: Skin = {
  hull(a, morph = 0) {
    const ca = Math.cos(a);
    const sa = Math.sin(a);
    const bx = sup(ca, BODY_N) * 1.12;
    const by = sup(sa, BODY_N) * 0.9;
    const m = Math.min(1, Math.max(0, morph));
    if (m < 0.005) return [bx, by];
    const box = rrPoint(ca, sa, BOX.rx, BOX.ry, BOX.r);
    return [bx + (box.x - bx) * m, by + (box.y - by) * m];
  },
  extents(morph = 0) {
    const m = Math.min(1, Math.max(0, morph));
    return [1.12 + (BOX.rx - 1.12) * m, 0.9 + (BOX.ry - 0.9) * m];
  },
  top: [0.66, 0.96, 0.9],
  bottom: [0.16, 0.45, 0.85],
  ink: "#06121E",
  accent: [0.48, 0.88, 1],
  eyes: { sp: 0.3, pitch: -0.04, w: 0.2, h: 0.22 },
  waves: false,
  shards: true,
};
