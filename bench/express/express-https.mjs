// Express 5 hello world served over HTTPS — the workload Bun publishes as
// "Serving Express over HTTPS". Same file for every runtime; the key and
// certificate are a throwaway self-signed pair bench/run.sh generates.
import express from "express";
import https from "node:https";
import fs from "node:fs";

const app = express();
const port = Number(process.env.PORT || 3443);

app.get("/", (req, res) => {
  res.send("Hello World!");
});

https
  .createServer(
    { key: fs.readFileSync(process.env.TLS_KEY), cert: fs.readFileSync(process.env.TLS_CERT) },
    app,
  )
  .listen(port, () => {
    console.log(`Listening on port ${port}`);
  });
