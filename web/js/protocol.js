// Hitscan mouse protocol: framing, commands, the settings map and the battery
// estimate. No browser APIs in here, so it can be tested under Node.
// See PROTOCOL.md for where each fact comes from.

export const VID = 0x3770;
export const USAGE_PAGE = 0xff02;
export const PID_WIRED = 0x0100;
export const REPORT_ID = 0x08;
export const REPORT_LEN = 17; // including the report ID byte

export const CMD = {
  online: 0x03,
  battery: 0x04,
  writeFlash: 0x07,
  readFlash: 0x08,
  setLongRange: 0x16,
  getLongRange: 0x17,
};

export const BLOCK_LEN = 0xc0;
export const CHUNK = 10;

/** Full 17-byte packet: [0x08, cmd, args..., checksum]. */
export function buildPacket(cmd, args = []) {
  const b = new Uint8Array(REPORT_LEN);
  b[0] = REPORT_ID;
  b[1] = cmd;
  b.set(args.slice(0, REPORT_LEN - 3), 2);
  b[REPORT_LEN - 1] = checksum(b.subarray(0, REPORT_LEN - 1));
  return b;
}

export function checksum(bytes) {
  let c = 0x55;
  for (const x of bytes) c = (c - x) & 0xff;
  return c;
}

/** `r` is a full 17-byte reply including the report ID. */
export function isValidReply(cmd, r) {
  return (
    r.length >= REPORT_LEN &&
    r[0] === REPORT_ID &&
    r[1] === cmd &&
    r[REPORT_LEN - 1] === checksum(r.subarray(0, REPORT_LEN - 1))
  );
}

export const flashArgs = (addr, len) => [0x00, (addr >> 8) & 0xff, addr & 0xff, len];

// ---------------------------------------------------------------------------
// Value encodings

export const encodePair = (v) => [v & 0xff, (0x55 - v) & 0xff];
export const encodeQuad = ([a, b, c]) => [a, b, c, (0x55 - a - b - c) & 0xff];

export function pairAt(raw, off) {
  const v = raw[off];
  return raw[off + 1] === ((0x55 - v) & 0xff) ? v : null;
}

export function quadAt(raw, off) {
  const [a, b, c, k] = raw.slice(off, off + 4);
  return k === ((0x55 - a - b - c) & 0xff) ? [a, b, c] : null;
}

// ---------------------------------------------------------------------------
// Settings map. `status` says how sure we are (see PROTOCOL.md).

export const OFF = {
  polling: 0x00,
  stageCount: 0x02,
  currentStage: 0x04,
  lod: 0x0a,
  dpi: 0x0c, // 8 quads
  colors: 0x2c, // 8 quads
  buttons: 0x60, // 16 quads
  debounce: 0xa9,
  motionSync: 0xab,
  angleSnap: 0xaf,
  ripple: 0xb1,
  sleepEnabled: 0xb5,
  sleepTime: 0xb7,
  highPerf: 0xb9,
};

export const MAX_STAGES = 8;
export const BUTTON_COUNT = 6;

export const POLLING = [
  { hz: 1000, code: 1, status: "confirmed" },
  { hz: 500, code: 2, status: "confirmed" },
  { hz: 250, code: 4, status: "confirmed" },
  { hz: 125, code: 8, status: "confirmed" },
];

export const LOD = [
  { mm: 1, code: 1 },
  { mm: 2, code: 2 },
];

/** Auto Sleep choices from the official app, stored in units of 10 s. 45 min (270) doesn't fit a byte, so it's left out. */
export const SLEEP = [
  { label: "10 sec", units: 1 },
  { label: "30 sec", units: 3 },
  { label: "1 min", units: 6 },
  { label: "2 min", units: 12 },
  { label: "5 min", units: 30 },
  { label: "10 min", units: 60 },
  { label: "15 min", units: 90 },
  { label: "30 min", units: 180 },
];

export const DEBOUNCE_MAX = 20;

/** DPI step encoding: DPI = (step + 1) * 50. Only the low byte is decoded, so 50..12,800. */
export const DPI_MIN = 50;
export const DPI_MAX = 12800;
export const DPI_STEP = 50;
export const dpiToStep = (dpi) => Math.round(dpi / DPI_STEP) - 1;
export const stepToDpi = (s) => (s + 1) * DPI_STEP;

/** Button actions we know the encoding of. */
export const ACTIONS = [
  { id: "left", label: "Left click", code: [1, 0x01, 0] },
  { id: "right", label: "Right click", code: [1, 0x02, 0] },
  { id: "middle", label: "Middle click", code: [1, 0x04, 0] },
  { id: "back", label: "Back (button 4)", code: [1, 0x08, 0] },
  { id: "forward", label: "Forward (button 5)", code: [1, 0x10, 0] },
  { id: "dpiLoop", label: "DPI loop", code: [2, 0x01, 0] },
  { id: "dpiUp", label: "DPI up", code: [2, 0x02, 0], status: "inferred" },
  { id: "dpiDown", label: "DPI down", code: [2, 0x03, 0], status: "inferred" },
];

/** How the official app numbers the Hyperlight's buttons. */
export const BUTTON_NAMES = [
  "Button 1 · left",
  "Button 2 · right",
  "Button 3 · wheel",
  "Button 4 · side, front",
  "Button 5 · side, rear",
  "Button 6 · DPI (underside)",
];

export function actionFor(code) {
  if (!code) return null;
  return ACTIONS.find((a) => a.code.every((v, i) => v === code[i])) ?? null;
}

/** Decodes a 192-byte block into plain values. Fields that fail their check byte come back as null. */
export function parseSettings(raw) {
  const stageCount = pairAt(raw, OFF.stageCount);
  const stages = [];
  for (let i = 0; i < MAX_STAGES; i++) {
    const d = quadAt(raw, OFF.dpi + i * 4);
    const c = quadAt(raw, OFF.colors + i * 4);
    stages.push({
      // A non-zero third byte means DPI above 12,800, which we can't decode yet.
      dpi: d && d[2] === 0 ? stepToDpi(d[0]) : null,
      dpiY: d && d[2] === 0 ? stepToDpi(d[1]) : null,
      dpiRaw: d,
      color: c ? rgbToHex(c) : null,
    });
  }
  const buttons = [];
  for (let i = 0; i < 16; i++) buttons.push(quadAt(raw, OFF.buttons + i * 4));
  const bool = (off) => {
    const v = pairAt(raw, off);
    return v === null ? null : v !== 0;
  };
  const pollCode = pairAt(raw, OFF.polling);
  return {
    pollingHz: POLLING.find((p) => p.code === pollCode)?.hz ?? null,
    stageCount,
    currentStage: pairAt(raw, OFF.currentStage),
    stages,
    lodMm: LOD.find((l) => l.code === pairAt(raw, OFF.lod))?.mm ?? null,
    debounceMs: pairAt(raw, OFF.debounce),
    motionSync: bool(OFF.motionSync),
    angleSnap: bool(OFF.angleSnap),
    ripple: bool(OFF.ripple),
    highPerf: bool(OFF.highPerf),
    sleepEnabled: bool(OFF.sleepEnabled),
    sleepUnits: pairAt(raw, OFF.sleepTime),
    buttons,
  };
}

export function rgbToHex([r, g, b]) {
  return "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");
}

export function hexToRgb(hex) {
  const n = parseInt(hex.replace("#", ""), 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

// ---------------------------------------------------------------------------
// Writes: each returns { addr, bytes } for one flash write.

export const pairWrite = (off, v) => ({ addr: off, bytes: encodePair(v) });
export const quadWrite = (off, q) => ({ addr: off, bytes: encodeQuad(q) });

export const writes = {
  polling: (hz) => pairWrite(OFF.polling, POLLING.find((p) => p.hz === hz).code),
  stageCount: (n) => pairWrite(OFF.stageCount, n),
  currentStage: (i) => pairWrite(OFF.currentStage, i),
  stageDpi: (i, dpi) => {
    const s = dpiToStep(dpi);
    return quadWrite(OFF.dpi + i * 4, [s, s, 0]);
  },
  stageColor: (i, hex) => quadWrite(OFF.colors + i * 4, hexToRgb(hex)),
  lod: (mm) => pairWrite(OFF.lod, LOD.find((l) => l.mm === mm).code),
  debounce: (ms) => pairWrite(OFF.debounce, ms),
  motionSync: (on) => pairWrite(OFF.motionSync, on ? 1 : 0),
  angleSnap: (on) => pairWrite(OFF.angleSnap, on ? 1 : 0),
  ripple: (on) => pairWrite(OFF.ripple, on ? 1 : 0),
  highPerf: (on) => pairWrite(OFF.highPerf, on ? 1 : 0),
  sleepEnabled: (on) => pairWrite(OFF.sleepEnabled, on ? 1 : 0),
  sleepTime: (units) => pairWrite(OFF.sleepTime, units),
  button: (i, actionId) => quadWrite(OFF.buttons + i * 4, ACTIONS.find((a) => a.id === actionId).code),
};

/**
 * Field-by-field writes that turn `current` into `target` (both raw blocks),
 * limited to fields we understand. Used to restore a backup.
 */
export function diffWrites(current, target) {
  const out = [];
  const pairs = [OFF.polling, OFF.stageCount, OFF.currentStage, OFF.lod, OFF.debounce, OFF.motionSync,
    OFF.angleSnap, OFF.ripple, OFF.highPerf, OFF.sleepEnabled, OFF.sleepTime];
  for (const off of pairs) {
    const v = pairAt(target, off);
    if (v !== null && v !== pairAt(current, off)) out.push(pairWrite(off, v));
  }
  const quads = [];
  for (let i = 0; i < MAX_STAGES; i++) quads.push(OFF.dpi + i * 4, OFF.colors + i * 4);
  for (let i = 0; i < 16; i++) quads.push(OFF.buttons + i * 4);
  for (const off of quads) {
    const t = quadAt(target, off);
    const c = quadAt(current, off);
    if (t && (!c || t.some((v, k) => v !== c[k]))) out.push(quadWrite(off, t));
  }
  return out;
}

// ---------------------------------------------------------------------------
// Battery: the official app's voltage curve and smoothing (port of src/estimate.rs).

export const CURVE_MV = [3050, 3420, 3480, 3540, 3600, 3660, 3720, 3760, 3800, 3840, 3880, 3920, 3940,
  3960, 3980, 4000, 4020, 4040, 4060, 4080, 4110];

export function percentFromMv(mv, charging) {
  if (mv >= CURVE_MV[20]) return charging ? 99 : 100;
  const i = CURVE_MV.findIndex((v) => mv < v);
  if (i < 0) return 0;
  if (i === 0) return 1;
  const lo = CURVE_MV[i - 1];
  const step = Math.trunc((CURVE_MV[i] - lo) / 5);
  const p = 5 * (i - 1) + Math.trunc((mv - lo) / step);
  return p === 0 || p === 15 ? p + 1 : p;
}

export function parseBattery(r) {
  if (r[6] > 100) return null;
  return { percent: r[6], charging: r[7] !== 0, millivolts: (r[8] << 8) | r[9] };
}

const target = (b) => (b.millivolts > 0 ? percentFromMv(b.millivolts, b.charging) : b.percent);

export function seed(t, saved, now) {
  if (!saved || now < saved.unix) return t;
  const elapsed = now - saved.unix;
  if (elapsed > 1800) return t;
  if (elapsed < 60) return saved.percent;
  const up = Math.min(100, saved.percent + Math.trunc(elapsed * 0.028));
  const down = Math.max(0, saved.percent - Math.trunc(elapsed * 0.014));
  if (t > up) return up;
  if (t < down) return down;
  return saved.percent;
}

function step(state, t, b) {
  let next = 10;
  if (b.charging) {
    if (t >= 95 && state.shown >= 85) {
      next = 300;
      if (state.shown < 99) state.shown++;
    } else if (t - state.shown > 10) state.shown++;
    if (b.percent >= 100) state.shown = 100;
  } else if (state.shown > t) {
    if (b.percent === 0) state.shown = 0;
    else {
      state.shown--;
      if (b.percent <= 15 && state.shown > 15) state.shown = 15;
    }
  }
  state.shown = Math.min(state.shown, 100);
  return next;
}

/** Same behaviour as the tray app's Estimator. `now` is Unix seconds. */
export class Estimator {
  constructor() {
    this.shown = null;
    this.lastTick = 0;
    this.interval = 10;
  }
  update(b, now, saved) {
    const t = target(b);
    if (this.shown === null) {
      this.shown = seed(t, saved, now);
      this.interval = step(this, t, b);
      this.lastTick = now;
      return this.shown;
    }
    if (now < this.lastTick) this.lastTick = now;
    let ticks = 0;
    while (now - this.lastTick >= this.interval && ticks < 10000) {
      this.lastTick += this.interval;
      this.interval = step(this, t, b);
      ticks++;
    }
    if (ticks === 10000) this.lastTick = now;
    if (b.percent === 0) this.shown = 0;
    else if (b.percent <= 15 && this.shown > 15) this.shown = 15;
    if (b.charging && b.percent >= 100) this.shown = 100;
    return this.shown;
  }
}

/** A real block read from a Hyperlight on 2026-11-05, used by tests and the demo device. */
export const SAMPLE_BLOCK_HEX =
  "01 54 03 52 02 53 00 55 00 55 01 54 1f 1f 00 17 1f 1f 00 17 1f 1f 00 17 3f 3f 00 d7 3f 3f 00 d7 3f 3f 00 d7 3f 3f 00 d7 3f 3f 00 d7 00 fb 02 58 f2 22 66 db 0a 0a ff 42 82 04 ff d0 82 04 ff d0 82 04 ff d0 82 04 ff d0 82 04 ff d0 02 53 80 d5 03 52 00 55 ff 00 ff 57 00 55 80 d5 03 52 00 55 01 01 00 53 01 02 00 52 01 04 00 50 01 08 00 4c 01 10 00 44 02 01 00 52 02 02 00 51 02 03 00 50 07 00 00 4e 08 04 00 49 04 0a 03 44 00 00 00 55 00 00 00 55 00 00 00 55 00 00 00 55 00 00 00 55 01 ff 00 ff 07 09 46 00 55 00 55 00 55 06 4f 00 55 00 55 00 55 01 54 0c 49 00 55 0a 4b ff ff ff";

export const hexToBytes = (s) => Uint8Array.from(s.trim().split(/\s+/).map((h) => parseInt(h, 16)));
export const bytesToHex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join(" ");
