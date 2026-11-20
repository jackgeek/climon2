fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed=lpa.manifest");
    println!("cargo:rustc-link-arg-bin=probe-lpa=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bin=probe-lpa=/MANIFESTINPUT:{dir}/lpa.manifest");
}
