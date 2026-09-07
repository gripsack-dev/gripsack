#![no_main]
libfuzzer_sys::fuzz_target!(|input: &[u8]| gripsack_fuzz::dispatch("journal", input));
