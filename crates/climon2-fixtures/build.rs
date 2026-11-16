//! The library host (`climon2-library-host`) counts the client library's heap
//! on x86-64 GNU/Linux by defining glibc's allocation entry points itself
//! (`src/bin/library/heap.rs`). Exporting them dynamically is what makes the
//! loader bind the loaded library to them.

const ALLOCATION_ENTRIES: [&str; 5] = ["malloc", "calloc", "realloc", "free", "posix_memalign"];

fn main() {
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_ARCH") == "x86_64"
        && target("CARGO_CFG_TARGET_OS") == "linux"
        && target("CARGO_CFG_TARGET_ENV") == "gnu"
    {
        for entry in ALLOCATION_ENTRIES {
            println!(
                "cargo:rustc-link-arg-bin=climon2-library-host=-Wl,--export-dynamic-symbol={entry}"
            );
        }
    }
}
