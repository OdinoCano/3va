// Chat bench client: opens C WebSocket connections; each sends M messages as
// fast as it can and counts every delivery (the server broadcasts each message
// to all C connections). Round-trip latency is measured per message on the
// connection that sent it. Prints one line:
//   "<deliveries> <elapsed_ms> <msgs_per_s> <p99_rt_ms>"
//
// Usage: node chat/client.js <port> [clients] [messages_per_client]

const WebSocket = require("ws");

const port = Number(process.argv[2] || 47123);
const C = Number(process.argv[3] || 16);
const M = Number(process.argv[4] || 80);

const t0 = process.hrtime.bigint();
let deliveries = 0;
let closed = 0;
const rtts = [];
// Every message is broadcast to all C connections, so a full run delivers
// exactly C (senders) × M (messages) × C (broadcasts) frames.
const expected = C * M * C;

function now_ms() {
  return Number(process.hrtime.bigint() - t0) / 1e6;
}

function finish(settle_ms) {
  const elapsed = now_ms();
  const per_s = Math.round(deliveries / (elapsed / 1000));
  const sorted = rtts.slice().sort((a, b) => a - b);
  const p99 = sorted.length
    ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.99))]
    : 0;
  console.log(`${deliveries} ${elapsed.toFixed(1)} ${per_s} ${p99.toFixed(2)}`);
  process.exit(deliveries === expected ? 0 : 1);
}

for (let i = 0; i < C; i++) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/chat`);
  const sendTimes = new Map();
  ws.on("open", () => {
    for (let j = 0; j < M; j++) {
      sendTimes.set(j, now_ms());
      ws.send(`c${i}:${j}`);
    }
  });
  ws.on("message", (data) => {
    deliveries++;
    if (deliveries >= expected) {
      // All broadcasts received; let any trailing frames land, then finish.
      setTimeout(() => finish(0), 100);
      return;
    }
    const s = data.toString();
    const sep = s.indexOf(":");
    if (sep === -1) return;
    const who = s.slice(1, sep);
    if (who === String(i)) {
      const j = Number(s.slice(sep + 1));
      const t = sendTimes.get(j);
      if (t !== undefined) rtts.push(now_ms() - t);
    }
  });
  ws.on("close", () => {
    closed++;
    if (closed === C) finish(0);
  });
  ws.on("error", () => {});
}

setTimeout(() => finish(0), 30000);