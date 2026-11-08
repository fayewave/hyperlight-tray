// node --test streamdeck/test
import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

import { view, options, keySvg, COLORS } from "../com.fayewave.hyperlight.sdPlugin/bin/key.js";
import { encodeFrame, FrameReader, OP } from "../com.fayewave.hyperlight.sdPlugin/bin/socket.js";

const PLUGIN = fileURLToPath(new URL("../com.fayewave.hyperlight.sdPlugin/bin/plugin.js", import.meta.url));

test("key: Hyperlight in white, the percent (orange/red when low), a green 0-100% bar with a dot", () => {
  const all = options({ showRemaining: true, showCharging: true, showAsleep: true });
  const svg = (s) => keySvg(view(s, all));
  // The number and its % in one colour: white/grey when healthy, orange or red when low.
  const num = (n, color, pct = color) => new RegExp(`fill="#fff"[^>]*>Hyperlight<.*fill="${color}"[^>]*>${n}<.*fill="${pct}"[^>]*>%<`);
  const k85 = svg({ state: "battery", percent: 85, charging: false, remaining: "~12h" });
  assert.match(k85, num(85, "#fff", "#8E8E93"));
  assert.match(k85, />~12H</);
  // Grey track the full 78 wide, green over it up to 85%.
  assert.match(k85, new RegExp(`<rect x="9" y="66" width="78"[^>]*fill="#2C2C2E"/>.*<rect x="9" y="66" width="66.3"[^>]*fill="${COLORS.healthy}"`));
  // The dot sits at 85% along the 78-wide bar starting at x=9.
  assert.match(k85, /<circle cx="75.3" cy="68.5"/);
  assert.match(svg({ state: "battery", percent: 26 }), num(26, "#fff", "#8E8E93"));
  assert.match(svg({ state: "battery", percent: 25, charging: true }), new RegExp(`fill="${COLORS.moderate}"[^>]*>25<.*>CHARGING<`));
  assert.match(svg({ state: "battery", percent: 20 }), num(20, COLORS.moderate));
  const k8 = svg({ state: "battery", percent: 8 });
  assert.match(k8, num(8, COLORS.low));
  // The bar stays green when low.
  assert.match(k8, new RegExp(`<rect x="9" y="66"[^>]*fill="${COLORS.healthy}"`));
  assert.match(svg({ state: "battery", percent: 100 }), /<circle cx="87"/);
  assert.match(svg({ state: "asleep", percent: 60 }), /fill="#fff"[^>]*>Hyperlight<.*fill="#6C6C70"[^>]*>60<.*opacity="0.18".*>ASLEEP</);
  assert.match(svg({ state: "noReceiver" }), />No<.*>RECEIVER</);
  assert.match(svg({ state: "stopped" }), />Start<.*>TRAY</);
  assert.match(svg({ state: "outdated" }), /font-size="22"[^>]*>Update<.*>TRAY</);
  assert.match(svg({ state: "missing" }), />Get<.*>TRAY</);
  assert.match(svg(undefined), />–<.*>BATTERY</);
  assert.doesNotMatch(svg({ state: "battery", percent: 85 }), /fill="#64D2FF"/);
  assert.match(svg({ state: "battery", percent: 85, update: "v0.2.0" }), /fill="#64D2FF"/);
  assert.match(svg({ state: "battery", percent: 85, remaining: "<b>" }), />&lt;B&gt;</);
});

test("key options: text lines are opt-in (Battery without them), the dot opt-out", () => {
  const s = { state: "battery", percent: 85, charging: false, remaining: "~12h", update: "v0.2.0" };
  // Defaults: the bottom line says Battery, plus the update dot.
  const plain = keySvg(view(s, options({})));
  assert.match(plain, />BATTERY<.*fill="#64D2FF"/);
  assert.doesNotMatch(plain, /~12H/);
  assert.match(keySvg(view(s)), />BATTERY</);
  assert.match(keySvg(view(s, options({ showRemaining: true }))), />~12H<.*fill="#64D2FF"/);
  const bare = keySvg(view(s, options({ showRemaining: false, showUpdate: false })));
  assert.match(bare, />BATTERY</);
  assert.doesNotMatch(bare, /~12H|#64D2FF/);
  // Charging and Asleep have their own options; charging also gets a bolt.
  const charging = { ...s, charging: true, remaining: null };
  const chargingKey = keySvg(view(charging, options({ showCharging: true })));
  assert.match(chargingKey, /<path[^>]*fill="#30D158".*>CHARGING</);
  assert.doesNotMatch(chargingKey, />%</);
  const noCharging = keySvg(view(charging, options({ showRemaining: true })));
  assert.match(noCharging, />BATTERY</);
  assert.doesNotMatch(noCharging, /CHARGING|<path/);
  const asleep = { state: "asleep", percent: 42 };
  assert.match(keySvg(view(asleep, options({ showAsleep: true }))), />ASLEEP</);
  const noAsleep = keySvg(view(asleep, options({})));
  assert.match(noAsleep, /fill="#6C6C70"[^>]*>42<.*>BATTERY</);
  assert.doesNotMatch(noAsleep, /ASLEEP/);
});

test("websocket frames round-trip at every length encoding", () => {
  for (const len of [0, 5, 125, 126, 300, 65535, 70000]) {
    for (const mask of [true, false]) {
      const payload = crypto.randomBytes(len);
      const frame = encodeFrame(OP.text, payload, mask);
      const got = [];
      const r = new FrameReader((op, fin, p) => got.push({ op, fin, p }));
      // Fed in small pieces, as a socket might deliver it.
      for (let i = 0; i < frame.length; i += 1000) r.push(frame.subarray(i, i + 1000));
      assert.equal(got.length, 1, `len ${len}`);
      assert.equal(got[0].op, OP.text);
      assert.ok(got[0].fin);
      assert.ok(got[0].p.equals(payload), `len ${len} mask ${mask}`);
    }
  }
});

/** A pretend Stream Deck app: a WebSocket server that records what the plugin sends. */
function fakeStreamDeck() {
  const got = [];
  let sock;
  const server = http.createServer();
  server.on("upgrade", (req, s) => {
    const accept = crypto.createHash("sha1").update(req.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
    s.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    sock = s;
    const r = new FrameReader((op, fin, p) => op === OP.text && got.push(JSON.parse(p.toString())));
    s.on("data", (c) => r.push(c));
  });
  return {
    server,
    got,
    send: (msg) => sock.write(encodeFrame(OP.text, JSON.stringify(msg), false)),
    close: () => sock.write(encodeFrame(OP.close, Buffer.alloc(0), false)),
  };
}

/** A pretend Hyperlight Tray. */
function fakeTray() {
  const t = { reading: { app: "hyperlight-tray", state: "battery", percent: 60, charging: false, remaining: "~16h", update: null }, refreshes: 0 };
  t.server = http.createServer((req, res) => {
    if (req.method === "GET" && req.url === "/api/battery") {
      res.writeHead(200, { "Content-Type": "application/json" });
      return res.end(JSON.stringify(t.reading));
    }
    if (req.method === "POST" && req.url === "/api/battery/refresh") {
      assert.equal(req.headers.origin, `http://127.0.0.1:${t.server.address().port}`);
      assert.match(req.headers["content-type"], /^application\/json/);
      t.refreshes++;
      res.writeHead(200, { "Content-Type": "application/json" });
      return res.end(JSON.stringify(t.reading));
    }
    res.writeHead(404);
    res.end();
  });
  return t;
}

const listen = (server) => new Promise((ok) => server.listen(0, "127.0.0.1", () => ok(server.address().port)));

async function until(check, what, ms = 4000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = check();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 25));
  }
}

const imageOf = (m) => decodeURIComponent(m.payload.image.replace("data:image/svg+xml,", ""));

test("plugin draws the battery, refreshes on press and opens the configurator on hold", async () => {
  const sd = fakeStreamDeck();
  const tray = fakeTray();
  const sdPort = await listen(sd.server);
  const trayPort = await listen(tray.server);
  const child = spawn(process.execPath, [PLUGIN, "-port", String(sdPort), "-pluginUUID", "PLUGIN-UUID", "-registerEvent", "registerPlugin", "-info", "{}"], {
    env: { ...process.env, HYPERLIGHT_TRAY_PORTS: String(trayPort), HYPERLIGHT_POLL_MS: "200" },
    stdio: "inherit",
  });
  const exited = new Promise((ok) => child.on("exit", ok));
  try {
    await until(() => sd.got.find((m) => m.event === "registerPlugin" && m.uuid === "PLUGIN-UUID"), "registration");

    sd.send({ event: "willAppear", action: "com.fayewave.hyperlight.battery", context: "key1", payload: { settings: { showRemaining: true } } });
    const first = await until(() => sd.got.find((m) => m.event === "setImage" && m.context === "key1"), "first image");
    assert.ok(first.payload.image.startsWith("data:image/svg+xml,"));
    assert.match(imageOf(first), />Hyperlight<.*fill="#fff"[^>]*>60<.*>~16H</);

    // Unchanged readings aren't re-sent; a new one is.
    const images = () => sd.got.filter((m) => m.event === "setImage");
    await new Promise((r) => setTimeout(r, 500));
    assert.equal(images().length, 1);
    tray.reading = { ...tray.reading, percent: 15, update: "v0.2.0" };
    const second = await until(() => images()[1], "updated image");
    assert.match(imageOf(second), new RegExp(`>Hyperlight<.*fill="${COLORS.low}"[^>]*>15<.*fill="#64D2FF"`));

    // Turning options off in the key's settings redraws it without them.
    sd.send({ event: "didReceiveSettings", context: "key1", payload: { settings: { showRemaining: false, showUpdate: false } } });
    const third = await until(() => images()[2], "image without options");
    assert.match(imageOf(third), />15<.*>BATTERY</);
    assert.doesNotMatch(imageOf(third), /~16H|#64D2FF/);

    // A quick press asks the tray to read the battery now.
    sd.send({ event: "keyDown", context: "key1" });
    sd.send({ event: "keyUp", context: "key1" });
    await until(() => tray.refreshes === 1, "refresh");

    // Holding opens the configurator instead.
    sd.send({ event: "keyDown", context: "key1" });
    const open = await until(() => sd.got.find((m) => m.event === "openUrl"), "openUrl", 2000);
    assert.equal(open.payload.url, `http://localhost:${trayPort}/`);
    sd.send({ event: "keyUp", context: "key1" });
    await new Promise((r) => setTimeout(r, 100));
    assert.equal(tray.refreshes, 1, "releasing after a hold doesn't also refresh");

    // Tray gone: the key says so.
    await new Promise((ok) => tray.server.close(ok));
    tray.server.closeAllConnections?.();
    const gone = await until(() => images().find((m) => />TRAY</.test(imageOf(m))), "tray-gone image", 8000);
    assert.match(imageOf(gone), />(Start|Get|Update)<.*>TRAY</);

    // The plugin exits when Stream Deck closes the connection.
    sd.close();
    assert.equal(await exited, 0);
  } finally {
    child.kill();
    sd.server.close();
    tray.server.close();
  }
});
