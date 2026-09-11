// 03 - File Operations
// Read and write files with scoped permissions.

const fs = require("fs");
const path = require("path");

const DATA_FILE = path.join(__dirname, "data.json");

// Read existing data or start fresh
let items = [];
if (fs.existsSync(DATA_FILE)) {
  items = JSON.parse(fs.readFileSync(DATA_FILE, "utf-8"));
  console.log(`Loaded ${items.length} items from data.json`);
} else {
  console.log("No data.json found, starting fresh");
}

// Add a new item
const newItem = {
  id: items.length + 1,
  text: `Item created at ${new Date().toISOString()}`,
  timestamp: Date.now(),
};
items.push(newItem);

// Write back to disk
fs.writeFileSync(DATA_FILE, JSON.stringify(items, null, 2));
console.log(`Wrote ${items.length} items to data.json`);

// Print the result
console.log("\nAll items:");
items.forEach((item) => console.log(`  #${item.id}: ${item.text}`));
