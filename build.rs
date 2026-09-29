fn main() {
    // Windows (MSVC): link the icon into nanomd.exe. link.exe takes the
    // prebuilt .res as input, so no resource compiler runs at build time.
    // After changing assets/windows/icon.ico, rebuild it from that folder:
    //   rc /nologo /fo nanomd.res nanomd.rc
    println!("cargo:rerun-if-changed=assets/windows/nanomd.res");
    let var = |k| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") == "windows" && var("CARGO_CFG_TARGET_ENV") == "msvc" {
        let res =
            std::path::Path::new(&var("CARGO_MANIFEST_DIR")).join("assets/windows/nanomd.res");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
