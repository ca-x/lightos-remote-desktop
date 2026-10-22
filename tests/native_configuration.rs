use clap::Parser;
use lightos_remote_desktop::{config::Config, desktop};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
#[ignore = "explicitly validates the current user's installed Omarchy configuration"]
fn installed_omarchy_uses_native_lua_with_only_remote_overrides() {
    let runtime = tempfile::Builder::new()
        .prefix("lrd.cfg.")
        .tempdir()
        .unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let config = Config::try_parse_from([
        "test",
        "--password",
        "native-config-test-secret",
        "--desktop-shell",
        "omarchy",
    ])
    .unwrap();
    let result = desktop::prepare(&config, runtime.path()).unwrap();
    assert_eq!(result.configuration.extension().unwrap(), "lua");
    let generated = fs::read_to_string(result.configuration).unwrap();
    assert!(generated.contains("dofile("));
    assert!(!generated.contains("o.bind("));
    println!(
        "native Lua entry verified; generated configuration uses private runtime {}",
        runtime.path().display()
    );
}
