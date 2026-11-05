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

test("key shows the number in the tray's colours", () => {
  const svg = (s) => keySvg(view(s));
  assert.match(svg({ state: "battery", percent: 85, charging: false, remaining: "~12h" }), new RegExp(`fill="${COLORS.healthy}">85<.*>~12h<`));
  assert.match(svg({ state: "battery", percent: 35, charging: true }), new RegExp(`fill="${COLORS.moderate}">35<.*>Charging<`));
  assert.match(svg({ state: "battery", percent: 8, charging: false }), new RegExp(`fill="${COLORS.low}">8<`));
  assert.match(svg({ state: "battery", percent: 100 }), /font-size="76"[^>]*>100</);
  assert.match(svg({ state: "asleep", percent: 60 }), new RegExp(`fill="${COLORS.stale}">60<.*>Asleep<`));
  assert.match(svg({ state: "noReceiver" }), />–<.*>No receiver</);
  assert.match(svg({ state: "stopped" }), />Start tray</);
  assert.match(svg({ state: "missing" }), />Get tray</);
  assert.match(svg(undefined), />–</);
  assert.doesNotMatch(svg({ state: "battery", percent: 85 }), /circle/);
  assert.match(svg({ state: "battery", percent: 85, update: "v0.2.0" }), /circle/);
  assert.match(svg({ state: "battery", percent: 85, remaining: "<b>" }), />&lt;b&gt;</);
});

test("key options hide the time remaining (number centred) and the update dot", () => {
  const s = { state: "battery", percent: 85, charging: false, remaining: "~12h", update: "v0.2.0" };
  const all = keySvg(view(s, options({})));
  assert.match(all, /y="90"[^>]*>85<.*>~12h<.*circle/);
  const bare = keySvg(view(s, options({ showRemaining: false, showUpdate: false })));
  assert.match(bare, /y="106"[^>]*>85</);
  assert.doesNotMatch(bare, /~12h|circle/);
  // Charging is still shown: it isn't an estimate.
  assert.match(keySvg(view({ ...s, charging: true }, options({ showRemaining: false }))), />Charging</);
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

    sd.send({ event: "willAppear", action: "com.fayewave.hyperlight.battery", context: "key1", payload: { settings: {} } });
    const first = await until(() => sd.got.find((m) => m.event === "setImage" && m.context === "key1"), "first image");
    assert.ok(first.payload.image.startsWith("data:image/svg+xml,"));
    assert.match(imageOf(first), new RegExp(`fill="${COLORS.healthy}">60<.*>~16h<`));

    // Unchanged readings aren't re-sent; a new one is.
    const images = () => sd.got.filter((m) => m.event === "setImage");
    await new Promise((r) => setTimeout(r, 500));
    assert.equal(images().length, 1);
    tray.reading = { ...tray.reading, percent: 15, update: "v0.2.0" };
    const second = await until(() => images()[1], "updated image");
    assert.match(imageOf(second), new RegExp(`fill="${COLORS.low}">15<.*circle`));

    // Turning options off in the key's settings redraws it without them.
    sd.send({ event: "didReceiveSettings", context: "key1", payload: { settings: { showRemaining: false, showUpdate: false } } });
    const third = await until(() => images()[2], "image without options");
    assert.match(imageOf(third), /y="106"[^>]*>15</);
    assert.doesNotMatch(imageOf(third), /~16h|circle/);

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
    const gone = await until(() => images().find((m) => /tray</.test(imageOf(m))), "tray-gone image", 8000);
    assert.match(imageOf(gone), />(Start|Get|Update) tray</);

    // The plugin exits when Stream Deck closes the connection.
    sd.close();
    assert.equal(await exited, 0);
  } finally {
    child.kill();
    sd.server.close();
    tray.server.close();
  }
});
