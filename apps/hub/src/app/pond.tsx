"use client";

/* The hero scene: Monet's pond at Giverny in pixel art. Pines on both banks, clouds behind, lily
 * pads on the water, the green footbridge over the gap, and a small robot at an easel painting a
 * sunrise — the studio's idea in one picture: a model holding the brush.
 *
 * Drawn at a low logical resolution and scaled by a whole number with nearest-neighbour sampling,
 * so every pixel stays square. The static scene is painted once per size and seed; only the water
 * shimmer and the robot's brush arm change between frames. Colours come from tokens.css (--px-*). */

import { useEffect, useRef } from "react";

const SEED = 1872;
const FRAME_MS = 650;

type Pal = Record<string, string>;
type Ctx = CanvasRenderingContext2D;

const KEYS = [
  "outline", "cloud", "cloud-shade", "pine-l", "pine-m", "pine-d", "trunk", "tuft", "bank-top", "bank",
  "bank-d", "water", "water-l", "water-d", "pad", "pad-d", "pink", "coral", "coral-l", "bridge", "bridge-l",
  "white", "grey", "screen",
];

function readPalette(): Pal {
  const css = getComputedStyle(document.documentElement);
  return Object.fromEntries(KEYS.map((k) => [k, css.getPropertyValue(`--px-${k}`).trim()]));
}

function rng(seed: number) {
  let s = seed | 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function scaleFor(width: number) {
  return width >= 1100 ? 4 : 3;
}

interface Layout {
  W: number;
  H: number;
  water: number; // first water row
  bankTop: number;
  L: number; // left bank right edge
  R: number; // right bank left edge
  narrow: boolean;
  robotX: number;
  easelX: number;
  rise: number; // bridge arch height
}

const ROBOT_W = 14;
const EASEL_W = 16;

function layoutFor(W: number, H: number): Layout {
  const water = H - 22;
  const narrow = W < 200;
  const L = Math.round(W * (narrow ? 0.36 : 0.42));
  const R = Math.round(W * (narrow ? 0.64 : 0.58));
  const rise = Math.max(6, Math.round((R - L + 6) * 0.26));
  /* The painter stands by the bridge: on the right bank when there is room, otherwise on the left. */
  const robotX = narrow ? L - ROBOT_W - EASEL_W - 6 : R + 6;
  const easelX = robotX + ROBOT_W + 3;
  return { W, H, water, bankTop: water - 5, L, R, narrow, robotX, easelX, rise };
}

function px(ctx: Ctx, colour: string, x: number, y: number, w = 1, h = 1) {
  ctx.fillStyle = colour;
  ctx.fillRect(x, y, w, h);
}

/* A mask painter: fill a shape, then shade by its edges — light on top-left, dark on bottom-right,
 * outline around — the same rule the icons use. */
function paintMask(
  ctx: Ctx,
  mask: Uint8Array,
  W: number,
  H: number,
  colour: (x: number, y: number, edge: { up: boolean; left: boolean; down: boolean; right: boolean }) => string | null,
  outline?: string,
) {
  const at = (x: number, y: number) => x >= 0 && y >= 0 && x < W && y < H && mask[y * W + x] === 1;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      if (at(x, y)) {
        const c = colour(x, y, { up: !at(x, y - 1), left: !at(x - 1, y), down: !at(x, y + 1), right: !at(x + 1, y) });
        if (c) px(ctx, c, x, y);
      } else if (outline && (at(x - 1, y) || at(x + 1, y) || at(x, y - 1) || at(x, y + 1))) {
        px(ctx, outline, x, y);
      }
    }
  }
}

function drawClouds(ctx: Ctx, l: Layout, p: Pal, r: () => number) {
  const { W, H, bankTop } = l;
  const mask = new Uint8Array(W * H);
  const count = Math.max(3, Math.round(W / 55));
  for (let i = 0; i < count; i++) {
    const cx = ((i + 0.5) * W) / count + (r() - 0.5) * 30;
    const cy = bankTop - 26 - r() * 22;
    for (let k = 0; k < 7; k++) {
      const ox = cx + (r() - 0.5) * 44;
      const oy = cy + (r() - 0.5) * 16;
      const rad = 7 + r() * 13;
      for (let y = Math.floor(oy - rad); y <= oy + rad; y++) {
        for (let x = Math.floor(ox - rad); x <= ox + rad; x++) {
          if (x < 0 || x >= W || y < 0 || y >= bankTop) continue;
          if ((x - ox) ** 2 + (y - oy) ** 2 <= rad * rad) mask[y * W + x] = 1;
        }
      }
    }
  }
  const at = (x: number, y: number) => x >= 0 && y >= 0 && x < W && y < H && mask[y * W + x] === 1;
  paintMask(ctx, mask, W, H, (x, y) => {
    /* Shade the underside of each puff, with a dithered transition. */
    const under = !at(x, y + 3) || (!at(x, y + 5) && (x + y) % 2 === 0);
    return under ? p["cloud-shade"] : p.cloud;
  });
}

const hash = (x: number, y: number) => {
  let h = (x * 374761393 + y * 668265263) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};

/* A pine of drooping tiers: each tier widens downward and ends in a row of hanging needle tips.
 * Shading is clustered (2px cells) so foliage reads as texture, not noise. Back-row pines are
 * paler, which gives the scene depth. */
function drawPine(ctx: Ctx, l: Layout, p: Pal, r: () => number, cx: number, h: number, back: boolean) {
  const { W, H, bankTop } = l;
  const base = bankTop;
  const top = base - h;
  const crownBottom = base - 6;
  const tiers = 3 + Math.floor(h / 18);
  const maxHalf = Math.max(5, Math.round(h * 0.27));

  for (let y = crownBottom - 2; y < base; y++) {
    px(ctx, p.outline, cx - 2, y);
    px(ctx, p.trunk, cx - 1, y, 3, 1);
    px(ctx, p.outline, cx + 2, y);
  }

  const mask = new Uint8Array(W * H);
  const tierPos = new Float32Array(W * H);
  const set = (x: number, y: number, f: number) => {
    if (x < 0 || x >= W || y < 0 || y >= H) return;
    mask[y * W + x] = 1;
    tierPos[y * W + x] = f;
  };
  const span = crownBottom - top;
  for (let k = 0; k < tiers; k++) {
    const y0 = top + Math.round((span * k) / tiers) - (k ? 2 : 0);
    const y1 = top + Math.round((span * (k + 1)) / tiers);
    /* Each tier starts well inside the one above ends, so the silhouette steps like a pine. */
    const wTop = Math.max(1, Math.round(maxHalf * (0.06 + (0.5 * k) / tiers)));
    const wBot = Math.round(maxHalf * (0.32 + (0.68 * (k + 1)) / tiers));
    for (let y = y0; y <= y1; y++) {
      const f = (y - y0) / Math.max(1, y1 - y0);
      const half = Math.round(wTop + (wBot - wTop) * f) + (r() < 0.35 ? 1 : 0) - (r() < 0.15 ? 1 : 0);
      for (let x = cx - half; x <= cx + half; x++) set(x, y, f);
    }
    // hanging needle tips under the tier
    for (let x = cx - wBot + 1; x <= cx + wBot - 1; x += 2) {
      const drop = 1 + Math.floor(r() * 2);
      for (let d = 1; d <= drop; d++) set(x, y1 + d, 1);
    }
  }

  const light = p["pine-l"];
  const mid = back ? p["pine-l"] : p["pine-m"];
  const dark = back ? p["pine-m"] : p["pine-d"];
  paintMask(
    ctx,
    mask,
    W,
    H,
    (x, y, e) => {
      const f = tierPos[y * W + x];
      const n = hash(x >> 1, y >> 1);
      if (e.down || (x > cx + 1 && n < 0.55) || f > 0.88) return dark;
      if (x <= cx && f < 0.7 && n > 0.35) return light;
      return n > 0.8 ? light : mid;
    },
    p.outline,
  );
}

function drawBush(ctx: Ctx, l: Layout, p: Pal, cx: number, w: number) {
  const { W, H, bankTop } = l;
  const mask = new Uint8Array(W * H);
  const hgt = Math.max(3, Math.round(w * 0.45));
  for (let y = bankTop - hgt; y < bankTop; y++) {
    const t = (bankTop - y) / hgt;
    const half = Math.round(w * Math.sqrt(Math.max(0, 1 - t * t)));
    for (let x = cx - half; x <= cx + half; x++) if (x >= 0 && x < W) mask[y * W + x] = 1;
  }
  paintMask(
    ctx,
    mask,
    W,
    H,
    (x, y, e) => {
      const n = hash(x >> 1, y >> 1);
      if (e.down || (x > cx && n < 0.5)) return p["pine-d"];
      return x < cx && n > 0.4 ? p["pine-l"] : p["pine-m"];
    },
    p.outline,
  );
}

function drawBanks(ctx: Ctx, l: Layout, p: Pal, r: () => number) {
  const { W, water, bankTop, L, R } = l;
  const spans: [number, number][] = [
    [2, L],
    [R, W - 3],
  ];
  for (const [a, b] of spans) {
    for (let x = a; x <= b; x++) {
      px(ctx, r() < 0.15 ? p.bank : p["bank-top"], x, bankTop, 1, 2);
      for (let y = bankTop + 2; y < water; y++) px(ctx, r() < 0.2 ? p["bank-d"] : p.bank, x, y);
    }
    px(ctx, p.outline, a - 1, bankTop, 1, water - bankTop);
    px(ctx, p.outline, b + 1, bankTop, 1, water - bankTop);
    px(ctx, p.outline, a, bankTop - 1, b - a + 1, 1);
  }
  px(ctx, p.outline, 0, water, W, 1);
}

function drawTufts(ctx: Ctx, l: Layout, p: Pal, r: () => number, a: number, b: number) {
  for (let x = a; x <= b; x += 2 + Math.floor(r() * 4)) {
    const blades = 2 + Math.floor(r() * 3);
    for (let i = 0; i < blades; i++) {
      const bh = 1 + Math.floor(r() * 4);
      px(ctx, p.tuft, x + i, l.bankTop - 1 - bh, 1, bh);
      px(ctx, p.outline, x + i, l.bankTop - 2 - bh);
    }
  }
}

function drawFlower(ctx: Ctx, l: Layout, p: Pal, x: number) {
  const y = l.bankTop - 1;
  const rows = [".ooo.", "occco", "ocwco", "occco", ".oto.", "..t..", ".tt.."];
  const map: Record<string, string> = { o: p.outline, c: p.coral, w: p["coral-l"], t: p.tuft };
  rows.forEach((row, j) => {
    for (let i = 0; i < row.length; i++) {
      const c = map[row[i]];
      if (c) px(ctx, c, x - 2 + i, y - rows.length + 1 + j);
    }
  });
}

function drawBridge(ctx: Ctx, l: Layout, p: Pal) {
  const { water, bankTop, L, R } = l;
  const xa = L - 3;
  const xb = R + 3;
  const span = xb - xa;
  const rise = l.rise;
  const deck = (x: number) => bankTop - Math.round(rise * Math.sin((Math.PI * (x - xa)) / span));
  for (let x = xa; x <= xb; x++) {
    const y = deck(x);
    px(ctx, p.outline, x, y - 1);
    px(ctx, p["bridge-l"], x, y);
    px(ctx, p.bridge, x, y + 1);
    px(ctx, p.outline, x, y + 2);
    const rail = y - 6;
    px(ctx, p.outline, x, rail - 1);
    px(ctx, p["bridge-l"], x, rail);
    if ((x - xa) % 5 === 0) px(ctx, p.bridge, x, rail + 1, 1, 4);
  }
  for (const x of [xa + 3, xb - 3]) px(ctx, p.outline, x, deck(x) + 3, 1, water - deck(x) - 3);
}

const ROBOT = [
  ".......o......",
  "......oco.....",
  "...oooooooo...",
  "..owwwwwwwwo..",
  "..owsswwsswo..",
  "..owsswwsswo..",
  "..owwwwwwwwo..",
  "..owwwccwwwo..",
  "...oooooooo...",
  "....oggggo....",
  "..oooooooooo..",
  ".oggggggggggo.",
  ".oggwwwwwwggo.",
  ".oggwccwwwggo.",
  ".oggwwwwwwggo.",
  ".oggggggggggo.",
  "..oooooooooo..",
  "...og....go...",
  "...og....go...",
  "..ooo....ooo..",
];

/* The easel's canvas is a tiny "Impression, Sunrise": coral sun, its reflection, a dark boat. */
const EASEL = [
  "oooooooooooooooo",
  "okkkkkkkkkkkkkko",
  "okkkkkkkkkcckkko",
  "okkkkkkkkkcckkko",
  "okkkkkkkkkkkkkko",
  "osssssssssssssso",
  "osssssssssscssso",
  "ossooossssssssso",
  "osssssssssscssso",
  "oooooooooooooooo",
  "....o......o....",
  "....o......o....",
  "...o........o...",
  "...o........o...",
  "..o..........o..",
  "..o..........o..",
  ".o............o.",
  ".o............o.",
];

function sprite(ctx: Ctx, p: Pal, rows: string[], x: number, y: number) {
  const map: Record<string, string> = {
    o: p.outline,
    c: p.coral,
    w: p.white,
    g: p.grey,
    s: p.screen,
    k: p["water-l"],
  };
  rows.forEach((row, j) => {
    for (let i = 0; i < row.length; i++) {
      const c = map[row[i]];
      if (c) px(ctx, c, x + i, y + j);
    }
  });
}

/* The brush arm reaches from the robot's shoulder toward the easel; two frames make it paint. */
function drawRobotArm(ctx: Ctx, l: Layout, p: Pal, frame: number) {
  const x = l.robotX;
  const y = l.bankTop - ROBOT.length;
  const arm =
    frame % 2 === 0
      ? [[13, 12], [14, 11], [15, 10], [16, 9]]
      : [[13, 12], [14, 12], [15, 11], [16, 11]];
  for (const [ax, ay] of arm) px(ctx, p.outline, x + ax, y + ay);
  const [tx, ty] = arm[arm.length - 1];
  px(ctx, p.coral, x + tx + 1, y + ty - 1, 2, 2);
}

function drawPads(ctx: Ctx, l: Layout, p: Pal, r: () => number) {
  const { W, H, water } = l;
  const count = Math.max(3, Math.round(W / 34));
  for (let i = 0; i < count; i++) {
    const x = Math.round(4 + r() * (W - 10));
    const y = Math.round(water + 6 + r() * (H - water - 10));
    px(ctx, p["pad-d"], x - 3, y - 1, 6, 1);
    px(ctx, p.pad, x - 4, y, 9, 1);
    px(ctx, p.pad, x - 4, y + 1, 3, 1);
    px(ctx, p.pad, x, y + 1, 5, 1);
    px(ctx, p["pad-d"], x - 3, y + 2, 7, 1);
    if (r() < 0.5) {
      px(ctx, p.pink, x - 1, y - 3, 3, 2);
      px(ctx, p.white, x, y - 4);
    }
  }
}

interface Shimmer {
  x: number;
  y: number;
  w: number;
}

function paintStatic(ctx: Ctx, l: Layout, p: Pal) {
  const r = rng(SEED);
  const { W, H, water, bankTop, L, R } = l;
  ctx.clearRect(0, 0, W, H);
  drawClouds(ctx, l, p, r);

  px(ctx, p.water, 0, water + 1, W, H - water - 1);
  px(ctx, p["water-d"], 0, water + 1, W, 1);

  /* Trees stay on the outer thirds so the centre is open sky over the bridge and the painter. */
  const painterA = l.robotX - 4;
  const painterB = l.easelX + EASEL_W + 4;
  const groves: [number, number][] = l.narrow
    ? [[R + 8, W - 6]]
    : [
        [6, Math.round(W * 0.27)],
        [Math.max(Math.round(W * 0.73), painterB + 8), W - 6],
      ];
  for (const back of [true, false]) {
    for (const [a, b] of groves) {
      if (b - a < 8) continue;
      let x = a + (back ? 10 : 0);
      while (x <= b) {
        const h = back ? 40 + Math.floor(r() * 20) : 54 + Math.floor(r() * 30);
        /* Narrow screens keep the grove low so it never sits under the buttons. */
        drawPine(ctx, l, p, r, x, Math.min(h, bankTop - 6, l.narrow ? 42 : Infinity), back);
        x += back ? 22 + Math.floor(r() * 10) : 16 + Math.floor(r() * 9);
      }
    }
  }
  if (!l.narrow) {
    drawBush(ctx, l, p, Math.round(W * 0.3), 7);
    drawBush(ctx, l, p, Math.round(W * 0.35), 5);
    drawBush(ctx, l, p, Math.round(W * 0.71), 6);
  }

  drawBanks(ctx, l, p, r);
  const tuftsAround = (a: number, b: number) => {
    if (b - a > 2) drawTufts(ctx, l, p, r, a, b);
  };
  if (l.narrow) {
    tuftsAround(3, painterA);
    tuftsAround(R + 1, W - 4);
  } else {
    tuftsAround(3, L - 1);
    tuftsAround(R + 1, painterA);
    tuftsAround(painterB, W - 4);
  }
  drawFlower(ctx, l, p, l.narrow ? Math.round(W * 0.82) : Math.round(W * 0.33));
  if (!l.narrow) drawFlower(ctx, l, p, Math.round(W * 0.78));
  drawBridge(ctx, l, p);
  drawPads(ctx, l, p, r);

  sprite(ctx, p, ROBOT, l.robotX, bankTop - ROBOT.length);
  sprite(ctx, p, EASEL, l.easelX, bankTop - EASEL.length);

  const shimmer: Shimmer[] = [];
  const count = Math.round(W / 6);
  for (let i = 0; i < count; i++) {
    shimmer.push({ x: Math.floor(r() * W), y: water + 3 + Math.floor(r() * (H - water - 4)), w: 2 + Math.floor(r() * 7) });
  }
  return shimmer;
}

export function PondScene() {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const wrap = wrapRef.current;
    const canvas = canvasRef.current;
    if (!wrap || !canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const base = document.createElement("canvas");
    let timer = 0;
    let frame = 0;
    let width = 0;

    const render = () => {
      const S = scaleFor(wrap.clientWidth);
      const W = Math.ceil(wrap.clientWidth / S);
      const H = Math.max(96, Math.min(140, Math.round((window.innerHeight * 0.5) / S)));
      const l = layoutFor(W, H);
      const p = readPalette();
      base.width = W;
      base.height = H;
      const bctx = base.getContext("2d");
      if (!bctx) return;
      const shimmer = paintStatic(bctx, l, p);

      canvas.width = W;
      canvas.height = H;
      canvas.style.width = `${W * S}px`;
      canvas.style.height = `${H * S}px`;
      wrap.style.height = `${H * S}px`;
      /* The hero keeps its text above the bridge and the painter, whatever the screen height. */
      const clear = (H - l.bankTop + Math.max(l.rise + 8, ROBOT.length) + 6) * S;
      wrap.parentElement?.style.setProperty("--scene-clear", `${clear}px`);

      const tick = () => {
        ctx.clearRect(0, 0, W, H);
        ctx.drawImage(base, 0, 0);
        for (const s of shimmer) {
          const x = (s.x + (still ? 0 : frame)) % (W + 10);
          px(ctx, p["water-l"], x - 5, s.y, s.w, 1);
        }
        drawRobotArm(ctx, l, p, frame);
        frame++;
      };
      window.clearInterval(timer);
      tick();
      if (!still) timer = window.setInterval(tick, FRAME_MS);
    };

    render();
    width = window.innerWidth;
    const onResize = () => {
      if (window.innerWidth === width) return;
      width = window.innerWidth;
      render();
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      window.clearInterval(timer);
    };
  }, []);

  return (
    <div ref={wrapRef} className="scene">
      <canvas
        ref={canvasRef}
        className="scene__canvas"
        role="img"
        aria-label="Pixel art of Monet's pond: pines on both banks, lily pads, a green footbridge, and a small robot painting a sunrise at an easel."
      />
    </div>
  );
}
