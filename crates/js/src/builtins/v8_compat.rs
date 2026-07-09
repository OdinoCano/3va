use v8::{ArrayBuffer, Local, PinScope, Uint8Array};

pub fn uint8array_to_vec(_scope: &mut PinScope, arr: Local<Uint8Array>) -> Vec<u8> {
    let len = arr.byte_length();
    let mut vec = vec![0u8; len];
    arr.copy_contents(&mut vec);
    vec
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
