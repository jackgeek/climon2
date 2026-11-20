// S6: imports a function from a DLL that does not exist.
#[link(name = "probe_missing_dll", kind = "raw-dylib")]
extern "system" {
    fn probe_missing_fn() -> u32;
}
fn main() {
    let v = unsafe { probe_missing_fn() };
    std::process::exit(v as i32);
}
