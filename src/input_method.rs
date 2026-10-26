//! Fcitx in an owned desktop: private D-Bus activation and writable state, seeded
//! from the user's installed configuration and static dictionaries.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
    process::Command,
};

fn quote(path: &Path) -> Result<String> {
    let value = path.to_str().context("input-method path is not UTF-8")?;
    ensure!(
        !value.contains(['\n', '\r', '\0']),
        "invalid input-method path"
    );
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

fn writable_state(name: &str) -> bool {
    matches!(name, ".git" | "sync" | "LOCK" | "LOG" | "installation.yaml")
        || [".userdb", ".db", ".sqlite", ".sqlite3", ".lock"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn copy_tree(source: &Path, target: &Path, depth: usize, bytes: &mut u64) -> Result<()> {
    ensure!(
        depth < 20,
        "input-method configuration has too many nested directories"
    );
    fs::create_dir_all(target)?;
    fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if writable_state(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let metadata = fs::metadata(entry.path())?;
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &destination, depth + 1, bytes)?;
        } else if metadata.is_file() {
            *bytes += metadata.len();
            ensure!(
                *bytes <= 512 * 1024 * 1024,
                "input-method seed exceeds 512 MiB"
            );
            fs::copy(entry.path(), &destination)?;
            fs::set_permissions(destination, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

fn seed(profile: &Path, config: &Path, data: &Path) -> Result<()> {
    let parent = profile
        .parent()
        .context("input-method profile has no parent")?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join("input-method.lock"))?;
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
        "lock input-method seed"
    );
    if profile.join("seeded").is_file() {
        return Ok(());
    }
    let temporary = tempfile::Builder::new()
        .prefix("input-method.")
        .tempdir_in(parent)?;
    let mut bytes = 0;
    copy_tree(
        config,
        &temporary.path().join("config/fcitx5"),
        0,
        &mut bytes,
    )?;
    if data.is_dir() {
        copy_tree(data, &temporary.path().join("data/fcitx5"), 0, &mut bytes)?;
    }
    fs::create_dir_all(temporary.path().join("data"))?;
    fs::write(temporary.path().join("seeded"), b"1\n")?;
    fs::rename(temporary.path(), profile)?;
    Ok(())
}

fn on_bus(runtime: &Path, program: &str) -> Command {
    let mut command = Command::new("timeout");
    command
        .args(["--kill-after=1", "3", program])
        .env("XDG_RUNTIME_DIR", runtime)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/bus", runtime.display()),
        );
    command
}

fn launcher_source(
    runtime: &Path,
    socket: &Path,
    profile: &Path,
    program: &Path,
) -> Result<String> {
    let bus = quote(Path::new(&format!("unix:path={}/bus", runtime.display())))?;
    Ok(format!(
        "#!/bin/sh\nset -eu\n\
         case ${{DBUS_STARTER_ADDRESS:-${{DBUS_SESSION_BUS_ADDRESS:-}}}} in\n\
         {bus}|{bus},guid=*) export DBUS_SESSION_BUS_ADDRESS={bus} ;;\n\
         *) printf '%s\\n' 'LightOS: input-method activation belongs to a different session' >&2; exit 1 ;;\nesac\n\
         . {}\nexport WAYLAND_DISPLAY={}\nexport XDG_CONFIG_HOME={}\nexport XDG_DATA_HOME={}\nexec {} --disable=notificationitem\n",
        quote(&runtime.join("application-environment.sh"))?,
        quote(socket)?,
        quote(&profile.join("config"))?,
        quote(&profile.join("data"))?,
        quote(program)?
    ))
}

/// Called only for the tool's owned virtual sessions, including hot attachment.
/// The private bus owns the daemon so transport restarts preserve its lifetime.
pub(crate) fn start(runtime: &Path, socket: &Path) -> Result<()> {
    let Some(program) = platform_detect::find_executable(Path::new("fcitx5")) else {
        return Ok(());
    };
    let config = platform_detect::user_config_home()?.join("fcitx5");
    if !config.join("profile").is_file() {
        return Ok(());
    }
    ensure!(
        socket.parent() == Some(runtime)
            && crate::session::is_owned_socket(socket, unsafe { libc::geteuid() }),
        "input-method display is outside its owned session"
    );
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .context("HOME or XDG_DATA_HOME required for input methods")?
        .join("fcitx5");
    let profile = crate::applications::profile_root(runtime)?.join("input-method");
    seed(&profile, &config, &data)?;
    let environment = runtime.join("input-method-environment.sh");
    crate::applications::write_atomic(
        &environment,
        "export INPUT_METHOD=fcitx QT_IM_MODULE=fcitx XMODIFIERS=@im=fcitx\nexport LIGHTOS_SESSION_INPUT_METHOD=fcitx5\n",
        0o600,
    )?;
    let launcher = runtime.join("bin/session-fcitx5");
    let source = launcher_source(runtime, socket, &profile, &program)?;
    crate::applications::write_atomic(&launcher, &source, 0o700)?;
    let services = runtime.join("dbus-1/services");
    fs::create_dir_all(&services)?;
    let executable = launcher
        .to_str()
        .context("input-method executable is not UTF-8")?
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    crate::applications::write_atomic(
        &services.join("org.fcitx.Fcitx5.service"),
        &format!("[D-BUS Service]\nName=org.fcitx.Fcitx5\nExec=\"{executable}\"\n"),
        0o600,
    )?;
    let updated = on_bus(runtime, "dbus-update-activation-environment")
        .args([
            format!("WAYLAND_DISPLAY={}", socket.display()),
            format!("XDG_RUNTIME_DIR={}", runtime.display()),
            "INPUT_METHOD=fcitx".into(),
            "QT_IM_MODULE=fcitx".into(),
            "XMODIFIERS=@im=fcitx".into(),
        ])
        .output()?;
    ensure!(
        updated.status.success(),
        "update owned input-method activation environment"
    );
    let reload = on_bus(runtime, "dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--reply-timeout=1500",
            "--dest=org.freedesktop.DBus",
            "/",
            "org.freedesktop.DBus.ReloadConfig",
        ])
        .output()?;
    ensure!(
        reload.status.success(),
        "reload owned input-method activation service"
    );
    // Do not wait for dictionary deployment: capture can start immediately.
    let activate = on_bus(runtime, "dbus-send")
        .args([
            "--session",
            "--type=method_call",
            "--dest=org.fcitx.Fcitx5",
            "/controller",
            "org.fcitx.Fcitx.Controller1.State",
        ])
        .output()?;
    ensure!(activate.status.success(), "activate owned input method");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_bus_activation_accepts_its_guid_but_rejects_other_sessions() {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("fake-fcitx5");
        fs::write(
            &program,
            "#!/bin/sh\nprintf '%s' \"$DBUS_SESSION_BUS_ADDRESS\" > \"$TEST_OUTPUT\"\n",
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let bus = format!("unix:path={}/bus", root.path().display());
        fs::write(
            root.path().join("application-environment.sh"),
            format!(
                "[ \"$DBUS_SESSION_BUS_ADDRESS\" = {} ] || exit 1\n",
                quote(Path::new(&bus)).unwrap()
            ),
        )
        .unwrap();
        let launcher = root.path().join("launcher");
        fs::write(
            &launcher,
            launcher_source(
                root.path(),
                &root.path().join("wayland-1"),
                &root.path().join("profile"),
                &program,
            )
            .unwrap(),
        )
        .unwrap();
        let output = root.path().join("output");
        for address in [
            bus.clone(),
            format!("{bus},guid=0123456789abcdef"),
            "unix:path=/another-session/bus".into(),
        ] {
            let _ = fs::remove_file(&output);
            let status = Command::new("sh")
                .arg(&launcher)
                .env_remove("DBUS_SESSION_BUS_ADDRESS")
                .env("DBUS_STARTER_ADDRESS", &address)
                .env("TEST_OUTPUT", &output)
                .status()
                .unwrap();
            assert_eq!(status.success(), address.starts_with(&bus));
            if status.success() {
                assert_eq!(fs::read_to_string(&output).unwrap(), bus);
            } else {
                assert!(!output.exists());
            }
        }
    }
    #[test]
    fn seeds_configuration_and_static_dictionaries_without_sharing_live_databases() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("host-config");
        let data = root.path().join("host-data/rime");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(data.join("learned.userdb")).unwrap();
        fs::write(config.join("profile"), "DefaultIM=rime\n").unwrap();
        fs::write(data.join("schema.yaml"), "schema: test\n").unwrap();
        fs::write(data.join("learned.userdb/LOCK"), "host lock").unwrap();
        fs::write(data.join("installation.yaml"), "installation_id: host\n").unwrap();
        let private = root.path().join("private");
        seed(&private, &config, data.parent().unwrap()).unwrap();
        assert_eq!(
            fs::read(private.join("config/fcitx5/profile")).unwrap(),
            fs::read(config.join("profile")).unwrap()
        );
        assert!(private.join("data/fcitx5/rime/schema.yaml").is_file());
        assert!(!private.join("data/fcitx5/rime/learned.userdb").exists());
        assert!(!private.join("data/fcitx5/rime/installation.yaml").exists());
        fs::write(private.join("config/fcitx5/profile"), "private choice").unwrap();
        seed(&private, &config, data.parent().unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(private.join("config/fcitx5/profile")).unwrap(),
            "private choice"
        );
        assert_eq!(
            fs::read_to_string(config.join("profile")).unwrap(),
            "DefaultIM=rime\n"
        );
    }
}
