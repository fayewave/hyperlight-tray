// Hyperlight Battery for Stream Deck: shows the mouse battery from Hyperlight Tray
// on a key, in the same text-first style as the Aircon plugin's keys.
//
// Press: re-read the battery. Hold: open the configurator.
// Without the tray running, press starts it (or opens the download page).
import { connect } from "./socket.js";
import { Tray, RELEASES, findTrayExe, launch, trayIsRunning } from "./tray.js";
import { view, options, keySvg, dataUrl } from "./key.js";

const POLL_MS = Number(process.env.HYPERLIGHT_POLL_MS) || 5000;
const HOLD_MS = 500;
/** How often to look for the tray's exe and process while it isn't answering. */
const OFFLINE_CHECK_MS = 30_000;

// Stream Deck starts us with: -port N -pluginUUID U -registerEvent E -info JSON
const args = {};
for (let i = 2; i + 1 < process.argv.length; i += 2) args[process.argv[i].replace(/^-+/, "")] = process.argv[i + 1];

const ports = process.env.HYPERLIGHT_TRAY_PORTS?.split(",").map(Number).filter(Boolean);
const tray = new Tray(ports?.length ? ports : undefined);
/** Visible keys: context -> { image, hold, opts } */
const keys = new Map();
let reading; // last status, undefined until the first poll
let offline = { running: false, exe: null, at: 0 };
let timer = null;
let ws = null;

const send = (event, rest) => ws?.send(JSON.stringify({ event, ...rest }));
const log = (message) => send("logMessage", { payload: { message: `[hyperlight] ${message}` } });

async function status() {
  const b = await tray.battery();
  if (b) return b;
  if (Date.now() - offline.at > OFFLINE_CHECK_MS) {
    offline = { running: await trayIsRunning(), exe: await findTrayExe(), at: Date.now() };
  }
  // Running but not answering means a version from before the Stream Deck plugin.
  return { state: offline.running ? "outdated" : offline.exe ? "stopped" : "missing" };
}

function draw() {
  for (const [context, key] of keys) {
    const image = dataUrl(keySvg(view(reading, key.opts)));
    if (key.image !== image) {
      key.image = image;
      send("setImage", { context, payload: { image, target: 0 } });
    }
  }
}

async function poll() {
  reading = await status();
  draw();
}

function startPolling() {
  if (timer) return;
  poll();
  timer = setInterval(poll, POLL_MS);
}

function stopPolling() {
  clearInterval(timer);
  timer = null;
}

async function press(held) {
  switch (reading?.state) {
    case "missing":
    case "outdated":
      return send("openUrl", { payload: { url: RELEASES } });
    case "stopped":
      launch(offline.exe);
      offline.at = 0;
      setTimeout(poll, 2500);
      return;
  }
  if (held) {
    const url = tray.configuratorUrl();
    if (url) send("openUrl", { payload: { url } });
    return;
  }
  await tray.refresh();
  setTimeout(poll, 1500);
}

function onMessage(text) {
  let msg;
  try {
    msg = JSON.parse(text);
  } catch {
    return;
  }
  const key = keys.get(msg.context);
  switch (msg.event) {
    case "willAppear":
      keys.set(msg.context, { image: null, hold: null, opts: options(msg.payload?.settings) });
      startPolling();
      if (reading !== undefined) draw();
      break;
    case "didReceiveSettings":
      // Changed in the key's settings panel (ui/battery.html).
      if (!key) break;
      key.opts = options(msg.payload?.settings);
      if (reading !== undefined) draw();
      break;
    case "willDisappear":
      clearTimeout(key?.hold);
      keys.delete(msg.context);
      if (!keys.size) stopPolling();
      break;
    case "keyDown":
      if (!key) break;
      clearTimeout(key.hold);
      key.hold = setTimeout(() => {
        key.hold = null;
        press(true);
      }, HOLD_MS);
      break;
    case "keyUp":
      if (key?.hold) {
        clearTimeout(key.hold);
        key.hold = null;
        press(false);
      }
      break;
    case "systemDidWakeUp":
      if (keys.size) poll();
      break;
  }
}

process.on("unhandledRejection", (e) => log(`error: ${e?.stack ?? e}`));

ws = connect(Number(args.port), {
  onOpen: (sock) => sock.send(JSON.stringify({ event: args.registerEvent, uuid: args.pluginUUID })),
  onMessage,
  onClose: () => process.exit(0),
});
