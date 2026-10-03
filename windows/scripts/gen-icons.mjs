
import { build } from "esbuild";
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync, unlinkSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, "..", "src-tauri", "icons");
const SKIN_TS = join(HERE, "..", "src", "bot", "skin.ts");
const BUNDLE = join(HERE, ".skin.bundle.mjs");

await build({ entryPoints: [SKIN_TS], bundle: true, format: "esm", outfile: BUNDLE, logLevel: "warning" });
const { ISKRA } = await import(pathToFileURL(BUNDLE).href);

const SS = 4; // supersampling factor
const RIM = 0.05; // outline thickness, in R units
const RIM_COLOR = [4, 14, 30];

const [BX, BY] = ISKRA.extents(0);
const TOP = ISKRA.top.map((v) => v * 255);
const BOTTOM = ISKRA.bottom.map((v) => v * 255);
const INK = [0, 1, 2].map((i) => parseInt(ISKRA.ink.slice(1 + i * 2, 3 + i * 2), 16));

const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);

/** Boundary radius of the outline at angle `a`, in R units. */
function edge(a) {
  const [x, y] = ISKRA.hull(a);
  return Math.hypot(x, y);
}

/** True when a point, in pixels from the centre, is inside the outline grown by `grow` (R units). */
function inside(x, y, R, grow = 0) {
  return Math.hypot(x, y) / R <= edge(Math.atan2(y, x)) + grow;
}

function render(size) {
  const px = new Uint8Array(size * size * 4);
  const R = size * 0.34;
  const cx = size / 2;
  const cy = size / 2;
  const rx = BX * R;
  const ry = BY * R;

  // Body gradient, same direction as the app: top-right to bottom-left.
  const g0 = [rx * 0.7, -ry * 0.85];
  const g1 = [-rx * 0.8, ry * 0.9];
  const gd = [g1[0] - g0[0], g1[1] - g0[1]];
  const glen = gd[0] * gd[0] + gd[1] * gd[1];

  const eyeSp = Math.sin(ISKRA.eyes.sp) * rx;
  const eyeR = [R * ISKRA.eyes.w * 0.5, R * ISKRA.eyes.h * 0.5];
  const eyeY = -Math.sin(ISKRA.eyes.pitch) * ry;

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let rimHits = 0;
      let bodyHits = 0;
      let eyeHits = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const lx = x + (sx + 0.5) / SS - cx;
          const ly = y + (sy + 0.5) / SS - cy;
          if (!inside(lx, ly, R, RIM)) continue;
          rimHits++;
          if (!inside(lx, ly, R)) continue;
          bodyHits++;
          for (const sd of [-1, 1]) {
            const dx = (lx - sd * eyeSp) / eyeR[0];
            const dy = (ly - eyeY) / eyeR[1];
            if (dx * dx + dy * dy <= 1) {
              eyeHits++;
              break;
            }
          }
        }
      }
      if (rimHits === 0) continue;

      const total = SS * SS;
      let col = RIM_COLOR;
      const bodyA = bodyHits / rimHits;
      if (bodyA > 0) {
        const t = Math.max(0, Math.min(1, ((x - cx - g0[0]) * gd[0] + (y - cy - g0[1]) * gd[1]) / glen));
        col = mix(mix(RIM_COLOR, mix(TOP, BOTTOM, t), bodyA), INK, eyeHits / rimHits);
      }
      const o = (y * size + x) * 4;
      px[o] = Math.round(col[0]);
      px[o + 1] = Math.round(col[1]);
      px[o + 2] = Math.round(col[2]);
      px[o + 3] = Math.round((rimHits / total) * 255);
    }
  }
  return px;
}

// ── PNG ───────────────────────────────────────────────────────────────────────

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePNG(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    Buffer.from(rgba.buffer, y * size * 4, size * 4).copy(raw, y * (size * 4 + 1) + 1);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ── ICO (PNG-in-ICO, Vista and later) ─────────────────────────────────────────

function encodeICO(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  const dir = Buffer.alloc(16 * entries.length);
  let offset = header.length + dir.length;
  entries.forEach((e, i) => {
    const o = i * 16;
    dir[o] = e.size >= 256 ? 0 : e.size;
    dir[o + 1] = e.size >= 256 ? 0 : e.size;
    dir[o + 2] = 0;
    dir[o + 3] = 0;
    dir.writeUInt16LE(1, o + 4);
    dir.writeUInt16LE(32, o + 6);
    dir.writeUInt32LE(e.png.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += e.png.length;
  });
  return Buffer.concat([header, dir, ...entries.map((e) => e.png)]);
}

// ── Go ────────────────────────────────────────────────────────────────────────

mkdirSync(OUT, { recursive: true });

const png = (size) => encodePNG(size, render(size));

const files = {
  "32x32.png": png(32),
  "128x128.png": png(128),
  "128x128@2x.png": png(256),
  "icon.png": png(512),
};
for (const [name, data] of Object.entries(files)) {
  writeFileSync(join(OUT, name), data);
  console.log(`${name} — ${data.length} bytes`);
}

const ico = encodeICO([16, 24, 32, 48, 64, 128, 256].map((size) => ({ size, png: png(size) })));
writeFileSync(join(OUT, "icon.ico"), ico);
console.log(`icon.ico — ${ico.length} bytes`);

unlinkSync(BUNDLE);
