#![no_main]
include!("../src/shim.rs");
libfuzzer_sys::fuzz_target!(|input: &[u8]| gripsack_fuzz::dispatch("manifest", input));
