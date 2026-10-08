// A real native addon (bcrypt, built on node-addon-api) loaded through Node-API.
// Loading a .node file needs --allow-ffi: native code runs outside the sandbox.
const bcrypt = require('bcrypt');

const hash = bcrypt.hashSync('correct horse', 10);
console.log('hash        :', hash);
console.log('right pass  :', bcrypt.compareSync('correct horse', hash));
console.log('wrong pass  :', bcrypt.compareSync('battery staple', hash));

// The async API runs on a native worker thread and completes on the event loop.
bcrypt
  .hash('async secret', 10)
  .then((h) => bcrypt.compare('async secret', h))
  .then((ok) => console.log('async check :', ok));
