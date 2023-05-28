#![no_main]

use std::ptr::{addr_of, addr_of_mut};

use talloc::copy_slice_bits;

use libfuzzer_sys::fuzz_target;


fuzz_target!(|data: (usize, usize, usize)| {
    let (mut from, mut to, mut size) = data;

    from %= 64;
    to %= 64;
    size %= (64 - from).min(64 - to);

    let data_src: u64 = rand::random();
    let data_dst: u64 = rand::random();

    let     src = data_src.to_le_bytes();
    let mut dst = data_dst.to_le_bytes();

    copy_slice_bits(addr_of_mut!(dst), addr_of!(src), to, from, size);

    let result = u64::from_le_bytes(dst);

    let mask = ((1 << size) - 1) << from;
    let (shifted_data, shifted_mask) = if from >= to {
        (
            data_src >> (from - to),
            mask >> (from - to),
        )
    } else {
        (
            data_src << (to - from),
            mask << (to - from),
        )
    };

    //print!("\n\n{:?}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n\n", (from, to, size), mask, shifted_mask, data_src, shifted_data, result, data_dst);

    assert!(result &  shifted_mask == shifted_data &  shifted_mask, "\n\n{:?}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n\n", (from, to, size), mask, shifted_mask, data_src, shifted_data, result, data_dst);
    assert!(result & !shifted_mask ==     data_dst & !shifted_mask, "\n\n{:?}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n{:064b}\n\n", (from, to, size), mask, shifted_mask, data_src, shifted_data, result, data_dst);
});
