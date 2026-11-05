// Draws the key: the battery number in the tray icon's colours, on black.

/** The tray icon's dark-taskbar colours (src/icon.rs). */
export const COLORS = { healthy: "#6ccb5f", moderate: "#ffa94d", low: "#ff6b61", stale: "#8a8a8a" };
const MUTED = "#9ba3a7";
const ACCENT = "#4cc2ff";

export function tone(percent) {
  return percent >= 50 ? "healthy" : percent >= 20 ? "moderate" : "low";
}

/** What to show for a tray reply (see tray.js); undefined means not read yet. */
export function view(s) {
  const dot = Boolean(s?.update);
  const number = (p) => (p === null || p === undefined ? "–" : String(p));
  switch (s?.state) {
    case "battery":
      return {
        text: number(s.percent),
        color: COLORS[tone(s.percent)],
        caption: s.charging ? "Charging" : (s.remaining ?? ""),
        dot,
      };
    case "asleep":
      return { text: number(s.percent), color: COLORS.stale, caption: "Asleep", dot };
    case "noReceiver":
      return { text: "–", color: COLORS.stale, caption: "No receiver", dot };
    case "stopped":
      return { icon: true, caption: "Start tray" };
    case "outdated":
      return { icon: true, caption: "Update tray" };
    case "missing":
      return { icon: true, caption: "Get tray" };
    default: // "checking", or before the first reply
      return { text: "–", color: COLORS.stale, caption: "" };
  }
}

const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** A 144×144 SVG for the key. */
export function keySvg(v) {
  const out = [
    '<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 144 144">',
    '<rect width="144" height="144" fill="#000"/>',
  ];
  const captioned = Boolean(v.caption);
  if (v.icon) {
    // The mouse outline from the configurator's icon.
    const y = captioned ? 24 : 41;
    out.push(
      `<rect x="54" y="${y}" width="36" height="62" rx="18" fill="none" stroke="${COLORS.stale}" stroke-width="6"/>`,
      `<path d="M72 ${y}v19" stroke="${COLORS.stale}" stroke-width="6" stroke-linecap="round"/>`,
    );
  } else {
    // Same typeface as the tray number. Three digits ("100") get a smaller size to fit.
    const size = v.text.length >= 3 ? 76 : 98;
    const y = captioned ? 90 : 106;
    out.push(
      `<text x="72" y="${y}" text-anchor="middle" font-family="Bahnschrift" font-weight="600" ` +
        `font-stretch="semi-condensed" font-size="${size}" fill="${v.color}">${esc(v.text)}</text>`,
    );
  }
  if (captioned) {
    out.push(
      `<text x="72" y="127" text-anchor="middle" font-family="Segoe UI" font-weight="600" ` +
        `font-size="21" fill="${MUTED}">${esc(v.caption)}</text>`,
    );
  }
  if (v.dot) {
    // "Update available", like the dot on the tray icon.
    out.push(`<circle cx="124" cy="20" r="12" fill="#000"/><circle cx="124" cy="20" r="8.5" fill="${ACCENT}"/>`);
  }
  out.push("</svg>");
  return out.join("");
}

export const dataUrl = (svg) => `data:image/svg+xml,${encodeURIComponent(svg)}`;
