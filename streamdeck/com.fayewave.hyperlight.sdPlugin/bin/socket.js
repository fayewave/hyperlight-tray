// A minimal WebSocket client (RFC 6455) for talking to the Stream Deck app, so the
// plugin needs no npm packages. Text messages only; pings are answered.
import http from "node:http";
import crypto from "node:crypto";

const GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

export const OP = { continuation: 0, text: 1, binary: 2, close: 8, ping: 9, pong: 10 };

/** One frame. Clients must mask what they send; servers must not. */
export function encodeFrame(opcode, payload, mask = true) {
  const data = Buffer.isBuffer(payload) ? payload : Buffer.from(payload ?? "", "utf8");
  const len = data.length;
  const m = mask ? 0x80 : 0;
  let head;
  if (len < 126) {
    head = Buffer.from([0x80 | opcode, m | len]);
  } else if (len < 65536) {
    head = Buffer.alloc(4);
    head[0] = 0x80 | opcode;
    head[1] = m | 126;
    head.writeUInt16BE(len, 2);
  } else {
    head = Buffer.alloc(10);
    head[0] = 0x80 | opcode;
    head[1] = m | 127;
    head.writeBigUInt64BE(BigInt(len), 2);
  }
  if (!mask) return Buffer.concat([head, data]);
  const key = crypto.randomBytes(4);
  const out = Buffer.alloc(len);
  for (let i = 0; i < len; i++) out[i] = data[i] ^ key[i & 3];
  return Buffer.concat([head, key, out]);
}

/** Splits a byte stream into frames: push() chunks, get onFrame(opcode, fin, payload). */
export class FrameReader {
  constructor(onFrame) {
    this.buf = Buffer.alloc(0);
    this.onFrame = onFrame;
  }

  push(chunk) {
    this.buf = this.buf.length ? Buffer.concat([this.buf, chunk]) : chunk;
    for (;;) {
      const b = this.buf;
      if (b.length < 2) return;
      const fin = (b[0] & 0x80) !== 0;
      const opcode = b[0] & 0x0f;
      const masked = (b[1] & 0x80) !== 0;
      let len = b[1] & 0x7f;
      let off = 2;
      if (len === 126) {
        if (b.length < 4) return;
        len = b.readUInt16BE(2);
        off = 4;
      } else if (len === 127) {
        if (b.length < 10) return;
        len = Number(b.readBigUInt64BE(2));
        off = 10;
      }
      const keyAt = off;
      if (masked) off += 4;
      if (b.length < off + len) return;
      let payload = b.subarray(off, off + len);
      if (masked) {
        payload = Buffer.from(payload);
        for (let i = 0; i < len; i++) payload[i] ^= b[keyAt + (i & 3)];
      }
      this.buf = b.subarray(off + len);
      this.onFrame(opcode, fin, payload);
    }
  }
}

/**
 * Connects to ws://127.0.0.1:<port>. Returns { send(text), close() }; onOpen gets the
 * same object once the handshake is done, onMessage gets each text message.
 */
export function connect(port, { onOpen, onMessage, onClose }) {
  const key = crypto.randomBytes(16).toString("base64");
  let socket = null;
  let closed = false;
  const done = (why) => {
    if (closed) return;
    closed = true;
    socket?.destroy();
    onClose?.(why);
  };
  const ws = {
    send(text) {
      if (socket && !closed) socket.write(encodeFrame(OP.text, text));
    },
    close() {
      if (socket && !closed) socket.write(encodeFrame(OP.close, Buffer.alloc(0)));
      done("closed");
    },
  };

  const req = http.request({
    host: "127.0.0.1",
    port,
    path: "/",
    headers: {
      Connection: "Upgrade",
      Upgrade: "websocket",
      "Sec-WebSocket-Key": key,
      "Sec-WebSocket-Version": "13",
    },
  });
  req.on("upgrade", (res, sock, head) => {
    const expected = crypto.createHash("sha1").update(key + GUID).digest("base64");
    if (res.headers["sec-websocket-accept"] !== expected) {
      sock.destroy();
      return done("bad handshake");
    }
    socket = sock;
    sock.setNoDelay(true);
    let messageOp = 0;
    let chunks = [];
    const reader = new FrameReader((op, fin, payload) => {
      if (op === OP.close) return done("server closed");
      if (op === OP.ping) return sock.write(encodeFrame(OP.pong, payload));
      if (op === OP.pong) return;
      if (op !== OP.continuation) {
        messageOp = op;
        chunks = [];
      }
      chunks.push(payload);
      if (fin) {
        if (messageOp === OP.text) onMessage?.(Buffer.concat(chunks).toString("utf8"));
        chunks = [];
      }
    });
    sock.on("data", (c) => reader.push(c));
    sock.on("close", () => done("socket closed"));
    sock.on("error", () => done("socket error"));
    onOpen?.(ws);
    if (head?.length) reader.push(head);
  });
  req.on("response", () => done("no upgrade"));
  req.on("error", (e) => done(e.message));
  req.end();
  return ws;
}
