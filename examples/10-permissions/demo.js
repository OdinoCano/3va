#!/usr/bin/env 3va
// 10 - Permissions
// Deny-by-default security demo: every capability 3va blocks unless you
// explicitly grant it. Run WITHOUT privileges to see what gets denied,
// then grant them one by one.

const fs = require("fs");

console.log("=== 3va deny-by-default permission demo ===\n");

// --- 1. Filesystem: read a file ---
try {
  const data = fs.readFileSync("./secret.txt", "utf-8");
  console.log("[ok]   read file:", data.trim());
} catch (err) {
  console.log("[DENIED] read file ->", err.code || err.message);
}

// --- 2. Filesystem: write a file ---
try {
  fs.writeFileSync("./output.txt", "written by 3va demo");
  console.log("[ok]   wrote file: ./output.txt");
} catch (err) {
  console.log("[DENIED] write file ->", err.code || err.message);
}

// --- 3. Environment variables ---
try {
  const envKeys = Object.keys(process.env);
  console.log("[ok]   env vars visible:", envKeys.length > 0 ? envKeys.slice(0, 5).join(", ") : "(none)");
} catch (err) {
  console.log("[DENIED] env vars ->", err.message);
}

// --- 4. Child processes ---
try {
  const { execSync } = require("child_process");
  const out = execSync("echo I am a child process", { encoding: "utf-8" }).trim();
  console.log("[ok]   child process ->", out);
} catch (err) {
  console.log("[DENIED] child process ->", err.code || err.message);
}

console.log("\n=== How to grant each capability ===");
console.log("  --allow-read=./secret.txt          read files");
console.log("  --allow-write=./output.txt         write files");
console.log("  --allow-net=localhost              network access");
console.log("  --allow-env                        all environment variables");
console.log("  --allow-child-process              spawn child processes");
console.log("");
console.log("Or grant everything at once:");
console.log("  3va run demo.js --allow-read=./secret.txt --allow-write=./output.txt \\");
console.log("    --allow-net=localhost --allow-env --allow-child-process");
console.log("");

// --- 5. Network: outbound request (async, reported last) ---
// Without --allow-net the http module reports a permission denial.
// With --allow-net the connection proceeds — ECONNREFUSED here because
// there's no server on :9999, but the permission check passed.
const http = require("http");
http
  .get("http://localhost:9999/", (res) => {
    console.log("[ok]   network fetch (status", res.statusCode + ")");
  })
  .on("error", (err) => {
    const msg = String(err.message || err);
    const denied = /permission|denied|allow-net/i.test(msg);
    console.log(denied
      ? "[DENIED] network fetch -> no --allow-net grant"
      : "[ok]   network fetch (permission granted — ECONNREFUSED, no server on :9999)");
  });
