// Draws the key in the same text-first style as the Aircon plugin's keys (after the
// Apple Watch Modular complications): "Hyperlight" top left in white, the percent as
// the biggest thing on the key (orange or red when low), a thin 0-100% bar (green up
// to a dot at the level, grey after), and a small caps line underneath.
//
// Keys are drawn in a 96-unit box (one unit = one Stream Deck XL pixel) and handed to
// Stream Deck as a 144px SVG. Stream Deck's SVG renderer is basic, so: no CSS, no
// filters, no tspan offsets, and only installed fonts.
import fs from "node:fs";
import path from "node:path";

/** Apple's system colours, as on the Aircon keys. */
export const COLORS = { healthy: "#30D158", moderate: "#FF9F0A", low: "#FF453A", stale: "#8E8E93" };
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
 * turned on, so by default the bottom line just says Battery; the update dot is on.
 */
export function options(settings) {
  return {
    remaining: settings?.showRemaining === true,
    charging: settings?.showCharging === true,
    asleep: settings?.showAsleep === true,
    update: settings?.showUpdate !== false,
  };
}

/**
 * What to show for a tray reply (see tray.js); undefined means not read yet.
 * { percent, color, caption, bolt, dim, dot } for a reading (color: the number's
 * colour when low, else null for white), or
 * { word, caption } for a key with no number ("Start / Tray").
 */
export function view(s, opts = options()) {
  const dot = Boolean(s?.update) && opts.update;
  switch (s?.state) {
    case "battery": {
      const charging = s.charging && opts.charging;
      const caption = charging ? "Charging" : !s.charging && opts.remaining && s.remaining ? s.remaining : "Battery";
      const t = tone(s.percent);
      return { percent: s.percent ?? null, color: t === "healthy" ? null : COLORS[t], caption, bolt: charging, dot };
    }
    case "asleep":
      return { percent: s.percent ?? null, color: null, caption: opts.asleep ? "Asleep" : "Battery", dim: true, dot };
    case "noReceiver":
      return { word: "No", caption: "Receiver", dot };
    case "stopped":
      return { word: "Start", caption: "Tray" };
    case "outdated":
      return { word: "Update", caption: "Tray" };
    case "missing":
      return { word: "Get", caption: "Tray" };
    default: // "checking", or before the first reply
      return { percent: null, color: null, caption: "Battery", dim: true };
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
function fontDirs() {
  const dirs = [path.join(process.env.WINDIR ?? "C:\\Windows", "Fonts")];
  if (process.env.LOCALAPPDATA) dirs.push(path.join(process.env.LOCALAPPDATA, "Microsoft", "Windows", "Fonts"));
  return dirs;
}
let FONT = "Segoe UI";
export const setFont = (f) => (FONT = f);

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
const text = (x, y, str, { size = 14, weight = 800, fill = "#fff", spacing = 0 } = {}) =>
  `<text x="${f2(x)}" y="${f2(y)}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}" ` +
  `text-anchor="start"${spacing ? ` letter-spacing="${spacing}"` : ""}>${esc(str)}</text>`;
const wrap = (body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 96 96">` +
  `<rect width="96" height="96" fill="#000"/>${body}</svg>`;

/* ---------- the layout (the Aircon plugin's modular.js) ---------- */
const X = 9; // left edge of text and bar
const NUM_Y = 57; // baseline of the big number
const BAR = { y: 66, w: 78, h: 5 };
/** The name keeps the Aircon keys' 17px when it fits, else shrinks to fit the width. */
const NAME = "Hyperlight";
const NAME_MAX = 79;

function name(color) {
  const size = Math.min(17, Math.floor((NAME_MAX / textWidth(NAME, 1)) * 2) / 2);
  return text(X, 23, NAME, { size, fill: color });
}
/** The bottom line: 13px caps, down to 11.5px to fit, then cut with "…". */
function footer(str, color, max) {
  str = String(str).toUpperCase();
  const o = { weight: 900, spacing: 0.4 };
  let size = 13;
  while (size > 11.5 && textWidth(str, size, o) > max) size -= 0.5;
  return text(X, 87, truncate(str, size, max, o), { size, ...o, fill: color });
}
/** A charging bolt, the Aircon plugin's "bolt" icon. */
const bolt = (x, y, s, c) =>
  `<g transform="translate(${f2(x)} ${f2(y)}) scale(${f2(s / 24)})"><path d="M13.8 1.8 4.6 13.6H11.4L10.2 22.2 19.4 10.4H12.6Z" fill="${c}"/></g>`;

/** The 0-100% bar: green up to the level, the grey track after it, and a white dot at the level. */
function bar(percent, dim) {
  const track = `<rect x="${X}" y="${BAR.y}" width="${BAR.w}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${TRACK}"/>`;
  if (percent == null) return track;
  const p = Math.max(0, Math.min(100, percent)) / 100;
  let s = track;
  if (p > 0) s += `<g opacity="${dim ? OFF.fade : 1}"><rect x="${X}" y="${BAR.y}" width="${f2(Math.max(BAR.h, BAR.w * p))}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${COLORS.healthy}"/></g>`;
  s += `<circle cx="${f2(X + BAR.w * p)}" cy="${BAR.y + BAR.h / 2}" r="4.4" fill="${dim ? OFF.dot : "#fff"}" stroke="#000" stroke-width="2.2"/>`;
  return s;
}

/** A 144×144 SVG for the key. */
export function keySvg(v) {
  let s = "";
  // "Update available": a blue dot at the right end of the bottom line (which then stops short of it).
  const footMax = v.dot ? 68 : 80;
  if (v.word != null) {
    s += name("#fff");
    const size = Math.min(24, Math.floor(79 / textWidth(v.word, 1, { weight: 900 })));
    s += text(X - 1, NUM_Y, v.word, { size, weight: 900, fill: GREY });
    s += `<rect x="${X}" y="${BAR.y}" width="${BAR.w}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${TRACK}"/>`;
    s += footer(v.caption, GREY, footMax);
  } else {
    const num = v.percent == null ? "–" : String(v.percent);
    s += name("#fff");
    s += text(X - 1, NUM_Y, num, { size: 33, weight: 900, fill: v.dim ? OFF.number : (v.color ?? "#fff") });
    // A small % after the number, like the Zones key's "/6" (grey, or the number's colour
    // when low); a green bolt instead while charging.
    const after = X + textWidth(num, 33, { weight: 900 }) - 1;
    if (v.bolt) s += bolt(after, NUM_Y - 20, 20, COLORS.healthy);
    else if (v.percent != null) s += text(after, NUM_Y, "%", { size: 20, weight: 900, fill: v.dim ? OFF.text : (v.color ?? GREY) });
    s += bar(v.percent, v.dim);
    s += footer(v.caption, v.dim ? OFF.text : GREY, footMax);
  }
  if (v.dot) s += `<circle cx="84" cy="82.5" r="5" fill="${ACCENT}"/>`;
  return wrap(s);
}

export const dataUrl = (svg) => `data:image/svg+xml,${encodeURIComponent(svg)}`;
