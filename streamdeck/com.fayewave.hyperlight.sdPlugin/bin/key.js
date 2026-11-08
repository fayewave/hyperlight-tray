// Draws the key in the same text-first style as the Aircon plugin's keys (after the
// Apple Watch Modular complications), turned upside down below the name: "Hyperlight"
// top left in white, a small caps line under it, then the percent as the biggest thing
// on the key and a thin 0-100% bar along the bottom (green, orange or red by level, or
// blue while charging, up to a dot at the level, grey after).
//
// Keys are drawn in a 96-unit box (one unit = one Stream Deck XL pixel) and handed to
// Stream Deck as a 144px SVG. Stream Deck's SVG renderer is basic (no CSS, no filters)
// and only draws text in installed fonts, so all the text is drawn from the outlines
// in glyphs.js: the number in Optician Sans, the rest in Bai Jamjuree Bold.
import { DIGITS, TEXT } from "./glyphs.js";

/** Apple's system colours, as on the Aircon keys. */
/** charging: an electric blue, deeper than the update dot's light blue. */
export const COLORS = { healthy: "#30D158", moderate: "#FF9F0A", low: "#FF453A", stale: "#8E8E93", charging: "#1E9BFF" };
const GREY = "#8E8E93";
const TRACK = "#2C2C2E";
const ACCENT = "#64D2FF";
/** No reading yet: the dash and caption dimmed, like a switched-off AC. */
const OFF = { number: "#6C6C70", dot: "#77777C", fade: 0.18, text: "#636366" };

export function tone(percent) {
  return percent > 25 ? "healthy" : percent >= 20 ? "moderate" : "low";
}

/**
 * The key's own options (its settings in Stream Deck). The charging and time-left
 * lines are on until turned off; the asleep line and update dot are off until turned on.
 */
export function options(settings) {
  return {
    remaining: settings?.showRemaining !== false,
    charging: settings?.showCharging !== false,
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
    case "asleep": // the last reading, as it was: the mouse just isn't answering
      return { percent: s.percent ?? null, color: s.percent == null || tone(s.percent) === "healthy" ? null : COLORS[tone(s.percent)], caption: opts.asleep ? "Asleep" : "", dot };
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

/* ---------- text, drawn from glyph outlines ---------- */
const f2 = (n) => Number(n.toFixed(2));
const glyph = (font, ch) => font[ch] ?? (font === TEXT ? TEXT["?"] : DIGITS["–"]);

/** Width of str at size in units; track = extra space after each character. */
export function textWidth(str, size, { font = TEXT, track = 0 } = {}) {
  let w = 0;
  for (const ch of String(str)) w += glyph(font, ch).w * size + track;
  return w;
}

/**
 * str as one path per glyph, left end at x, baseline at y. track = extra space after
 * each character, in units (negative pulls them closer). Returns the SVG and the end x.
 */
function draw(x, y, str, size, fill, { font = TEXT, track = 0 } = {}) {
  let s = "";
  const k = Number((size / 1000).toFixed(5));
  for (const ch of String(str)) {
    const g = glyph(font, ch);
    if (g.d) s += `<path transform="translate(${f2(x)} ${f2(y)}) scale(${k})" d="${g.d}" fill="${fill}"/>`;
    x += g.w * size + track;
  }
  return { svg: s, end: x };
}

/** str cut to fit max units wide, ending in "…" when it had to be cut. */
function truncate(str, size, max, opts = {}) {
  str = String(str);
  if (textWidth(str, size, opts) <= max) return str;
  let chars = [...str];
  while (chars.length > 1 && textWidth(`${chars.join("").trimEnd()}…`, size, opts) > max) chars = chars.slice(0, -1);
  return `${chars.join("").trimEnd()}…`;
}

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
  return draw(X, 23, NAME, size, color).svg;
}
/** The line under the name: 13px caps, down to 11px to fit, then cut with "…". */
function label(str, color, max) {
  str = String(str).toUpperCase();
  const o = { track: 0.4 };
  let size = 13;
  while (size > 11 && textWidth(str, size, o) > max) size -= 0.5;
  return draw(X, LABEL_Y, truncate(str, size, max, o), size, color, o).svg;
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
    const size = Math.min(24, Math.floor(79 / textWidth(v.word, 1)));
    s += draw(X - 1, NUM_Y, v.word, size, GREY).svg;
    s += `<rect x="${X}" y="${BAR.y}" width="${BAR.w}" height="${BAR.h}" rx="${BAR.h / 2}" fill="${TRACK}"/>`;
  } else {
    const num = v.percent == null ? "–" : String(v.percent);
    s += name("#fff");
    if (v.caption) s += label(v.caption, v.dim ? OFF.text : GREY, labelMax);
    // The number and its % in Optician Sans. Its digits are only half an em tall (about
    // Segoe UI Black's x-height), hence the big sizes, and spaced wide, so they're pulled
    // 6% of the size closer. "100" drops the % rather than shrinking to fit it.
    const size = 46;
    const n = draw(X - 1, NUM_Y, num, size, v.dim ? OFF.number : (v.color ?? "#fff"), { font: DIGITS, track: -0.06 * size });
    s += n.svg;
    // A small % after the number, like the Zones key's "/6" (grey, or the number's colour
    // when low or charging).
    if (v.percent != null && num.length < 3) s += draw(n.end + 2.5, NUM_Y, "%", 26, v.dim ? OFF.text : (v.color ?? GREY), { font: DIGITS }).svg;
    s += bar(v.percent, v.dim, v.color ?? COLORS.healthy);
  }
  if (v.dot) s += `<circle cx="84" cy="37.5" r="5" fill="${ACCENT}"/>`;
  return wrap(s);
}

export const dataUrl = (svg) => `data:image/svg+xml,${encodeURIComponent(svg)}`;
