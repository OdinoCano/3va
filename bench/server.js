// Minimal HTTP server used by throughput.sh and memory.sh — same shape run
// against 3va, Node, and Bun so the comparison measures the runtimes, not
// three different servers. PORT is read from the environment so all three
// can run side by side without colliding.
const port = Number(process.env.PORT || 8811);

if (typeof Bun !== "undefined") {
  Bun.serve({ port, fetch: () => new Response("ok") });
  console.log("listening");
} else {
  const http = require("http");
  // Keep a reference to the server instead of chaining .listen() straight
  // off createServer(): an unreferenced Server can be GC'd once this
  // synchronous script finishes, which silently exits the process even
  // though the socket was still open — assigning it to a variable is what
  // keeps the process alive for incoming connections.
  const server = http.createServer((req, res) => {
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end("ok");
  });
  server.listen(port, () => console.log("listening"));
}
