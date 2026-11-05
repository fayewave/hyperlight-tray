// UI for the Hyperlight Configurator. Plain DOM, no framework or build step.
import * as P from "./protocol.js";
import { HidMouse, webHidSupported } from "./device.js";
import { DemoMouse } from "./demo.js";

const $app = document.getElementById("app");
const $nav = document.getElementById("nav");
const $connbar = document.getElementById("connbar");
const $toasts = document.getElementById("toasts");

const STATUS_POLL_MS = 3000;
const BATTERY_EVERY = 10; // status polls, so every 30 s

const PAGES = [
  { id: "performance", label: "Performance", icon: '<path d="M2 4h7M12 4h2M2 12h2M7 12h7M9 2v4M4 10v4" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>' },
  { id: "buttons", label: "Buttons", icon: '<rect x="2" y="2" width="5" height="5" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><rect x="9" y="2" width="5" height="5" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><rect x="2" y="9" width="5" height="5" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><rect x="9" y="9" width="5" height="5" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/>' },
  { id: "device", label: "Device", icon: '<rect x="5" y="1.5" width="6" height="13" rx="3" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M8 1.5v4" stroke="currentColor" stroke-width="1.3"/>' },
];

const state = {
  /** Tray app settings: null while checking, false when not served by Hyperlight Tray. */
  tray: null,
  dev: null,
  raw: null,
  s: null,
  awake: false,
  id: null,
  shown: null,
  battery: null,
  longRange: null,
  readError: false,
  chain: Promise.resolve(),
  timer: null,
  ticks: 0,
  est: new P.Estimator(),
};

// ---------------------------------------------------------------------------
// tiny DOM helper

function h(tag, props, ...kids) {
  const el = tag === "svg"
    ? document.createElementNS("http://www.w3.org/2000/svg", tag)
    : document.createElement(tag);
  for (const [k, v] of Object.entries(props ?? {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") el.setAttribute("class", v);
    else if (k === "html") el.innerHTML = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "value") el.value = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of kids.flat(Infinity)) {
    if (c === null || c === undefined || c === false) continue;
    el.append(c instanceof Node ? c : String(c));
  }
  return el;
}

const icon = (paths, cls) => h("svg", { viewBox: "0 0 16 16", "aria-hidden": "true", class: cls, html: paths });
// ---------------------------------------------------------------------------
// setting tooltips: hover or focus the (i) button

const TIPS = {
  lowAlert: {
    title: "Low battery alert",
    body: "Shows a Windows notification when the battery drops to this level, and again at 5%. Each alert shows once and comes back after you charge.",
  },
  fullAlert: {
    title: "Fully charged alert",
    body: "Shows a Windows notification when the battery reaches 100% while charging.",
  },
  autostart: {
    title: "Start with Windows",
    body: "Starts Hyperlight Tray when you sign in, so the battery number and alerts are always there. Also in the tray icon's menu.",
  },
  sensitivity: {
    title: "Sensitivity",
    body: "How far the cursor moves for each inch the mouse travels. The DPI button on the underside cycles through the stages, and the stage colour shows on the mouse when you switch.",
  },
  polling: {
    title: "Polling rate",
    body: "How often the mouse reports to the computer. Higher rates shorten the time between reports and use more battery.",
  },
  lod: {
    title: "Lift-off distance",
    body: "How high you can lift the mouse before the sensor stops tracking. A lower setting stops the cursor drifting while you reposition.",
  },
  motionSync: {
    title: "Motion sync",
    body: "Lines sensor readings up with USB reports so tracking is more consistent, at the cost of a very small delay.",
    impact: "low",
  },
  ripple: {
    title: "Ripple control",
    body: "Smooths small jitter in the sensor signal. Mostly useful at high DPI, and can make tracking feel slightly less direct.",
    impact: "low",
  },
  angleSnap: {
    title: "Angle snapping",
    body: "Straightens movements that are almost horizontal or vertical. Handy for drawing lines, usually left off for gaming.",
    impact: "low",
  },
  highPerf: {
    title: "Performance mode",
    body: "Runs the sensor harder to support faster movements. Uses more power.",
    impact: "medium",
  },
  longRange: {
    title: "Long distance mode",
    body: "Strengthens the wireless link for setups where the mouse sits far from the receiver. This is stored on the receiver, not the mouse.",
  },
  debounce: {
    title: "Debounce time",
    body: "How long a button ignores further presses after a click. Raise it if a worn switch starts double-clicking; 0 ms gives the fastest response.",
  },
  sleep: {
    title: "Auto sleep",
    body: "How long the mouse waits without moving before it sleeps. Moving it wakes it up again. Shorter times save battery.",
  },
};

const IMPACT = { low: "Low", medium: "Medium", high: "High" };
const $tip = h("div", { class: "setting-info", id: "setting-info", role: "tooltip", hidden: true });
document.body.append($tip);
let tipOwner = null;

function showTip(btn, key) {
  const t = TIPS[key];
  if (!t) return;
  $tip.replaceChildren(...[
    h("strong", {}, t.title),
    h("p", {}, t.body),
    t.impact && h("div", { class: "battery-impact" },
      h("span", {}, "Battery impact"), h("b", { "data-impact": t.impact }, IMPACT[t.impact])),
  ].filter(Boolean));
  $tip.hidden = false;
  tipOwner?.removeAttribute("aria-describedby");
  tipOwner = btn;
  btn.setAttribute("aria-describedby", "setting-info");
  // Above the button, left edges lined up; below it if there's no room.
  const r = btn.getBoundingClientRect();
  const w = $tip.offsetWidth;
  const ht = $tip.offsetHeight;
  const left = Math.min(Math.max(12, r.left), window.innerWidth - w - 12);
  const top = r.top - ht - 8 >= 12 ? r.top - ht - 8 : r.bottom + 8;
  $tip.style.left = `${left}px`;
  $tip.style.top = `${top}px`;
}

function hideTip() {
  $tip.hidden = true;
  tipOwner?.removeAttribute("aria-describedby");
  tipOwner = null;
}
window.addEventListener("scroll", hideTip, { passive: true });
window.addEventListener("keydown", (e) => e.key === "Escape" && hideTip());

const INFO_ICON = '<circle cx="12" cy="12" r="9"/><path d="M12 10v6m0-10v.1"/>';
const info = (key) => h("button", {
  class: "info-button", type: "button", "aria-label": `About ${TIPS[key]?.title ?? key}`,
  onmouseenter: (e) => showTip(e.currentTarget, key),
  onmouseleave: hideTip,
  onfocus: (e) => showTip(e.currentTarget, key),
  onblur: hideTip,
  onclick: (e) => (tipOwner === e.currentTarget ? hideTip() : showTip(e.currentTarget, key)),
}, h("svg", { viewBox: "0 0 24 24", "aria-hidden": "true", html: INFO_ICON }));

function toast(msg, kind = "ok") {
  const t = h("div", { class: `toast ${kind === "error" ? "error" : ""}`, role: "status" }, msg);
  $toasts.append(t);
  setTimeout(() => t.remove(), kind === "error" ? 5000 : 2200);
}

const page = () => {
  const id = location.hash.replace("#", "");
  return PAGES.some((p) => p.id === id) ? id : "performance";
};

// ---------------------------------------------------------------------------
// tray app settings (only when served by Hyperlight Tray)

const trayApi = {
  async call(path, body) {
    const r = await fetch(path, body === undefined ? { cache: "no-store" } : {
      method: "POST", cache: "no-store", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
    });
    if (!r.ok) throw new Error(`${r.status}`);
    return r.json();
  },
  get: () => trayApi.call("/api/tray"),
  set: (patch) => trayApi.call("/api/tray", patch),
  test: () => trayApi.call("/api/tray/test", {}),
};

async function loadTray() {
  try {
    state.tray = await trayApi.get();
  } catch {
    state.tray = false;
  }
  render();
}

async function saveTray(label, patch) {
  try {
    state.tray = await trayApi.set(patch);
    toast(`${label} saved`);
  } catch {
    toast(`Couldn't save ${label.toLowerCase()}. Is Hyperlight Tray still running?`, "error");
  }
  render();
}

// ---------------------------------------------------------------------------
// theme

const $themeBtn = document.getElementById("theme-toggle");
function paintTheme() {
  const t = document.documentElement.dataset.theme === "light" ? "light" : "carbon";
  document.getElementById("theme-label").textContent = t === "light" ? "Light" : "Dark";
  $themeBtn.title = "Switch appearance";
}
$themeBtn.addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "light" ? "carbon" : "light";
  document.documentElement.dataset.theme = next;
  try {
    localStorage.setItem("theme", next);
  } catch {}
  paintTheme();
});
paintTheme();

// Scale the sidebar name so it spans the sidebar (the same width as the nav items' text).
const $wordmark = document.getElementById("wordmark-name");
function fitWordmark() {
  $wordmark.style.fontSize = "";
  if (window.innerWidth <= 680) return;
  const box = $wordmark.parentElement;
  const cs = getComputedStyle(box);
  const avail = box.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const size = parseFloat(getComputedStyle($wordmark).fontSize);
  $wordmark.style.fontSize = `${(size * avail) / $wordmark.getBoundingClientRect().width}px`;
}
document.fonts.ready.then(fitWordmark);
window.addEventListener("resize", fitWordmark);

// ---------------------------------------------------------------------------
// device lifecycle

async function connect() {
  try {
    const m = await HidMouse.request();
    if (m === undefined) return toast("No device selected");
    if (!m) return toast("Couldn't open that device. Try unplugging and replugging the receiver.", "error");
    await attach(m);
  } catch (err) {
    if (err?.name !== "NotFoundError") {
      console.error(err);
      toast("Couldn't connect: " + (err?.message ?? err), "error");
    }
  }
}

async function attach(m) {
  state.dev = m;
  state.est = new P.Estimator();
  state.shown = null;
  state.battery = null;
  state.raw = null;
  state.s = null;
  await refreshStatus();
  await loadSettings();
  await refreshBattery();
  render();
  clearInterval(state.timer);
  state.ticks = 0;
  state.timer = setInterval(tick, STATUS_POLL_MS);
}

async function detach(reason) {
  clearInterval(state.timer);
  const dev = state.dev;
  state.dev = null;
  state.raw = state.s = null;
  await dev?.close();
  render();
  if (reason) toast(reason, "error");
}

async function loadSettings() {
  const raw = await state.dev.readBlock();
  state.readError = !raw;
  if (raw) {
    state.raw = raw;
    state.s = P.parseSettings(raw);
  }
  state.longRange = await state.dev.longRange();
}

async function refreshStatus() {
  const st = await state.dev.status();
  const changed = st.awake !== state.awake;
  state.awake = st.awake;
  state.id = st.id;
  return changed;
}

async function refreshBattery() {
  if (!state.awake) return;
  const b = await state.dev.battery();
  if (!b) return;
  state.battery = b;
  const key = `hitscan.battery.${state.id ?? "unknown"}`;
  const now = Math.floor(Date.now() / 1000);
  let saved = null;
  try {
    saved = JSON.parse(localStorage.getItem(key));
  } catch {}
  const before = state.shown;
  state.shown = state.est.update(b, now, before === null ? saved : null);
  if (state.shown !== before) {
    try {
      localStorage.setItem(key, JSON.stringify({ percent: state.shown, unix: now }));
    } catch {}
  }
}

async function tick() {
  if (!state.dev || document.hidden) return;
  // Run between saves, never alongside one.
  state.chain = state.chain.then(async () => {
    if (!state.dev) return;
    try {
      const wasAwake = state.awake;
      if (await refreshStatus()) {
        if (!wasAwake && state.awake) await loadSettings();
        render();
      }
      if (++state.ticks % BATTERY_EVERY === 0) await refreshBattery();
      renderConnbar();
    } catch (err) {
      console.warn(err);
    }
  });
}

/** Runs one change: checks the mouse is awake, writes, re-reads, reports. */
function save(label, fn) {
  state.chain = state.chain.then(async () => {
    if (!state.dev) return;
    await refreshStatus();
    if (!state.awake) {
      render();
      return toast("The mouse is asleep. Move it, then try again.", "error");
    }
    $app.classList.add("saving");
    let ok = false;
    try {
      ok = await fn();
    } catch (err) {
      console.error(err);
    }
    await loadSettings();
    $app.classList.remove("saving");
    render();
    toast(ok ? `${label} saved` : `Couldn't save ${label.toLowerCase()}`, ok ? "ok" : "error");
  });
}

function reload() {
  state.chain = state.chain.then(async () => {
    if (!state.dev) return;
    await refreshStatus();
    await loadSettings();
    render();
  });
}

async function writeAll(list) {
  for (const w of list) if (!(await state.dev.write(w))) return false;
  return true;
}

const saveWrite = (label, w) => save(label, () => writeAll([w]));

if (webHidSupported()) {
  navigator.hid.addEventListener("disconnect", (e) => {
    if (state.dev?.device === e.device) detach("Receiver disconnected");
  });
}

// ---------------------------------------------------------------------------
// chrome: nav and connection bar

function renderNav() {
  const current = page();
  $nav.replaceChildren(...PAGES.map((p) => h("a", {
    href: `#${p.id}`,
    "aria-current": state.dev && p.id === current ? "page" : null,
    "aria-disabled": state.dev ? null : "true",
  }, icon(p.icon), h("span", {}, p.label))));
}

function batteryIcon(pct, charging) {
  const w = Math.max(1, Math.round((pct / 100) * 9));
  const fill = charging ? "var(--ok)" : pct <= 15 ? "var(--danger)" : "currentColor";
  return icon(`<rect x="1" y="4.5" width="12" height="7" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.2"/>` +
    `<rect x="13.5" y="6.5" width="1.5" height="3" rx=".5" fill="currentColor"/>` +
    `<rect x="2.5" y="6" width="${w}" height="4" rx=".5" fill="${fill}"/>`, "batt");
}

function renderConnbar() {
  if (!state.dev) {
    $connbar.replaceChildren(
      h("div", { class: "left" }, h("span", { class: "model" }, "Hyperlight"), h("span", { class: "sep" }), h("span", {}, "Not connected")),
      h("div", { class: "right" }));
    return;
  }
  const kind = state.dev.demo ? "Demo mode" : state.dev.wired ? "Cable" : "Wireless receiver";
  const right = [];
  if (state.shown !== null && state.battery) {
    const b = state.battery;
    right.push(h("span", { class: "ind", title: `${b.charging ? "Charging" : "On battery"} · ${(b.millivolts / 1000).toFixed(2)} V` },
      batteryIcon(state.shown, b.charging), h("span", { class: "num" }, `${state.shown}%`), b.charging ? "Charging" : "Battery"));
  }
  right.push(state.awake
    ? h("span", { class: "ind ok" }, "Connected")
    : h("span", { class: "ind asleep", title: "Move the mouse to wake it" }, "Asleep"));
  $connbar.replaceChildren(
    h("div", { class: "left" },
      h("span", { class: "model" }, "Hyperlight"), h("span", { class: "sep" }), h("span", {}, kind),
      h("button", { class: "linkbtn", onclick: () => detach() }, state.dev.demo ? "Disconnect demo" : "Disconnect")),
    h("div", { class: "right" }, right));
}

function render() {
  hideTip();
  renderNav();
  renderConnbar();
  if (!state.dev) return $app.replaceChildren(connectView());
  const p = PAGES.find((x) => x.id === page());
  const body = [];
  if (!state.awake) body.push(h("div", { class: "notice warn" }, h("span", { class: "dot" }),
    "The mouse is asleep. Move it to wake it up; settings can't be changed until it's awake."));
  if (!state.s) {
    body.push(h("div", { class: "notice error" }, h("span", { class: "dot" }), "Couldn't read the mouse's settings.",
      h("button", { class: "linkbtn", onclick: reload }, "Try again")));
  } else {
    const off = !state.awake;
    body.push(p.id === "buttons" ? buttonsPage(state.s, off) : p.id === "device" ? devicePage() : performancePage(state.s, off));
  }
  $app.replaceChildren(h("h1", { class: "page" }, p.label), ...body);
}

window.addEventListener("hashchange", render);

// ---------------------------------------------------------------------------
// connect screen

function connectView() {
  const supported = webHidSupported();
  return h("div", { class: "connect" },
    h("section", { class: "card" },
      h("h2", {}, "Connect your Hyperlight"),
      h("p", {}, "Change DPI, polling rate, sensor options, buttons and sleep from your browser. Your settings stay on your computer and your mouse."),
      !supported && h("div", { class: "notice error" }, h("span", { class: "dot" }),
        "This browser can't talk to USB devices. Open this page in Chrome, Edge or Opera on a computer."),
      h("div", { class: "btn-row" },
        h("button", { class: "btn btn-primary", onclick: connect, disabled: !supported }, "Connect"),
        h("button", { class: "btn", onclick: () => attach(new DemoMouse()) }, "Start demo")),
      h("details", { class: "help" },
        h("summary", {}, "Can't connect?"),
        h("ul", {},
          h("li", {}, "Plug in the receiver (or the cable) first, then click Connect."),
          h("li", {}, "In the browser's list, pick ", h("strong", {}, "Hitscan USB Receiver"), "."),
          h("li", {}, "Use Chrome, Edge or Opera on a computer. Firefox and Safari don't support WebHID."),
          h("li", {}, "If the mouse is asleep, move it so changes can be saved.")))));
}

// ---------------------------------------------------------------------------
// controls

function segments(options, current, onPick, disabled, cols) {
  return h("div", { class: "segments", style: `--cols:${cols ?? options.length}` },
    options.map((o) => h("button", {
      class: "segment",
      "aria-pressed": String(o.value === current),
      title: o.title,
      disabled,
      onclick: () => o.value !== current && onPick(o.value),
    }, o.label, o.unit && h("small", {}, o.unit))));
}

function toggle(name, on, onChange, disabled) {
  return h("button", {
    class: "switch", role: "switch", "aria-checked": String(!!on), "aria-label": name, disabled,
    onclick: () => onChange(!on),
  });
}

const card = (title, meta, ...body) => h("section", { class: "card" },
  h("div", { class: "card-heading" }, h("span", { class: "card-title" }, ...[title].flat()), meta), body);

// ---------------------------------------------------------------------------
// performance

function performancePage(s, off) {
  const flag = (key, name, extra) => h("div", { class: "row" },
    h("span", { class: "row-name" }, name, info(key), extra),
    toggle(name, s[key], (on) => saveWrite(name, P.writes[key](on)), off || s[key] === null));

  const hz = s.pollingHz;
  const polling = card(["Polling rate", info("polling")],
    hz && h("span", { class: "card-meta num" }, `${(1000 / hz).toFixed(1)} ms`),
    segments(P.POLLING.slice().reverse().map((p) => ({
      label: p.hz.toLocaleString("en-US"), unit: "Hz", value: p.hz,
      title: p.status === "inferred" ? "Not yet confirmed on hardware" : null,
    })), hz, (v) => saveWrite("Polling rate", P.writes.polling(v)), off));

  const lod = card(["Lift-off distance", info("lod")], null,
    segments(P.LOD.map((l) => ({ label: String(l.mm), unit: "mm", value: l.mm })), s.lodMm,
      (mm) => saveWrite("Lift-off distance", P.writes.lod(mm)), off));

  const sensor = card("Sensor & response", null,
    flag("motionSync", "Motion sync"),
    flag("ripple", "Ripple control"),
    flag("angleSnap", "Angle snapping"),
    flag("highPerf", "Performance mode"),
    h("div", { class: "row" },
      h("span", { class: "row-name" }, "Long distance mode", info("longRange")),
      toggle("Long distance mode", state.longRange,
        (on) => save("Long distance mode", () => state.dev.setLongRange(on)), off || state.longRange === null)));

  const deb = s.debounceMs ?? 0;
  const setDeb = (ms) => saveWrite("Debounce", P.writes.debounce(Math.min(P.DEBOUNCE_MAX, Math.max(0, ms))));
  const sleepVal = s.sleepEnabled ? s.sleepUnits : "off";
  const sleepOpts = [{ label: "Off", unit: " ", value: "off" }].concat(P.SLEEP.map((x) => {
    const [n, u] = x.label.split(" ");
    return { label: n, unit: u, value: x.units };
  }));
  const pickSleep = (v) => {
    if (v === "off") return saveWrite("Auto sleep", P.writes.sleepEnabled(false));
    const list = [P.writes.sleepTime(v)];
    if (!s.sleepEnabled) list.push(P.writes.sleepEnabled(true));
    save("Auto sleep", () => writeAll(list));
  };
  const additional = card("Additional settings", null,
    h("div", { class: "row" },
      h("span", { class: "row-name" }, "Debounce time", info("debounce")),
      h("div", { class: "stepper" },
        h("button", { "aria-label": "Less debounce", disabled: off || deb <= 0, onclick: () => setDeb(deb - 1) }, "−"),
        h("span", { class: "num" }, deb, h("small", {}, "ms")),
        h("button", { "aria-label": "More debounce", disabled: off || deb >= P.DEBOUNCE_MAX, onclick: () => setDeb(deb + 1) }, "+"))),
    h("div", { class: "row", style: "border-bottom:0" },
      h("span", { class: "row-name" }, "Auto sleep", info("sleep"))),
    segments(sleepOpts, sleepVal, pickSleep, off, 5));

  return h("div", { class: "grid" },
    state.tray && trayCards(),
    sensitivityCard(s, off),
    polling, lod, sensor, additional);
}

function sensitivityCard(s, off) {
  const count = Math.min(Math.max(s.stageCount ?? 1, 1), P.MAX_STAGES);
  const cur = Math.min(s.currentStage ?? 0, count - 1);
  const st = s.stages[cur];
  const setCount = (n) => {
    const list = [];
    if (s.currentStage >= n) list.push(P.writes.currentStage(n - 1));
    list.push(P.writes.stageCount(n));
    save("DPI stages", () => writeAll(list));
  };
  const countSel = h("select", { "aria-label": "Number of DPI stages", disabled: off, onchange: (e) => setCount(Number(e.target.value)) },
    Array.from({ length: P.MAX_STAGES }, (_, k) => h("option", { value: k + 1, selected: k + 1 === count }, k + 1)));

  const editable = st.dpi !== null;
  const fill = (v) => `--fill:${((v - P.DPI_MIN) / (P.DPI_MAX - P.DPI_MIN)) * 100}%`;
  const big = h("input", {
    class: "dpi-input", type: "number", min: P.DPI_MIN, max: P.DPI_MAX, step: P.DPI_STEP,
    value: st.dpi ?? "", disabled: off || !editable, "aria-label": `Stage ${cur + 1} DPI`,
    onchange: (e) => commit(e.target.value),
    onkeydown: (e) => e.key === "Enter" && e.target.blur(),
  });
  const slider = h("input", {
    type: "range", min: P.DPI_MIN, max: P.DPI_MAX, step: P.DPI_STEP, value: st.dpi ?? P.DPI_MIN,
    style: fill(st.dpi ?? P.DPI_MIN), disabled: off || !editable, "aria-label": `Stage ${cur + 1} DPI slider`,
    oninput: (e) => {
      big.value = e.target.value;
      e.target.style.cssText = fill(Number(e.target.value));
    },
    onchange: (e) => commit(e.target.value),
  });
  function commit(v) {
    let dpi = Math.round(Number(v) / P.DPI_STEP) * P.DPI_STEP;
    dpi = Math.min(P.DPI_MAX, Math.max(P.DPI_MIN, dpi || st.dpi));
    if (dpi === st.dpi) return render();
    saveWrite(`Stage ${cur + 1} DPI`, P.writes.stageDpi(cur, dpi));
  }

  const stages = s.stages.slice(0, count).map((x, i) => h("button", {
    class: "stage", "aria-pressed": String(i === cur), disabled: off,
    title: i === cur ? "Current stage" : "Switch to this stage",
    onclick: () => i !== cur && saveWrite("DPI stage", P.writes.currentStage(i)),
  }, x.dpi === null ? "12800+" : String(x.dpi), h("span", { class: "bar", style: `background:${x.color ?? "transparent"}` })));

  return h("section", { class: "card span-2" },
    h("div", { class: "card-heading" },
      h("span", { class: "card-title" }, "Sensitivity", info("sensitivity")),
      h("span", { class: "colour" }, "Stages", countSel)),
    h("div", { class: "dpi-head" },
      h("div", { class: "dpi-value" }, big, h("span", { class: "dpi-unit" }, "DPI")),
      h("label", { class: "colour" }, `Stage ${cur + 1} colour`,
        h("input", {
          type: "color", class: "swatch", value: st.color ?? "#ffffff", disabled: off,
          onchange: (e) => saveWrite(`Stage ${cur + 1} colour`, P.writes.stageColor(cur, e.target.value)),
        }))),
    editable ? [slider, h("div", { class: "range-ends" }, h("span", {}, P.DPI_MIN), h("span", {}, P.DPI_MAX.toLocaleString("en-US")))]
      : h("p", { class: "muted" }, "This stage is above 12,800 DPI, which this page can't edit yet."),
    h("div", { class: "stages" }, stages));
}

// ---------------------------------------------------------------------------
// buttons

const BUTTON_LABELS = ["Left button", "Right button", "Middle button", "Front side", "Rear side", "DPI button"];
const MOUSE_BUTTONS = ["Left click", "Middle click", "Right click", "Back (button 4)", "Forward (button 5)"];

function buttonsPage(s, off) {
  const rows = BUTTON_LABELS.map((label, i) => {
    const code = s.buttons[i];
    const current = P.actionFor(code);
    return h("div", { class: "assign" },
      h("span", { class: "n" }, i + 1),
      h("span", { class: "label" }, label),
      h("select", {
        "aria-label": label, disabled: off || !code,
        onchange: (e) => saveWrite(label, P.writes.button(i, e.target.value)),
      },
      !current && h("option", { selected: true, disabled: true }, code ? `Other (${P.bytesToHex(code)})` : "Unreadable"),
      P.ACTIONS.map((a) => h("option", { value: a.id, selected: current?.id === a.id },
        a.status === "inferred" ? `${a.label} (untested)` : a.label))));
  });

  const out = h("strong", {}, "Press any mouse button here");
  const tester = h("div", {
    class: "tester", tabindex: "0",
    onmousedown: (e) => {
      e.preventDefault();
      out.textContent = MOUSE_BUTTONS[e.button] ?? `Button ${e.button + 1}`;
    },
    // Keep Back/Forward from navigating away while testing.
    onmouseup: (e) => e.preventDefault(),
    onauxclick: (e) => e.preventDefault(),
    oncontextmenu: (e) => e.preventDefault(),
  }, out, h("span", {}, "Shows what each button sends after remapping"));

  return h("div", { class: "grid" },
    h("section", { class: "card span-2" },
      h("div", { class: "card-heading" }, h("span", { class: "card-title" }, "Button assignments"),
        h("span", { class: "card-meta" }, "Keyboard keys and macros aren't supported yet")),
      rows),
    h("section", { class: "card span-2" },
      h("div", { class: "card-heading" }, h("span", { class: "card-title" }, "Button test")),
      tester));
}

// ---------------------------------------------------------------------------
// device

function devicePage() {
  const b = state.battery;
  const kv = (k, v) => h("div", { class: "kv" }, h("span", {}, k), h("span", { class: "v" }, v));
  const connection = card("Connection",
    h("span", { class: "badge" }, state.awake ? "Connected" : "Asleep"),
    kv("Mouse", "Hyperlight"),
    kv("Connected by", state.dev.demo ? "Demo" : state.dev.wired ? "USB cable" : "Wireless receiver"),
    kv("Mouse ID", state.id ?? "–"),
    kv("Battery", state.shown === null ? "–" : `${state.shown}%`),
    kv("Voltage", b ? `${(b.millivolts / 1000).toFixed(2)} V` : "–"),
    kv("Charging", b ? (b.charging ? "Yes" : "No") : "–"));

  const fileInput = h("input", { type: "file", accept: "application/json,.json", hidden: true, onchange: restore });
  const download = () => {
    const data = {
      app: "hitscan-configurator",
      version: 1,
      savedAt: new Date().toISOString(),
      deviceId: state.id,
      longRange: state.longRange,
      block: P.bytesToHex(state.raw),
    };
    const a = h("a", {
      href: URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: "application/json" })),
      download: `hyperlight-settings-${new Date().toISOString().slice(0, 10)}.json`,
    });
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };
  async function restore(e) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    let target;
    try {
      const data = JSON.parse(await file.text());
      target = P.hexToBytes(data.block);
      if (data.app !== "hitscan-configurator" || target.length !== P.BLOCK_LEN) throw new Error("not a backup");
    } catch {
      return toast("That file isn't a settings backup from this page", "error");
    }
    const list = P.diffWrites(state.raw, target);
    if (!list.length) return toast("Your mouse already matches that backup");
    if (!confirm(`Restore ${list.length} setting${list.length > 1 ? "s" : ""} from this backup?`)) return;
    save("Backup", () => writeAll(list));
  }

  const backup = card("Settings backup", null,
    h("p", { class: "muted", style: "margin:0 0 18px" }, "Save your settings to a file, or put a saved file back on the mouse."),
    h("div", { class: "btn-row" },
      h("button", { class: "btn", onclick: download }, "Export"),
      h("button", { class: "btn", disabled: !state.awake, onclick: () => fileInput.click() }, "Import"),
      fileInput),
    h("details", {},
      h("summary", {}, "Raw settings data"),
      h("pre", { class: "raw" },
        Array.from({ length: P.BLOCK_LEN / 16 }, (_, r) =>
          `${(r * 16).toString(16).padStart(4, "0")}  ${P.bytesToHex(state.raw.subarray(r * 16, r * 16 + 16))}\n`))));

  return h("div", { class: "grid" }, connection, backup);
}

// ---------------------------------------------------------------------------
// tray

/** The tray app's own settings: two cards at the top of Performance when served by Hyperlight Tray. */
function trayCards() {
  const t = state.tray;
  const test = async () => {
    try {
      await trayApi.test();
      toast("Notification sent");
    } catch {
      toast("Couldn't reach Hyperlight Tray", "error");
    }
  };
  const alerts = card(["Battery alerts", info("lowAlert")], null,
    segments([0, 5, 10, 15, 20, 25, 30].map((v) => ({ label: v ? String(v) : "Off", unit: v ? "%" : " ", value: v })),
      t.threshold, (v) => saveTray("Low battery alert", { threshold: v }), false, 7),
    h("div", { class: "row", style: "margin-top:10px" },
      h("span", { class: "row-name" }, "Fully charged alert", info("fullAlert")),
      toggle("Fully charged alert", t.notifyFull, (on) => saveTray("Fully charged alert", { notifyFull: on }))));
  const app = card("Hyperlight Tray", h("span", { class: "badge" }, `v${t.version}`),
    h("div", { class: "row" },
      h("span", { class: "row-name" }, "Start with Windows", info("autostart")),
      toggle("Start with Windows", t.autostart, (on) => saveTray("Start with Windows", { autostart: on }))),
    h("div", { class: "row" },
      h("span", { class: "row-name" }, "Test notification"),
      h("button", { class: "btn btn-sm", onclick: test }, "Send")));
  return [alerts, app];
}

// ---------------------------------------------------------------------------
// start

document.addEventListener("visibilitychange", () => !document.hidden && tick());

render();
loadTray();
if (webHidSupported()) {
  HidMouse.reconnect().then((m) => m && attach(m)).catch((err) => console.warn(err));
}
