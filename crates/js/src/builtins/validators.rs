// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Validadores de argumentos con los mensajes de Node 24.
//!
//! Node valida los argumentos de sus APIs y lanza `ERR_INVALID_ARG_TYPE`,
//! `ERR_INVALID_ARG_VALUE`, `ERR_OUT_OF_RANGE` o `ERR_MISSING_ARGS` con un
//! `code`, una clase (`TypeError`/`RangeError`) y un mensaje exacto. 3va
//! aceptaba cualquier valor: un `fs.readFile(42)` no fallaba hasta que la
//! llamada al sistema fallaba por otro motivo.
//!
//! Los mensajes son los de `lib/internal/validators.js` y
//! `lib/internal/errors.js` de Node 24 (`The "name" argument must be of type
//! string. Received undefined`), escritos aquí de forma independiente: son el
//! texto que un usuario ve, no una copia del código de Node.
//!
//! Se publican como `globalThis.__validators` para los builtins que los
//! usan.

use v8::{ContextScope, HandleScope};

/// Install the shared argument validators.
pub fn inject_validators(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    crate::builtins::code_cache::bootstrap_js(scope, "validators", VALIDATORS_JS)?;
    Ok(())
}

const VALIDATORS_JS: &str = r##"
(function () {
    'use strict';

    var util = globalThis.__requireCache['util'] || globalThis.__requireCache['node:util'];

    function inspect(value) {
        if (util && typeof util.inspect === 'function') {
            try { return util.inspect(value); } catch (e) { /* fall through */ }
        }
        if (value === null) return 'null';
        if (value === undefined) return 'undefined';
        var t = typeof value;
        if (t === 'string') return "'" + value + "'";
        if (t === 'number' || t === 'boolean' || t === 'bigint') return String(value);
        if (t === 'function') {
            return value.name ? '[Function: ' + value.name + ']' : '[Function (anonymous)]';
        }
        if (Array.isArray(value)) return '[Array]';
        return '[Object]';
    }

    // Node's `getPrefix`: `The "name" argument`, with the special cases for
    // `node:`-prefixed and dot-prefixed internal names.
    function prefix(name, thing) {
        thing = thing || 'argument';
        if (typeof name !== 'string' || name.length === 0) return 'The ' + thing;
        if (name.startsWith('node:')) {
            var rest = name.slice(5);
            if (rest.startsWith('internal/')) return 'The ' + rest.slice(9) + ' ' + thing;
        }
        if (name[0] === '.') return 'The ' + name.slice(1) + ' ' + thing;
        return 'The "' + name + '" ' + thing;
    }

    function invalidArgType(name, expected, actual) {
        var err = new TypeError(prefix(name) + ' must be ' + expected + '. Received ' + inspect(actual));
        err.code = 'ERR_INVALID_ARG_TYPE';
        return err;
    }

    function invalidArgValue(name, actual, reason) {
        var err = new TypeError(prefix(name) + ' received an invalid value.' + (reason ? ' ' + reason : '') +
            ' Received ' + inspect(actual));
        err.code = 'ERR_INVALID_ARG_VALUE';
        return err;
    }

    function outOfRange(name, value, min, max) {
        var range = min !== undefined && max !== undefined
            ? 'It must be >= ' + min + ' and <= ' + max + '.'
            : (min !== undefined ? 'It must be >= ' + min + '.' : 'It must be <= ' + max + '.');
        var err = new RangeError('The value of ' + prefix(name) + ' is out of range. ' + range +
            ' Received ' + inspect(value));
        err.code = 'ERR_OUT_OF_RANGE';
        return err;
    }

    function missingArgs(args) {
        var err = new TypeError('The ' + args + ' argument must be provided');
        err.code = 'ERR_MISSING_ARGS';
        return err;
    }

    var validators = {
        inspect: inspect,

        validateString: function (value, name) {
            if (typeof value !== 'string') throw invalidArgType(name, 'of type string', value);
            return value;
        },

        validateNumber: function (value, name) {
            if (typeof value !== 'number') throw invalidArgType(name, 'of type number', value);
            return value;
        },

        validateInteger: function (value, name, min, max) {
            if (typeof value !== 'number') throw invalidArgType(name, 'of type number', value);
            if (!Number.isInteger(value)) throw invalidArgType(name, 'an integer', value);
            if ((min !== undefined && value < min) || (max !== undefined && value > max)) {
                throw outOfRange(name, value, min, max);
            }
            return value;
        },

        validateFunction: function (value, name) {
            if (typeof value !== 'function') throw invalidArgType(name, 'of type function', value);
            return value;
        },

        validateObject: function (value, name) {
            if (value === null || typeof value !== 'object') {
                throw invalidArgType(name, 'of type object', value);
            }
            return value;
        },

        validateBuffer: function (value, name) {
            var isBuffer = typeof Buffer !== 'undefined' && Buffer.isBuffer && Buffer.isBuffer(value);
            if (!isBuffer && !(value instanceof Uint8Array)) {
                throw invalidArgType(name, 'an instance of Buffer or Uint8Array', value);
            }
            return value;
        },

        validateAbortSignal: function (value, name) {
            if (value === null || typeof value !== 'object' ||
                typeof AbortSignal === 'undefined' || !(value instanceof AbortSignal)) {
                throw invalidArgType(name, 'an instance of AbortSignal', value);
            }
            return value;
        },

        validateOneOf: function (value, values, name) {
            if (values.indexOf(value) === -1) {
                var err = new TypeError('The value "' + String(value) + '" is invalid for argument "' +
                    name + '". Allowed values: ' + values.join(', '));
                err.code = 'ERR_INVALID_ARG_VALUE';
                throw err;
            }
            return value;
        },

        invalidArgType: invalidArgType,
        invalidArgValue: invalidArgValue,
        outOfRange: outOfRange,
        missingArgs: missingArgs
    };

    globalThis.__validators = validators;
})();
"##;
