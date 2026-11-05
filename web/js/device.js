// Talks to a Hitscan receiver (or cabled mouse) through WebHID.
import * as P from "./protocol.js";

const REPLY_TIMEOUT_MS = 300;
const ATTEMPTS = 3;

export const webHidSupported = () => typeof navigator !== "undefined" && "hid" in navigator;

const FILTERS = [{ vendorId: P.VID, usagePage: P.USAGE_PAGE }];

const hasCommandChannel = (d) =>
  d.vendorId === P.VID && d.collections.some((c) => c.usagePage === P.USAGE_PAGE);

/** Prefer the cable when the mouse is plugged in, as the tray app does. */
const rank = (d) => (d.productId === P.PID_WIRED ? 0 : 1);

export class HidMouse {
  /** Opens a device the user already granted, if any. */
  static async reconnect() {
    const granted = (await navigator.hid.getDevices()).filter(hasCommandChannel).sort((a, b) => rank(a) - rank(b));
    for (const d of granted) {
      const m = new HidMouse(d);
      if (await m.open()) return m;
    }
    return null;
  }

  /**
   * Shows the browser's device picker. Must be called from a click.
   * Returns undefined if nothing was picked, null if a pick couldn't be opened.
   */
  static async request() {
    const picked = (await navigator.hid.requestDevice({ filters: FILTERS })).filter(hasCommandChannel);
    if (!picked.length) return undefined;
    picked.sort((a, b) => rank(a) - rank(b));
    for (const d of picked) {
      const m = new HidMouse(d);
      if (await m.open()) return m;
    }
    return null;
  }

  constructor(device) {
    this.device = device;
    this.name = device.productName || "Hitscan device";
    this.wired = device.productId === P.PID_WIRED;
    this.pending = null;
    this.queue = Promise.resolve();
    this.onReport = (e) => this.handleReport(e);
  }

  async open() {
    try {
      if (!this.device.opened) await this.device.open();
      this.device.addEventListener("inputreport", this.onReport);
      return true;
    } catch (err) {
      console.warn("Couldn't open", this.device.productName, err);
      return false;
    }
  }

  async close() {
    this.device.removeEventListener("inputreport", this.onReport);
    try {
      await this.device.close();
    } catch {}
  }

  handleReport(e) {
    if (!this.pending) return;
    const r = new Uint8Array(P.REPORT_LEN);
    r[0] = e.reportId;
    r.set(new Uint8Array(e.data.buffer, e.data.byteOffset, Math.min(e.data.byteLength, P.REPORT_LEN - 1)), 1);
    const { cmd, match, resolve } = this.pending;
    // Other programs may talk to the receiver too; take only our own reply.
    if (P.isValidReply(cmd, r) && (!match || match(r))) {
      this.pending = null;
      resolve(r);
    }
  }

  /** One request/reply, serialised with any others in flight. */
  transact(cmd, args = [], match = null) {
    const run = async () => {
      const pkt = P.buildPacket(cmd, args);
      for (let attempt = 0; attempt < ATTEMPTS; attempt++) {
        const reply = new Promise((resolve) => {
          this.pending = { cmd, match, resolve };
          setTimeout(() => resolve(null), REPLY_TIMEOUT_MS);
        });
        try {
          await this.device.sendReport(P.REPORT_ID, pkt.subarray(1));
        } catch (err) {
          this.pending = null;
          throw err;
        }
        const r = await reply;
        this.pending = null;
        if (r) return r;
      }
      return null;
    };
    const p = this.queue.then(run, run);
    this.queue = p.catch(() => {});
    return p;
  }

  async status() {
    const r = await this.transact(P.CMD.online);
    if (!r) return { awake: false, id: null };
    const id = P.bytesToHex(r.subarray(7, 10)).replaceAll(" ", "");
    return { awake: this.wired || r[6] > 0, id };
  }

  async battery() {
    const r = await this.transact(P.CMD.battery);
    return r ? P.parseBattery(r) : null;
  }

  async read(addr, len) {
    const out = new Uint8Array(len);
    for (let done = 0; done < len; done += P.CHUNK) {
      const n = Math.min(P.CHUNK, len - done);
      const a = addr + done;
      const r = await this.transact(P.CMD.readFlash, P.flashArgs(a, n), (r) => r[3] === a >> 8 && r[4] === (a & 0xff));
      if (!r) return null;
      out.set(r.subarray(6, 6 + n), done);
    }
    return out;
  }

  readBlock() {
    return this.read(0, P.BLOCK_LEN);
  }

  /** Writes one field and confirms it by reading it back. */
  async write({ addr, bytes }) {
    for (let done = 0; done < bytes.length; done += P.CHUNK) {
      const chunk = bytes.slice(done, done + P.CHUNK);
      const a = addr + done;
      const r = await this.transact(P.CMD.writeFlash, [...P.flashArgs(a, chunk.length), ...chunk],
        (r) => r[3] === a >> 8 && r[4] === (a & 0xff));
      if (!r) return false;
    }
    const back = await this.read(addr, bytes.length);
    return !!back && bytes.every((v, i) => back[i] === v);
  }

  async longRange() {
    const r = await this.transact(P.CMD.getLongRange);
    return r ? r[6] !== 0 : null;
  }

  async setLongRange(on) {
    const mode = on ? 1 : 0;
    const r = await this.transact(P.CMD.setLongRange, [0, 0, 0, 0x0a, mode]);
    return !!r && r[6] === mode && (await this.longRange()) === on;
  }
}
