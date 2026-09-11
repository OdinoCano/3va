// 09 - Process Manager
// A simple HTTP server designed to run as a managed background process.

const http = require("http");

const PORT = process.env.PORT || 4000;

// Simulate a crash every ~30 requests to demonstrate auto-restart
let requestCount = 0;

const server = http.createServer((req, res) => {
  requestCount += 1;

  if (requestCount % 30 === 0) {
    console.error("Simulated crash after request #" + requestCount);
    process.exit(1);
  }

  res.writeHead(200, { "Content-Type": "application/json" });
  res.end(
    JSON.stringify({
      pid: process.pid,
      uptime: process.uptime().toFixed(1) + "s",
      requestCount,
    })
  );
});

server.listen(PORT, () => {
  console.log(`[worker] listening on :${PORT} (pid ${process.pid})`);
});