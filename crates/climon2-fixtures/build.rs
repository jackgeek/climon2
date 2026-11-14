//! The library host (`climon2-library-host`) counts the client library's heap
//! on Linux by defining glibc's allocation entry points itself
//! (`src/bin/library/heap.rs`). Exporting them dynamically is what makes the
//! loader bind the loaded library to them.

const ALLOCATION_ENTRIES: [&str; 5] = ["malloc", "calloc", "realloc", "free", "posix_memalign"];

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        for entry in ALLOCATION_ENTRIES {
            println!(
                "cargo:rustc-link-arg-bin=climon2-library-host=-Wl,--export-dynamic-symbol={entry}"
            );
        }
    }
}
