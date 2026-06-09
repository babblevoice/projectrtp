fn main() {
    napi_build::setup();

    // Two native libs: libilbc (iLBC codec) and libspandsp (G.722). Both
    // use the same pattern: prefer the unversioned `.so` symlink that
    // `-devel`/`-dev` packages ship, fall back to the versioned soname
    // when only the runtime package is installed (Fedora's default).
    link_native("ilbc", "libilbc.so.3");
    link_native("spandsp", "libspandsp.so.2");
}

fn link_native(name: &str, fallback_versioned_soname: &str) {
    // `/usr/local/*` covers the case where the library was built from
    // source in the Docker image (as libilbc is) and installed there.
    let search_paths = [
        "/usr/lib64",
        "/usr/lib",
        "/lib",
        "/usr/local/lib",
        "/usr/local/lib64",
    ];
    let unversioned = format!("lib{}.so", name);
    let has_unversioned = search_paths
        .iter()
        .any(|d| std::path::Path::new(d).join(&unversioned).exists());
    if has_unversioned {
        for d in &search_paths {
            if std::path::Path::new(d).join(&unversioned).exists() {
                println!("cargo:rustc-link-search=native={}", d);
            }
        }
        println!("cargo:rustc-link-lib=dylib={}", name);
    } else {
        println!("cargo:rustc-link-arg=-l:{}", fallback_versioned_soname);
    }
}
