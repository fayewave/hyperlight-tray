// Run with: node --test web/test
import { test } from "node:test";
import assert from "node:assert/strict";
import * as P from "../js/protocol.js";

const sample = () => P.hexToBytes(P.SAMPLE_BLOCK_HEX);

test("packet framing matches captured traffic", () => {
  // Polling 1000 -> 500 write, as sent and echoed on 2026-11-05.
  const pkt = P.buildPacket(P.CMD.writeFlash, [...P.flashArgs(0, 2), 0x02, 0x53]);
  assert.equal(P.bytesToHex(pkt), "08 07 00 00 00 02 02 53 00 00 00 00 00 00 00 00 ef");
  const reply = P.hexToBytes("08 04 00 00 00 02 19 00 0e 91 00 00 00 00 00 00 8f");
  assert.ok(P.isValidReply(P.CMD.battery, reply));
  assert.deepEqual(P.parseBattery(reply), { percent: 25, charging: false, millivolts: 3729 });
});

test("decodes the real settings block", () => {
  const s = P.parseSettings(sample());
  assert.equal(s.pollingHz, 1000);
  assert.equal(s.stageCount, 3);
  assert.equal(s.currentStage, 2);
  assert.deepEqual(s.stages.slice(0, 3).map((x) => x.dpi), [1600, 1600, 1600]);
  assert.deepEqual(s.stages.slice(0, 3).map((x) => x.color), ["#00fb02", "#f22266", "#0a0aff"]);
  assert.equal(s.lodMm, 1);
  assert.equal(s.debounceMs, 0);
  assert.equal(s.sleepEnabled, true);
  assert.equal(s.sleepUnits, 12);
  for (const k of ["motionSync", "angleSnap", "ripple", "highPerf"]) assert.equal(s[k], false);
  assert.deepEqual(s.buttons.slice(0, 6).map((b) => P.actionFor(b)?.id),
    ["left", "right", "middle", "back", "forward", "dpiLoop"]);
});

test("writes encode with check bytes", () => {
  const raw = sample();
  assert.deepEqual(P.writes.stageDpi(0, 1600), { addr: 0x0c, bytes: Array.from(raw.slice(0x0c, 0x10)) });
  assert.deepEqual(P.writes.stageColor(1, "#f22266").bytes, Array.from(raw.slice(0x30, 0x34)));
  assert.deepEqual(P.writes.polling(500), { addr: 0, bytes: [0x02, 0x53] });
  assert.deepEqual(P.writes.button(3, "back").bytes, Array.from(raw.slice(0x6c, 0x70)));
});

test("diffWrites restores only what differs", () => {
  const a = sample();
  const b = sample();
  b.set(P.encodePair(2), P.OFF.polling);
  b.set(P.encodeQuad([0x3f, 0x3f, 0]), P.OFF.dpi);
  assert.deepEqual(P.diffWrites(a, b), [
    { addr: 0x00, bytes: [0x02, 0x53] },
    { addr: 0x0c, bytes: [0x3f, 0x3f, 0x00, 0xd7] },
  ]);
  assert.deepEqual(P.diffWrites(a, sample()), []);
});

test("battery estimate matches the Rust port", () => {
  assert.equal(P.percentFromMv(3729, false), 31);
  assert.equal(P.percentFromMv(3954, true), 63);
  assert.equal(P.percentFromMv(3540, false), 16);
  const e = new P.Estimator();
  const b = { percent: 55, charging: true, millivolts: 3954 };
  assert.equal(e.update(b, 1000, { percent: 40, unix: 995 }), 41);
  assert.equal(e.update(b, 1030), 44);
  assert.equal(e.update(b, 2000), 53);
});
