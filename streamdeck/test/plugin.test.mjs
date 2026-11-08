// node --test streamdeck/test
import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

import { view, options, keySvg, COLORS } from "../com.fayewave.hyperlight.sdPlugin/bin/key.js";
import { DIGITS, TEXT } from "../com.fayewave.hyperlight.sdPlugin/bin/glyphs.js";
import { encodeFrame, FrameReader, OP } from "../com.fayewave.hyperlight.sdPlugin/bin/socket.js";

/** Regex source for str drawn from the bundled outlines, every glyph in that colour (spaces draw nothing). */
const reEscape = (t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const drawn = (str, fill, font = TEXT) =>
  [...String(str)]
    .filter((c) => font[c].d)
    .map((c) => `d="${reEscape(font[c].d)}" fill="${fill}"`)
    .join(".*");
/** The same, for the battery number's Optician Sans outlines. */
const digits = (str, fill) => drawn(str, fill, DIGITS);
const GREY = "#8E8E93";
const name = drawn("Hyperlight", "#fff");

const PLUGIN = fileURLToPath(new URL("../com.fayewave.hyperlight.sdPlugin/bin/plugin.js", import.meta.url));

test("key: Hyperlight and a caps line on top, the percent and bar (orange/red when low) along the bottom", () => {
  const svg = (s) => keySvg(view(s, options({ showUpdate: true })));
  // The number and its % in one colour: white/grey when healthy, orange or red when low.
  const num = (n, color, pct = color) => new RegExp(`${name}.*${digits(n, color)}.*${digits("%", pct)}`);
  const k85 = svg({ state: "battery", percent: 85, charging: false, remaining: "~12h" });
  assert.match(k85, num(85, "#fff", GREY));
  // The caps line sits under the name, above the number.
  assert.match(k85, new RegExp(`translate\\(9 23\\)[^>]*${drawn("H", "#fff")}.*translate\\(9 42\\)[^>]*${drawn("~", GREY)}.*${drawn("12H", GREY)}.*translate\\(8 76\\) scale\\(0.046\\)`));
  // Along the bottom: grey track the full 78 wide, green over it up to 85%.
  assert.match(k85, new RegExp(`<rect x="9" y="84" width="78"[^>]*fill="#2C2C2E"/>.*<rect x="9" y="84" width="66.3"[^>]*fill="${COLORS.healthy}"`));
  // The dot sits at 85% along the 78-wide bar starting at x=9.
  assert.match(k85, /<circle cx="75.3" cy="86.5"/);
  assert.match(svg({ state: "battery", percent: 26 }), num(26, "#fff", GREY));
  assert.match(svg({ state: "battery", percent: 25, charging: true }), new RegExp(`${drawn("CHARGING", GREY)}.*${digits(25, COLORS.charging)}.*<rect x="9" y="84" width="19.5"[^>]*fill="${COLORS.charging}"`));
  assert.match(svg({ state: "battery", percent: 20 }), num(20, COLORS.moderate));
  const k8 = svg({ state: "battery", percent: 8 });
  assert.match(k8, num(8, COLORS.low));
  // The bar takes the number's colour when low.
  assert.match(k8, new RegExp(`<rect x="9" y="84" width="6.24"[^>]*fill="${COLORS.low}"`));
  assert.match(svg({ state: "battery", percent: 22 }), new RegExp(`<rect x="9" y="84" width="17.16"[^>]*fill="${COLORS.moderate}"`));
  assert.match(svg({ state: "battery", percent: 100 }), /<circle cx="87"/);
  assert.match(keySvg(view({ state: "asleep", percent: 60 }, options({ showAsleep: true }))), new RegExp(`${name}.*${drawn("ASLEEP", GREY)}.*${digits(60, "#fff")}.*opacity="1"`));
  assert.match(svg({ state: "noReceiver" }), new RegExp(`${drawn("NO RECEIVER", GREY)}.*${digits("–", GREY)}`));
  assert.match(svg({ state: "stopped" }), new RegExp(`${drawn("TRAY OFF", GREY)}.*${drawn("Start", GREY)}`));
  assert.match(svg({ state: "outdated" }), new RegExp(`${drawn("OLD TRAY", GREY)}.*${drawn("Update", GREY)}`));
  assert.match(svg({ state: "missing" }), new RegExp(`${drawn("NO TRAY", GREY)}.*${drawn("Get", GREY)}`));
  assert.match(svg(undefined), new RegExp(digits("–", "#6C6C70")));
  assert.doesNotMatch(svg({ state: "battery", percent: 85 }), /fill="#64D2FF"/);
  assert.match(svg({ state: "battery", percent: 85, update: "v0.2.0" }), /fill="#64D2FF"/);
  // Text from the tray is drawn, never put in the SVG as markup.
  const odd = svg({ state: "battery", percent: 85, remaining: "<b>" });
  assert.match(odd, new RegExp(drawn("<B>", GREY)));
  assert.doesNotMatch(odd, /<b>|<B>|<text/);
});

test("key options: charging and time left on by default, asleep and the update dot off", () => {
  const s = { state: "battery", percent: 85, charging: false, remaining: "~12h", update: "v0.2.0" };
  const remaining = drawn("~12H", GREY);
  // Defaults: the time left shows, the update dot doesn't.
  const plain = keySvg(view(s, options({})));
  assert.match(plain, new RegExp(remaining));
  assert.doesNotMatch(plain, /#64D2FF/);
  assert.match(keySvg(view(s)), new RegExp(remaining));
  assert.match(keySvg(view(s, options({ showUpdate: true }))), new RegExp(`${remaining}.*fill="#64D2FF"`));
  const bare = keySvg(view(s, options({ showRemaining: false, showUpdate: false })));
  assert.match(bare, new RegExp(digits(85, "#fff")));
  assert.doesNotMatch(bare, new RegExp(`${remaining}|#64D2FF`));
  // Charging and Asleep have their own options. Charging keeps the %, and has no bolt.
  const charging = { ...s, charging: true, remaining: null };
  const chargingKey = keySvg(view(charging, options({})));
  assert.match(chargingKey, new RegExp(`${drawn("CHARGING", GREY)}.*${digits("%", COLORS.charging)}`));
  assert.doesNotMatch(chargingKey, /M13\.8/); // the old bolt
  const noCharging = keySvg(view(charging, options({ showCharging: false })));
  // Still blue without the text.
  assert.match(noCharging, new RegExp(`${digits(85, COLORS.charging)}.*${digits("%", COLORS.charging)}`));
  assert.doesNotMatch(noCharging, new RegExp(`${drawn("CHARGING", GREY)}|M13\\.8`));
  const asleep = { state: "asleep", percent: 42 };
  assert.match(keySvg(view(asleep, options({ showAsleep: true }))), new RegExp(drawn("ASLEEP", GREY)));
  const noAsleep = keySvg(view(asleep, options({})));
  // Asleep keeps showing the last percent and bar as they were, not dimmed.
  assert.match(noAsleep, new RegExp(`${digits(42, "#fff")}.*<g opacity="1"><rect[^>]*fill="${COLORS.healthy}"`));
  assert.match(keySvg(view({ state: "asleep", percent: 12 }, options({}))), new RegExp(`${digits(12, COLORS.low)}.*fill="${COLORS.low}"`));
  assert.doesNotMatch(noAsleep, new RegExp(drawn("ASLEEP", GREY)));
});

test("key: all text drawn from the bundled outlines, no fonts needed", () => {
  const k = keySvg(view({ state: "battery", percent: 85, remaining: "~12h" }));
  assert.doesNotMatch(k, /<text|font-family/);
  // The number: one path per glyph, 6% of the size closer than the font's spacing.
  assert.match(k, new RegExp(`translate\\(8 76\\) scale\\(0.046\\)" ${digits(8, "#fff")}.*translate\\(30.54 76\\) scale\\(0.046\\)" ${digits(5, "#fff")}.*scale\\(0.026\\)" ${digits("%", GREY)}`));
  // "100%" a size down to fit, and ends inside the key.
  const k100 = keySvg(view({ state: "battery", percent: 100 }));
  assert.match(k100, new RegExp(`scale\\(0.04\\)" ${digits(1, "#fff")}.*${digits(0, "#fff")}.*${digits(0, "#fff")}.*scale\\(0.023\\)" ${digits("%", GREY)}`));
  const pctX = Number(k100.match(/translate\(([\d.]+) 76\) scale\(0.023\)/)[1]);
  assert.ok(pctX + DIGITS["%"].w * 23 <= 92, `100% ends at ${pctX + DIGITS["%"].w * 23}`);
  // Long labels shrink to fit before they're cut: NO RECEIVER fits whole.
  assert.doesNotMatch(keySvg(view({ state: "noReceiver" })), new RegExp(drawn("…", GREY)));
  // Every glyph either font is asked for is there.
  for (const c of "0123456789%–") assert.ok(DIGITS[c]?.d.startsWith("M"), c);
  for (let c = 33; c < 127; c++) assert.ok(TEXT[String.fromCharCode(c)]?.d.startsWith("M"), String.fromCharCode(c));
  assert.ok(TEXT[" "].w > 0 && !TEXT[" "].d);
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

    sd.send({ event: "willAppear", action: "com.fayewave.hyperlight.battery", context: "key1", payload: { settings: { showRemaining: true, showUpdate: true } } });
    const first = await until(() => sd.got.find((m) => m.event === "setImage" && m.context === "key1"), "first image");
    assert.ok(first.payload.image.startsWith("data:image/svg+xml,"));
    assert.match(imageOf(first), new RegExp(`${name}.*${drawn("~16H", GREY)}.*${digits(60, "#fff")}`));

    // Unchanged readings aren't re-sent; a new one is.
    const images = () => sd.got.filter((m) => m.event === "setImage");
    await new Promise((r) => setTimeout(r, 500));
    assert.equal(images().length, 1);
    tray.reading = { ...tray.reading, percent: 15, update: "v0.2.0" };
    const second = await until(() => images()[1], "updated image");
    assert.match(imageOf(second), new RegExp(`${name}.*${digits(15, COLORS.low)}.*fill="#64D2FF"`));

    // Turning options off in the key's settings redraws it without them.
    sd.send({ event: "didReceiveSettings", context: "key1", payload: { settings: { showRemaining: false, showUpdate: false } } });
    const third = await until(() => images()[2], "image without options");
    assert.match(imageOf(third), new RegExp(digits(15, COLORS.low)));
    assert.doesNotMatch(imageOf(third), new RegExp(`${drawn("~16H", GREY)}|#64D2FF`));

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
    const gone = await until(() => images().find((m) => new RegExp(drawn("TRAY", GREY)).test(imageOf(m))), "tray-gone image", 8000);
    const said = (label, word) => `${drawn(label, GREY)}.*${drawn(word, GREY)}`;
    assert.match(imageOf(gone), new RegExp(`(?:${said("TRAY OFF", "Start")}|${said("OLD TRAY", "Update")}|${said("NO TRAY", "Get")})`));

    // The plugin exits when Stream Deck closes the connection.
    sd.close();
    assert.equal(await exited, 0);
  } finally {
    child.kill();
    sd.server.close();
    tray.server.close();
  }
});
