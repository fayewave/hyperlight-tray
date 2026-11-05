// A pretend mouse with the same interface as HidMouse, for trying the page
// without hardware. It starts from a real Hyperlight's settings.
import * as P from "./protocol.js";

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

export class DemoMouse {
  constructor() {
    this.name = "Demo Hyperlight";
    this.demo = true;
    this.wired = false;
    this.block = P.hexToBytes(P.SAMPLE_BLOCK_HEX);
    this.long = false;
    this.millivolts = 3820;
  }
  async close() {}
  async status() {
    return { awake: true, id: "demo00" };
  }
  async battery() {
    await wait(20);
    this.millivolts = Math.max(3500, this.millivolts - 1);
    return { percent: 40, charging: false, millivolts: this.millivolts };
  }
  async read(addr, len) {
    await wait(30);
    return this.block.slice(addr, addr + len);
  }
  readBlock() {
    return this.read(0, P.BLOCK_LEN);
  }
  async write({ addr, bytes }) {
    await wait(60);
    this.block.set(bytes, addr);
    return true;
  }
  async longRange() {
    return this.long;
  }
  async setLongRange(on) {
    await wait(60);
    this.long = on;
    return true;
  }
}
