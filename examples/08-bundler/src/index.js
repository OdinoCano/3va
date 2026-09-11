// 08 - Bundler: entry point
// Bundle this module plus its imports into a single self-contained file.

import { formatTime } from "./utils.js";

export function createMessage(name) {
  return {
    greeting: `Hello, ${name}!`,
    timestamp: formatTime(new Date()),
    generatedBy: "3va bundler",
  };
}

// Standalone execution (3va run src/index.js or 3va run dist/bundle.js) prints
// the message. In a browser page (where the bundled output is used as a
// <script>), `window` exists — instead we attach createMessage() globally so
// the page can call it after the bundle loads.
if (typeof window === "undefined") {
  console.log(createMessage("3va"));
} else {
  globalThis.createMessage = createMessage;
}