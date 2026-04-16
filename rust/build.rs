fn main() {
    napi_build::setup();

    // iLBC: prefer the unversioned `libilbc.so` (present when `-devel` or
    // `-dev` packages are installed, or when the bundled libilbc was built
    // from source in the Docker image — which installs to /usr/local/lib).
    // Fall back to `libilbc.so.3` — Fedora's `ilbc` package ships only the
    // versioned sonames, which rust-lld cannot resolve via plain `-lilbc`.
    // The `-l:filename` linker syntax takes an exact library file name
    // rather than the `libNAME.so` search.
    let search_paths = [
        "/usr/lib64", "/usr/lib", "/lib", "/usr/local/lib", "/usr/local/lib64",
    ];
    let ilbc_so = search_paths
        .iter()
        .any(|d| std::path::Path::new(d).join("libilbc.so").exists());
    if ilbc_so {
        // Tell the linker about /usr/local/lib (not on its default path).
        for d in &search_paths {
            if std::path::Path::new(d).join("libilbc.so").exists() {
                println!("cargo:rustc-link-search=native={}", d);
            }
        }
        println!("cargo:rustc-link-lib=dylib=ilbc");
    } else {
        println!("cargo:rustc-link-arg=-l:libilbc.so.3");
    }
}
