fn main() {
    // let size_to_bin = |size: usize| unsafe { WasmBinning::size_to_bin(size) };
    // let size_to_bin = |size: usize| B::size_to_bin(size);
    let size_to_bin = |size: usize| {
        talc::base::binning::linear_extent_then_linearly_divided_exponential_binning::<4, 4>(size)
    };

    talc::base::binning::test_utils::find_binning_boundaries(
        0,
        None,
        &size_to_bin,
        &mut |bin, size| println!("{:>4}: {1:>16}  {1:>12X}", bin, size),
    );
}
