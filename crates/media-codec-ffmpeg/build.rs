fn main() {
    if std::env::var_os("CARGO_FEATURE_FAST_YUV").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
    {
        println!("cargo:rustc-link-lib=yuv");
    }
}
