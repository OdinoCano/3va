// Test ES2024+ polyfills
var tests = [];

// Object.groupBy
try {
  var arr = [1, 2, 3, 4, 5];
  var grouped = Object.groupBy(arr, function(n) { return n % 2 === 0 ? 'even' : 'odd'; });
  tests.push('Object.groupBy: ' + (grouped.odd.length === 3 && grouped.even.length === 2 ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Object.groupBy FAIL: ' + e.message); }

// Map.groupBy
try {
  var mapped = Map.groupBy([1, 2, 3], function(n) { return n > 2 ? 'big' : 'small'; });
  tests.push('Map.groupBy: ' + (mapped.get('small').length === 2 ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Map.groupBy FAIL: ' + e.message); }

// Promise.withResolvers
try {
  var wr = Promise.withResolvers();
  tests.push('Promise.withResolvers: ' + (typeof wr.resolve === 'function' && typeof wr.reject === 'function' && wr.promise instanceof Promise ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Promise.withResolvers FAIL: ' + e.message); }

// Array.prototype.toSorted
try {
  var sorted = [3, 1, 2].toSorted();
  tests.push('Array.toSorted: ' + (sorted[0] === 1 && sorted[2] === 3 && sorted !== [3,1,2] ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Array.toSorted FAIL: ' + e.message); }

// Array.prototype.toReversed
try {
  var rev = [1, 2, 3].toReversed();
  tests.push('Array.toReversed: ' + (rev[0] === 3 && rev[2] === 1 ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Array.toReversed FAIL: ' + e.message); }

// Array.prototype.toSpliced
try {
  var spliced = [1, 2, 3, 4].toSpliced(1, 2);
  tests.push('Array.toSpliced: ' + (spliced.length === 2 && spliced[0] === 1 && spliced[1] === 4 ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Array.toSpliced FAIL: ' + e.message); }

// Array.prototype.with
try {
  var withArr = [1, 2, 3].with(1, 99);
  tests.push('Array.with: ' + (withArr[1] === 99 && withArr[0] === 1 ? 'PASS' : 'FAIL'));
} catch(e) { tests.push('Array.with FAIL: ' + e.message); }

// RegExp.escape
try {
  var esc = RegExp.escape('hello.world');
  tests.push('RegExp.escape: ' + (esc === 'hello\\.world' ? 'PASS' : 'FAIL (' + esc + ')'));
} catch(e) { tests.push('RegExp.escape FAIL: ' + e.message); }

tests.forEach(function(t) { console.log(t); });
