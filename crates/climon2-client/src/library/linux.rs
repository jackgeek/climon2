//! The Linux client library links the GCC unwinder statically. Unwinding never
//! leaves the library, because no unwinding crosses its C ABI, so this private
//! copy lets `libclimon2_client.so` need no shared library beyond glibc's,
//! where Rust's standard library otherwise needs the shared GCC runtime
//! `libgcc_s.so.1` (library embedding).
//!
//! This is the library target's own native library, so the linker meets it
//! before the standard library's `-lgcc_s` and never needs the shared runtime.
#[link(name = "gcc_eh", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {}
