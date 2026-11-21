//! A minimal Rust cdylib: loaded by a probe process (probe 4) and inspected
//! with dumpbin (probe 7). It uses std formatting and allocation so its CRT
//! imports resemble a real library's.

#[no_mangle]
pub extern "C" fn probe_len(n: u32) -> usize {
    let s = format!("probe-{n}-{:?}", std::time::SystemTime::now());
    s.len()
}
