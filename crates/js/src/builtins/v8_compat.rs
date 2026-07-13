use v8::{Array, ArrayBuffer, Local, PinScope, Uint8Array, Value};

pub fn uint8array_to_vec(_scope: &mut PinScope, arr: Local<Uint8Array>) -> Vec<u8> {
    let len = arr.byte_length();
    let mut vec = vec![0u8; len];
    arr.copy_contents(&mut vec);
    vec
}

/// Accepts either a `Uint8Array` or a plain JS `Array` of numbers (e.g. from
/// `Array.from(bytes)`) and returns the bytes. Native functions called with
/// `Array.from(...)` args must use this instead of a bare `Uint8Array`
/// `try_into` — that silently yields an empty Vec for plain arrays.
pub fn js_value_to_bytes(scope: &mut PinScope, val: Local<Value>) -> Vec<u8> {
    if let Ok(arr) = Local::<Uint8Array>::try_from(val) {
        return uint8array_to_vec(scope, arr);
    }
    if let Ok(arr) = Local::<Array>::try_from(val) {
        return (0..arr.length())
            .map(|i| {
                arr.get_index(scope, i)
                    .and_then(|v| v.uint32_value(scope))
                    .unwrap_or(0) as u8
            })
            .collect();
    }
    Vec::new()
}

pub fn uint8array_copy_into(
    _scope: &mut PinScope,
    arr: Local<Uint8Array>,
    dest: &mut [u8],
) -> usize {
    arr.copy_contents(dest)
}

pub fn uint8array_from_bytes<'s>(
    scope: &mut PinScope<'s, '_>,
    bytes: &[u8],
) -> Local<'s, Uint8Array> {
    let backing_store = ArrayBuffer::new_backing_store_from_vec(bytes.to_vec()).make_shared();
    let buffer = ArrayBuffer::with_backing_store(scope, &backing_store);
    Uint8Array::new(scope, buffer, 0, bytes.len()).unwrap()
}
