fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/macos_yuv.c")
            .compile("lightos_macos_yuv");
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rerun-if-changed=src/macos_yuv.c");
    }
    if std::env::var_os("CARGO_FEATURE_FAST_YUV").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
    {
        println!("cargo:rustc-link-lib=yuv");
    }
}
