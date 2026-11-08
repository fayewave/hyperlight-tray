// Draws the key in the same text-first style as the Aircon plugin's keys (after the
// Apple Watch Modular complications), turned upside down below the name: "Hyperlight"
// top left in white, a small caps line under it, then the percent as the biggest thing
// on the key and a thin 0-100% bar along the bottom (green, orange or red by level, or
// blue while charging, up to a dot at the level, grey after).
//
// Keys are drawn in a 96-unit box (one unit = one Stream Deck XL pixel) and handed to
// Stream Deck as a 144px SVG. Stream Deck's SVG renderer is basic, so: no CSS, no
// filters, no tspan offsets, and only installed fonts.
import fs from "node:fs";
import path from "node:path";

/** Apple's system colours, as on the Aircon keys. */
/** charging: an electric blue, deeper than the update dot's light blue. */
export const COLORS = { healthy: "#30D158", moderate: "#FF9F0A", low: "#FF453A", stale: "#8E8E93", charging: "#1E9BFF" };
const GREY = "#8E8E93";
const DIM = "#636366";
const TRACK = "#2C2C2E";
const ACCENT = "#64D2FF";
/** Asleep: the number, bar, dot and caption dimmed, like a switched-off AC. */
const OFF = { number: "#6C6C70", dot: "#77777C", fade: 0.18, text: "#636366" };

export function tone(percent) {
  return percent > 25 ? "healthy" : percent >= 20 ? "moderate" : "low";
}

/**
 * The key's own options (its settings in Stream Deck). The text lines are off until
 * turned on, so by default the line under the name is empty; so is the update dot.
 */
export function options(settings) {
  return {
    remaining: settings?.showRemaining === true,
    charging: settings?.showCharging === true,
    asleep: settings?.showAsleep === true,
    update: settings?.showUpdate === true,
  };
}

/**
 * What to show for a tray reply (see tray.js); undefined means not read yet.
 * { percent, color, caption, dim, dot } for a reading (color: the number's
 * colour when low, else null for white), or
 * { word, caption } for a key with no number ("Tray off / Start").
 */
export function view(s, opts = options()) {
  const dot = Boolean(s?.update) && opts.update;
  switch (s?.state) {
    case "battery": {
      const caption = s.charging && opts.charging ? "Charging" : !s.charging && opts.remaining && s.remaining ? s.remaining : "";
      // Charging turns the number and bar blue whether or not the Charging text is on.
      const t = s.charging ? "charging" : tone(s.percent);
      return { percent: s.percent ?? null, color: t === "healthy" ? null : COLORS[t], caption, dot };
    }
    case "asleep":
      return { percent: s.percent ?? null, color: null, caption: opts.asleep ? "Asleep" : "", dim: true, dot };
    case "noReceiver":
      return { percent: null, color: GREY, caption: "No receiver", dot };
    case "stopped":
      return { word: "Start", caption: "Tray off" };
    case "outdated":
      return { word: "Update", caption: "Old tray" };
    case "missing":
      return { word: "Get", caption: "No tray" };
    default: // "checking", or before the first reply
      return { percent: null, color: null, caption: "", dim: true };
  }
}

/* ---------- fonts (the same pick and widths as the Aircon plugin's draw.js) ---------- */
/** Nunito (free, rounded, close to Apple's SF Rounded) when installed, else Segoe UI. */
export function pickFont(dirs = fontDirs()) {
  for (const dir of dirs) {
    try {
      if (fs.readdirSync(dir).some((f) => /^nunito/i.test(f))) return "Nunito";
    } catch {
      // folder missing
    }
  }
  return "Segoe UI";
}
/**
 * The battery number's font: Optician Sans when installed (a free font after the
 * letters on eye charts), else null for the same font as the rest of the key.
 */
export function pickNumberFont(dirs = fontDirs()) {
  for (const dir of dirs) {
    try {
      if (fs.readdirSync(dir).some((f) => /^optician/i.test(f))) return "Optician Sans";
    } catch {
      // folder missing
    }
  }
  return null;
}
function fontDirs() {
  const dirs = [path.join(process.env.WINDIR ?? "C:\\Windows", "Fonts")];
  if (process.env.LOCALAPPDATA) dirs.push(path.join(process.env.LOCALAPPDATA, "Microsoft", "Windows", "Fonts"));
  return dirs;
}
let FONT = "Segoe UI";
export const setFont = (f) => (FONT = f);
let NUM_FONT = null;
export const setNumberFont = (f) => (NUM_FONT = f);

/*
 * Stream Deck can't measure text for us, so: advance widths (thousandths of the font
 * size) of printable ASCII plus … ’ °, measured in Chrome. Segoe UI's 800 and 900 both
 * come out as Segoe UI Black. Unknown characters count as wide.
 */
const CHARS = [...Array(95)].map((_, i) => String.fromCharCode(32 + i)).join("") + "\u2026\u2019\u00b0";
const table = (csv) => new Map(csv.split(",").map((w, i) => [CHARS[i], Number(w) / 1000]));
const SEGOE = table("278,366,580,599,599,898,739,344,421,421,484,715,320,407,320,482,599,483,599,599,621,599,599,599,599,599,320,320,715,715,715,423,949,750,691,630,759,552,541,727,802,349,511,690,541,987,812,765,656,765,682,572,636,740,702,1053,702,650,641,421,482,421,715,415,347,561,642,495,642,560,431,642,617,308,308,593,308,950,626,634,642,642,436,483,417,626,588,845,621,574,509,421,398,421,715,1046,339,386");
const WIDTHS = {
  "Segoe UI": { 800: SEGOE, 900: SEGOE },
  Nunito: {
    800: table("279,260,481,600,600,954,745,256,382,382,454,600,260,440,260,331,600,600,600,600,600,600,600,600,600,600,260,260,600,600,600,468,952,753,695,684,774,605,570,742,780,297,372,688,573,876,753,796,664,796,696,641,632,743,727,1120,685,631,615,377,331,377,600,500,388,557,610,478,610,549,382,615,595,268,273,558,333,889,595,589,610,610,412,492,404,590,534,861,559,533,480,414,302,414,600,780,260,384"),
    900: table("286,272,515,600,600,964,766,269,407,407,456,600,272,445,272,349,600,600,600,600,600,600,600,600,600,600,272,272,600,600,600,478,954,763,702,688,786,614,579,747,786,312,390,712,585,884,758,807,676,807,706,651,644,748,742,1128,698,645,625,401,349,401,600,500,399,568,620,484,620,555,402,626,606,281,287,580,348,902,606,602,620,620,434,496,426,601,542,868,572,541,487,438,316,438,600,816,272,389"),
  },
};
/** Optician Sans comes in one weight; widths of what the number line can show (from its hmtx table). */
const OPTICIAN = new Map(Object.entries({ 0: 0.6, 1: 0.55, 2: 0.55, 3: 0.55, 4: 0.551, 5: 0.55, 6: 0.55, 7: 0.515, 8: 0.55, 9: 0.55, "%": 0.77, "–": 0.8 }));
const opticianWidth = (str, size) => [...String(str)].reduce((w, ch) => w + (OPTICIAN.get(ch) ?? 0.8), 0) * size;

/** Width of str in the current font. spacing = letter-spacing, added after each character. */
export function textWidth(str, size, { weight = 800, spacing = 0 } = {}) {
  const t = (WIDTHS[FONT] ?? WIDTHS["Segoe UI"])[weight >= 850 ? 900 : 800];
  let w = 0;
  for (const ch of String(str)) w += t.get(ch) ?? 0.9;
  return w * size + spacing * [...String(str)].length;
}

/** str cut to fit max units wide, ending in "…" when it had to be cut. */
function truncate(str, size, max, opts = {}) {
  str = String(str);
  if (textWidth(str, size, opts) <= max) return str;
  let chars = [...str];
  while (chars.length > 1 && textWidth(`${chars.join("").trimEnd()}\u2026`, size, opts) > max) chars = chars.slice(0, -1);
  return `${chars.join("").trimEnd()}\u2026`;
}

/* ---------- primitives ---------- */
const f2 = (n) => Number(n.toFixed(2));
const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const text = (x, y, str, { size = 14, weight = 800, fill = "#fff", spacing = 0, family = FONT } = {}) =>
  `<text x="${f2(x)}" y="${f2(y)}" font-family="${family}" font-size="${size}" font-weight="${weight}" fill="${fill}" ` +
  `text-anchor="start"${spacing ? ` letter-spacing="${spacing}"` : ""}>${esc(str)}</text>`;
const wrap = (body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 96 96">` +
  `<rect width="96" height="96" fill="#000"/>${body}</svg>`;

/* ---------- the layout (the Aircon plugin's modular.js, upside down below the name) ---------- */
const X = 9; // left edge of text and bar
const LABEL_Y = 42; // baseline of the caps line under the name
const NUM_Y = 76; // baseline of the big number
const BAR = { y: 84, w: 78, h: 5 };
/** The name keeps the Aircon keys' 17px when it fits, else shrinks to fit the width. */
const NAME = "Hyperlight";
const NAME_MAX = 79;

function name(color) {
  const size = Math.min(17, Math.floor((NAME_MAX / textWidth(NAME, 1)) * 2) / 2);
  return text(X, 23, NAME, { size, fill: color });
}
/** The line under the name: 13px caps, down to 11.5px to fit, then cut with "…". */
function label(str, color, max) {
  str = String(str).toUpperCase();
  const o = { weight: 900, spacing: 0.4 };
  let size = 13;
  while (size > 11.5 && textWidth(str, size, o) > max) size -= 0.5;
  return text(X, LABEL_Y, truncate(str, size, max, o), { size, ...o, fill: color });
}

/** The 0-100% bar: green (orange or red when low) up to the level, the grey track after it, and a white dot at the level. */
function bar(percent, dim, color = COLORS.healthy) {
  const track = `<rect x="${X}" y="${BAR.y}" width="${BAR.w}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${TRACK}"/>`;
  if (percent == null) return track;
  const p = Math.max(0, Math.min(100, percent)) / 100;
  let s = track;
  if (p > 0) s += `<g opacity="${dim ? OFF.fade : 1}"><rect x="${X}" y="${BAR.y}" width="${f2(Math.max(BAR.h, BAR.w * p))}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${color}"/></g>`;
  s += `<circle cx="${f2(X + BAR.w * p)}" cy="${BAR.y + BAR.h / 2}" r="4.4" fill="${dim ? OFF.dot : "#fff"}" stroke="#000" stroke-width="2.2"/>`;
  return s;
}

/** A 144×144 SVG for the key. */
export function keySvg(v) {
  let s = "";
  // "Update available": a blue dot at the right end of the line under the name (which then stops short of it).
  const labelMax = v.dot ? 68 : 80;
  if (v.word != null) {
    s += name("#fff");
    s += label(v.caption, GREY, labelMax);
    const size = Math.min(24, Math.floor(79 / textWidth(v.word, 1, { weight: 900 })));
    s += text(X - 1, NUM_Y, v.word, { size, weight: 900, fill: GREY });
    s += `<rect x="${X}" y="${BAR.y}" width="${BAR.w}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${TRACK}"/>`;
  } else {
    const num = v.percent == null ? "–" : String(v.percent);
    s += name("#fff");
    if (v.caption) s += label(v.caption, v.dim ? OFF.text : GREY, labelMax);
    // The number and its % in Optician Sans when installed (Regular only, so weight 400:
    // asking for bold would get a smeared fake bold), else the key's own font.
    const nf = NUM_FONT ? { family: NUM_FONT, weight: 400 } : { weight: 900 };
    // Optician Sans spaces its digits wide, so they're pulled 6% of the size closer. Stream
    // Deck ignores negative letter-spacing, so each digit is placed on its own.
    const tight = (size) => (NUM_FONT ? -0.06 * size : 0);
    const numWidth = (str, size) => (NUM_FONT ? opticianWidth(str, size) + tight(size) * [...str].length : textWidth(str, size, { weight: 900 }));
    // Bigger than the Aircon keys' 33; "100%" a size down to fit.
    // Optician Sans's digits sit shorter than Segoe UI Black's, so it gets more.
    const three = num.length >= 3;
    const [size, pct] = NUM_FONT ? (three ? [40, 23] : [46, 26]) : three ? [34, 20] : [41, 24];
    const y = NUM_Y;
    const numFill = v.dim ? OFF.number : (v.color ?? "#fff");
    if (NUM_FONT) {
      let x = X - 1;
      for (const ch of num) {
        s += text(x, y, ch, { size, ...nf, fill: numFill });
        x += opticianWidth(ch, size) + tight(size);
      }
    } else s += text(X - 1, y, num, { size, ...nf, fill: numFill });
    // A small % after the number, like the Zones key's "/6" (grey, or the number's colour
    // when low or charging).
    const after = X + numWidth(num, size) + (NUM_FONT ? 1.5 : -1);
    if (v.percent != null) s += text(after, y, "%", { size: pct, ...nf, fill: v.dim ? OFF.text : (v.color ?? GREY) });
    s += bar(v.percent, v.dim, v.color ?? COLORS.healthy);
  }
  if (v.dot) s += `<circle cx="84" cy="37.5" r="5" fill="${ACCENT}"/>`;
  return wrap(s);
}

export const dataUrl = (svg) => `data:image/svg+xml,${encodeURIComponent(svg)}`;
