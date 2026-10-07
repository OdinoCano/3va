// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_js::builtins::mqtt::encode_remaining_length;

// MQTT variable-byte "remaining length". The encoder lives in Rust
// (`encode_remaining_length`); the decoder is embedded JS in the same file, so
// it is mirrored here. The target asserts encode→decode round-trips and that
// the mirrored decoder never panics or overflows on hostile bytes.
fn decode_remaining_length(data: &[u8]) -> Option<(u64, usize)> {
    let mut multiplier: u64 = 1;
    let mut value: u64 = 0;
    let mut i = 0usize;
    loop {
        let b = *data.get(i)?;
        value = value.checked_add(((b & 0x7F) as u64).checked_mul(multiplier)?)?;
        i += 1;
        if b & 0x80 == 0 {
            return Some((value, i));
        }
        multiplier = multiplier.checked_mul(128)?;
        if i >= 4 {
            // MQTT allows at most 4 length bytes.
            return None;
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() >= 8 {
        let len = u64::from_le_bytes(data[..8].try_into().unwrap());
        if len <= u32::MAX as u64 {
            let enc = encode_remaining_length(len as usize);
            if enc.len() <= 4 {
                assert_eq!(
                    decode_remaining_length(&enc),
                    Some((len, enc.len())),
                    "MQTT remaining-length round-trip failed for {len}"
                );
            }
        }
    }

    // The mirrored decoder must not panic on arbitrary bytes.
    let _ = decode_remaining_length(data);
});
