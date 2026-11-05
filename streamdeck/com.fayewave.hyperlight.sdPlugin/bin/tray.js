// Talks to Hyperlight Tray, which serves the battery reading on 127.0.0.1:47820
// (or the next free port up to 47824).
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { execFile, spawn } from "node:child_process";

export const DEFAULT_PORTS = [47820, 47821, 47822, 47823, 47824];
export const RELEASES = "https://github.com/fayewave/hyperlight-tray/releases/latest";

function request(port, method, urlPath, timeout = 1500) {
  return new Promise((resolve, reject) => {
    const body = method === "POST" ? "{}" : null;
    const headers = body
      ? {
          // The tray only accepts changes that look like they come from its own page.
          Origin: `http://127.0.0.1:${port}`,
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        }
      : {};
    const req = http.request({ host: "127.0.0.1", port, method, path: urlPath, headers, timeout }, (res) => {
      let text = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (text += c));
      res.on("end", () => resolve({ status: res.statusCode, body: text }));
    });
    req.on("timeout", () => req.destroy(new Error("timeout")));
    req.on("error", reject);
    req.end(body ?? undefined);
  });
}

export class Tray {
  constructor(ports = DEFAULT_PORTS) {
    this.ports = ports;
    this.port = null;
  }

  /**
   * The battery as the tray shows it ({ state, percent, charging, remaining, update }),
   * { state: "outdated" } for a tray too old to have the battery API, or null when no
   * tray answers.
   */
  async battery() {
    const order = this.port ? [this.port, ...this.ports.filter((p) => p !== this.port)] : this.ports;
    for (const port of order) {
      try {
        const r = await request(port, "GET", "/api/battery");
        if (r.status === 200) {
          const json = JSON.parse(r.body);
          if (json.app === "hyperlight-tray") {
            this.port = port;
            return json;
          }
        } else if (r.status === 404 && (await request(port, "GET", "/api/tray")).status === 200) {
          this.port = port;
          return { state: "outdated" };
        }
      } catch {
        // nothing on this port
      }
    }
    this.port = null;
    return null;
  }

  /** Asks the tray to read the battery now. */
  async refresh() {
    if (this.port) await request(this.port, "POST", "/api/battery/refresh").catch(() => {});
  }

  configuratorUrl() {
    return this.port ? `http://localhost:${this.port}/` : null;
  }
}

function run(file, args) {
  return new Promise((resolve) => {
    execFile(file, args, { windowsHide: true, timeout: 5000 }, (err, stdout) => resolve(err ? "" : stdout));
  });
}

/** Whether hyperlight-tray.exe is running (an old one has no battery API). */
export async function trayIsRunning() {
  const out = await run("tasklist", ["/FI", "IMAGENAME eq hyperlight-tray.exe", "/NH"]);
  return out.toLowerCase().includes("hyperlight-tray.exe");
}

/** Where Hyperlight Tray is installed: its Start with Windows entry, or the usual folder. */
export async function findTrayExe() {
  const out = await run("reg", ["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "HyperlightTray"]);
  const fromRun = out.match(/REG_SZ\s+"?([^"\r\n]+?)"?\s*$/m)?.[1];
  const usual = process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, "Programs", "HyperlightTray", "hyperlight-tray.exe");
  return [fromRun, usual].find((p) => p && fs.existsSync(p)) ?? null;
}

export function launch(exe) {
  spawn(exe, [], { detached: true, stdio: "ignore", windowsHide: false }).unref();
}
