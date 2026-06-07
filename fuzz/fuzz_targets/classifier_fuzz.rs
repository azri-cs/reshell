#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(stderr) = std::str::from_utf8(data) {
        let _ = reshell::classify::classify(1, stderr, "", false, "", None);
    }
});
