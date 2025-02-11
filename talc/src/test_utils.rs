// TODO: single bin strat
// TODO: two bin strat

macro_rules! for_many_talc_configurations {
    ($test_fn:ident) => {
        $test_fn::<crate::DefaultBinning>();
        $test_fn::<crate::wasm::WasmBinning>();
    };
}
