//! `Intl.*` conformance patches (ECMA-402 / test262 `intl402/`).
//!
//! V8 150 ships a near-complete `Intl` (ICU 76): `Intl.NumberFormat`,
//! `Intl.DateTimeFormat`, `Intl.Collator`, `Intl.PluralRules`, `Intl.Locale`,
//! `Intl.ListFormat`, `Intl.RelativeTimeFormat`, `Intl.DisplayNames`,
//! `Intl.Segmenter`, `Intl.DurationFormat`, plus `Temporal` — all pass the
//! bulk of the test262 `intl402/` suite out of the box.
//!
//! A handful of tests target the *very latest* spec + CLDR behavior, which the
//! ICU version embedded in this V8 build predates. Those are patched here as a
//! small JS layer over the native `Intl`:
//!
//! 1. `String.prototype.toLocaleLowerCase/UpperCase` validate *every* locale
//!    identifier (delegating to `Intl.getCanonicalLocales`).
//! 2. `Intl.getCanonicalLocales` only drops a `yes` value for the boolean
//!    extension keys (`kb`/`kc`/`kh`/`kk`/`kn`), preserving it for the others
//!    (`ka`/`kf`/`kr`/`ks`/`kv`).
//!
//! Two hard constraints keep these patches safe:
//!
//! - The `Intl.*` constructor/function objects themselves must keep their
//!   intrinsic identity: `name`, `length`, the `{ [[Writable]]: true,
//!   [[Enumerable]]: false }` property descriptor, being non-constructible
//!   (no `prototype`) where applicable, and `[[FallbackSymbol]]` internals.
//!   Replacing `Intl.DateTimeFormat`, `Intl.supportedValuesOf`, etc. with a
//!   plain `function` breaks `name`/`length`/`builtin`/`supportedLocalesOf`/
//!   `legacy-constructed-symbol` tests, so only the two APIs below are layered,
//!   and each replacement is an arrow function or a non-constructible method
//!   with the exact same `name`, `length` and descriptors.
//!
//! - Anything requiring ICU/CLDR data V8's ICU lacks is left unfixed by design:
//!   legacy non-IANA time-zone abbreviations, U+2212 offsets, the `islamic` /
//!   `islamic-rgsa` calendar fallback, `Etc/GMT+13`/`+14`, simple-digit
//!   numbering systems and a few `Temporal`↔`Intl` interactions. Filtering the
//!   `supportedValuesOf("calendar")` list or extending `"timeZone"` would
//!   silently contradict what `DateTimeFormat` actually accepts and regress
//!   other tests.

use v8::ContextScope;
use v8::HandleScope;

const INTL_SHIM: &str = r#"(function () {
  'use strict';

  // ---- 1) toLocaleLowerCase/UpperCase validate EVERY locale identifier
  //         (CanonicalizeLocaleList throws a RangeError on any invalid one,
  //         not just the first). The replacements are non-constructible object
  //         methods: no own "prototype", `this` still dynamic, and the native
  //         name/length/descriptors are preserved.
  ['toLocaleLowerCase', 'toLocaleUpperCase'].forEach(function (name) {
    var orig = String.prototype[name];
    var wrapper = {
      [name](locales) {
        if (locales !== undefined) Intl.getCanonicalLocales(locales);
        return orig.apply(this, arguments);
      }
    }[name];
    Object.defineProperty(wrapper, 'length', {
      value: orig.length,
      writable: false,
      enumerable: false,
      configurable: true,
    });
    Object.defineProperty(String.prototype, name, {
      value: wrapper,
      writable: true,
      enumerable: false,
      configurable: true,
    });
  });

  // ---- 2) getCanonicalLocales: only the boolean Unicode extension keys
  //         (kb/kc/kh/kk/kn) canonicalise a "yes" type off; ka/kf/kr/ks/kv
  //         must keep it. Arrow replacement: non-constructible, no prototype,
  //         name "getCanonicalLocales", length 1 (matches the native builtin).
  var OrigGCL = Intl.getCanonicalLocales;
  var KEEP_YES = /-u-(ka|kf|kr|ks|kv)-yes$/i;
  Intl.getCanonicalLocales = (locales) => {
    var keeps = [];
    if (locales !== undefined && locales !== null) {
      var items;
      if (typeof locales === 'string') {
        items = [locales];
      } else if (typeof locales[Symbol.iterator] === 'function') {
        items = Array.from(locales);
      } else {
        items = [locales];
      }
      for (var i = 0; i < items.length; i++) {
        var el = items[i];
        if (typeof el === 'string' && KEEP_YES.test(el)) keeps.push(el);
      }
    }
    var result = OrigGCL(locales);
    result:
    for (var i = 0; i < result.length; i++) {
      var low = result[i].toLowerCase();
      for (var j = 0; j < keeps.length; j++) {
        var keptLow = keeps[j].toLowerCase();
        if (keptLow === low) {
          result[i] = keeps[j];
          continue result;
        }
        if (keptLow === low + '-yes') {
          result[i] = low + '-yes';
          continue result;
        }
      }
    }
    return result;
  };
  // This V8 build does not run "named evaluation" for property-assigned arrow
  // functions, so pin the name explicitly; the length (1) and the intrinsic
  // name/length descriptors already match the native builtin.
  Object.defineProperty(Intl.getCanonicalLocales, 'name', {
    value: 'getCanonicalLocales',
    writable: false,
    enumerable: false,
    configurable: true,
  });
  Object.defineProperty(Intl, 'getCanonicalLocales', {
    value: Intl.getCanonicalLocales,
    writable: true,
    enumerable: false,
    configurable: true,
  });
})();
"#;

pub fn inject_intl(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    crate::builtins::code_cache::compile_and_run_cached(scope, "intl-shim", INTL_SHIM)?;
    Ok(())
}
