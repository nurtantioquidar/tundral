/* Pixel art rasteriser shared by the tool icons and the studio logo. Artwork is described as simple
 * shapes in a 120-unit space, sampled onto a small grid, then outlined and shaded the way the pond
 * scene is: a dark outline, light pixels on top-left edges, dark on bottom-right. Layers paint in
 * order; one coral accent region sits on top and means the thing the subject decides or protects
 * (see DESIGN.md § Icons). Pure data in, rects out — rendering lives in icons.tsx. */

export type Pred = (u: number, v: number) => boolean;
export type Tone = "lilac" | "sage" | "blue" | "green" | "sand" | "pink" | "grey";

export interface Layer {
  tone: Tone;
  shapes: Pred[];
  /* Removed from this layer only (holes, notches). */
  cut?: Pred[];
}

export interface ArtSpec {
  /* Pixels per side before the outline; defaults to 24. */
  grid?: number;
  layers: Layer[];
  accent?: Pred[];
  dark?: Pred[];
  /* Darker bands every few rows, for a wrapped look. */
  bands?: boolean;
  /* Outline the seam where one layer meets another, so overlapping objects read separately. */
  separate?: boolean;
}

export const ellipse =
  (cx: number, cy: number, rx: number, ry: number, deg = 0): Pred =>
  (u, v) => {
    const a = (-deg * Math.PI) / 180;
    const dx = (u - cx) * Math.cos(a) - (v - cy) * Math.sin(a);
    const dy = (u - cx) * Math.sin(a) + (v - cy) * Math.cos(a);
    return (dx / rx) ** 2 + (dy / ry) ** 2 <= 1;
  };

export const rect =
  (x: number, y: number, w: number, h: number): Pred =>
  (u, v) =>
    u >= x && u <= x + w && v >= y && v <= y + h;

export const poly =
  (pts: [number, number][]): Pred =>
  (u, v) => {
    let inside = false;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i];
      const [xj, yj] = pts[j];
      if (yi > v !== yj > v && u < ((xj - xi) * (v - yi)) / (yj - yi) + xi) inside = !inside;
    }
    return inside;
  };

const ACCENT = -1;
const DARK = -2;

export interface PixelRect {
  x: number;
  y: number;
  w: number;
  cls: string;
}

export function rasterise(spec: ArtSpec) {
  const grid = spec.grid ?? 24;
  const unit = 120 / grid;
  const size = grid + 2;
  const cells = new Int8Array(size * size);
  const at = (x: number, y: number) => (x < 0 || y < 0 || x >= size || y >= size ? 0 : cells[y * size + x]);

  for (let y = 0; y < grid; y++) {
    for (let x = 0; x < grid; x++) {
      const u = (x + 0.5) * unit;
      const v = (y + 0.5) * unit;
      let c = 0;
      spec.layers.forEach((layer, i) => {
        if (layer.shapes.some((p) => p(u, v)) && !layer.cut?.some((p) => p(u, v))) c = i + 1;
      });
      if (spec.accent?.some((p) => p(u, v))) c = ACCENT;
      if (spec.dark?.some((p) => p(u, v))) c = DARK;
      cells[(y + 1) * size + (x + 1)] = c;
    }
  }

  const classes: (string | null)[] = new Array(size * size).fill(null);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const c = at(x, y);
      if (c === 0) {
        const edge = at(x - 1, y) || at(x + 1, y) || at(x, y - 1) || at(x, y + 1);
        classes[y * size + x] = edge ? "px-outline" : null;
        continue;
      }
      if (c === DARK) {
        classes[y * size + x] = "px-outline";
        continue;
      }
      /* A pixel of a front layer that touches a layer behind it becomes the seam outline. */
      const order = (n: number) => (n === ACCENT ? spec.layers.length + 1 : n);
      if (spec.separate) {
        const behind = [at(x - 1, y), at(x + 1, y), at(x, y - 1), at(x, y + 1)].some(
          (n) => n > 0 && order(n) < order(c),
        );
        if (behind) {
          classes[y * size + x] = "px-outline";
          continue;
        }
      }
      const lit = !at(x - 1, y) || !at(x, y - 1);
      const shade = !at(x + 1, y) || !at(x, y + 1);
      if (c === ACCENT) {
        classes[y * size + x] = lit ? "px-coral-l" : "px-coral";
        continue;
      }
      let k = lit ? "l" : shade ? "d" : "m";
      if (spec.bands && k === "m" && y % 3 === 0) k = "d";
      classes[y * size + x] = `px-${spec.layers[c - 1].tone}-${k}`;
    }
  }

  /* Merge horizontal runs of one class into a single rect. */
  const rects: PixelRect[] = [];
  for (let y = 0; y < size; y++) {
    let x = 0;
    while (x < size) {
      const cls = classes[y * size + x];
      if (!cls) {
        x++;
        continue;
      }
      let w = 1;
      while (x + w < size && classes[y * size + x + w] === cls) w++;
      rects.push({ x, y, w, cls });
      x += w;
    }
  }
  return { size, rects };
}

const arch: Pred = (u, v) => {
  const outer = u >= 10 && u <= 110 && v <= 100 && (v >= 72 || ((u - 60) / 50) ** 2 + ((v - 72) / 34) ** 2 <= 1);
  const inner = u > 28 && u < 92 && (v >= 74 || ((u - 60) / 32) ** 2 + ((v - 74) / 20) ** 2 < 1);
  return outer && !inner;
};

export const ICONS = {
  /* Butterfly — the harness. Coral: the cache hit on the upper wing. */
  butterfly: {
    layers: [
      {
        tone: "lilac",
        shapes: [
          ellipse(33, 42, 27, 18, -38),
          ellipse(87, 42, 27, 18, 38),
          ellipse(39, 84, 17, 12, 35),
          ellipse(81, 84, 17, 12, -35),
        ],
      },
    ],
    accent: [ellipse(92, 38, 13, 7, 38)],
    dark: [
      rect(55, 30, 10, 66),
      poly([[56, 33], [61, 33], [46, 10], [41, 10]]),
      poly([[59, 33], [64, 33], [79, 10], [74, 10]]),
    ],
  },
  /* Cocoon — the credential firewall. Coral: the key, kept inside. */
  cocoon: {
    layers: [{ tone: "sage", shapes: [ellipse(60, 60, 30, 48, -18)] }],
    accent: [ellipse(60, 60, 11, 11)],
    bands: true,
  },
  /* Nimble — the router. Coral: the route it chose. */
  nimble: {
    layers: [
      {
        tone: "blue",
        shapes: [
          rect(6, 52, 54, 16),
          rect(56, 53, 58, 14),
          poly([[56, 54], [70, 58], [86, 80], [114, 88], [114, 104], [80, 96], [64, 74], [54, 68]]),
        ],
      },
    ],
    accent: [poly([[54, 52], [64, 46], [80, 24], [114, 16], [114, 32], [86, 40], [70, 62], [56, 66]])],
  },
  /* Inver — the gateway, drawn as Monet's footbridge arch. Coral: the request passing under it. */
  inver: {
    layers: [{ tone: "green", shapes: [arch, rect(4, 104, 112, 10)] }],
    accent: [rect(38, 82, 44, 14)],
  },
  /* Cicada — the token ledger. Coral: the balance still left on the wing. */
  cicada: {
    layers: [
      { tone: "sand", shapes: [ellipse(38, 74, 13, 40, 24), ellipse(82, 74, 13, 40, -24), ellipse(60, 62, 11, 32)] },
    ],
    accent: [poly([[84, 86], [100, 80], [104, 92], [88, 98]])],
    dark: [ellipse(60, 26, 20, 11), rect(57, 40, 6, 50)],
  },
} satisfies Record<string, ArtSpec>;

/* The studio logo, drawn on a 16-pixel grid so it stays crisp at 2× in the nav. */
export const LOGOS = {
  /* A water lily opening on its pad. Coral: the heart of the flower. */
  lily: {
    grid: 16,
    separate: true,
    layers: [
      { tone: "green", shapes: [ellipse(60, 88, 56, 22)], cut: [poly([[60, 88], [118, 76], [118, 100]])] },
      {
        tone: "pink",
        shapes: [
          ellipse(60, 46, 13, 32),
          ellipse(36, 62, 11, 26, -40),
          ellipse(84, 62, 11, 26, 40),
          ellipse(20, 78, 9, 16, -70),
          ellipse(100, 78, 9, 16, 70),
        ],
      },
    ],
    accent: [ellipse(60, 76, 11, 8)],
  },
} satisfies Record<string, ArtSpec>;
