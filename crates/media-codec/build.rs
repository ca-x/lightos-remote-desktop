fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/videotoolbox.m")
            .flag("-fobjc-arc")
            .compile("lightos_videotoolbox");
        for framework in [
            "Foundation",
            "VideoToolbox",
            "CoreVideo",
            "CoreMedia",
            "CoreGraphics",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rerun-if-changed=src/videotoolbox.m");
    }
}
