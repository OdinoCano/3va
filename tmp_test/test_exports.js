// Test subpath exports resolution
try {
  var evalMod = require('es-errors/eval');
  console.log('PASS: es-errors/eval resolved, type:', typeof evalMod);
} catch (e) {
  console.log('FAIL: es-errors/eval failed:', e.message);
}

try {
  var root = require('es-errors');
  console.log('PASS: es-errors root resolved, type:', typeof root);
} catch (e) {
  console.log('FAIL: es-errors root failed:', e.message);
}

try {
  var hidden = require('es-errors/internal.js');
  console.log('FAIL: es-errors/internal.js should have thrown');
} catch (e) {
  if (e.message && e.message.indexOf('ERR_PACKAGE_PATH_NOT_EXPORTED') >= 0) {
    console.log('PASS: ERR_PACKAGE_PATH_NOT_EXPORTED thrown for internal path');
  } else {
    console.log('PARTIAL: Got error but no ERR code:', e.message);
  }
}

// Test require.resolve
try {
  var r = require.resolve('es-errors');
  console.log('PASS: require.resolve works:', typeof r === 'string');
} catch (e) {
  console.log('FAIL: require.resolve:', e.message);
}

// Test require.cache
try {
  var cacheKeys = Object.keys(require.cache).filter(function(k) { return k.indexOf('es-errors') >= 0; });
  console.log('PASS: require.cache has es-errors entries:', cacheKeys.length > 0);
} catch (e) {
  console.log('FAIL: require.cache:', e.message);
}

// Test require.main
try {
  console.log('PASS: require.main exists:', typeof require.main === 'object');
  console.log('PASS: require.main.filename:', require.main.filename);
} catch (e) {
  console.log('FAIL: require.main:', e.message);
}

// Test createRequire
try {
  var mod = require('module');
  var cr = mod.createRequire('/');
  console.log('PASS: createRequire works, type:', typeof cr);
} catch (e) {
  console.log('FAIL: createRequire:', e.message);
}
