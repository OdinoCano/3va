// WebSocket chat room: every message received is broadcast to every connected
// client, exactly the workload Bun publishes as "WebSocket chat: 32 clients
// broadcasting". PORT from the environment so the three runtimes can run side
// by side.
//
// Branches by runtime like bench/server.js: 3va and Node serve via http +
// the `ws` npm library over server.on('upgrade'); Bun uses its native
// Bun.serve WebSocket (the pubsub loop below is deliberately identical so the
// comparison is the runtime, not two different broadcast designs).

const port = Number(process.env.PORT || 47123);

function broadcast(clients, data) {
  for (const c of clients) {
    try {
      c.send(data);
    } catch (e) {
      /* client mid-close */
    }
  }
}

if (typeof Bun !== "undefined") {
  const clients = new Set();
  Bun.serve({
    port,
    fetch(req, server) {
      if (server.upgrade(req)) return undefined;
      return new Response("ok");
    },
    websocket: {
      open(ws) {
        clients.add(ws);
      },
      close(ws) {
        clients.delete(ws);
      },
      message(ws, msg) {
        broadcast(clients, typeof msg === "string" ? msg : String(msg));
      },
    },
  });
  console.log("listening");
} else {
  const http = require("http");
  const { WebSocketServer } = require("ws");
  const server = http.createServer((req, res) => {
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end("ok");
  });
  const wss = new WebSocketServer({ noServer: true });
  const clients = new Set();
  wss.on("connection", (ws) => {
    clients.add(ws);
    ws.on("message", (data) => broadcast(clients, data.toString()));
    ws.on("close", () => clients.delete(ws));
    ws.on("error", () => clients.delete(ws));
  });
  server.on("upgrade", (req, socket, head) => {
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit("connection", ws, req));
  });
  server.listen(port, () => console.log("listening"));
}