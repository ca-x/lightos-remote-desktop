use super::{DesktopInitializer, PreparedDesktop, configuration_directory};
use crate::config::Config;
use anyhow::{Context, Result};
use platform_detect::{ConfigFormat, OmarchyInstallation, user_config_home};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct OmarchyInitializer {
    pub installation: OmarchyInstallation,
}
impl DesktopInitializer for OmarchyInitializer {
    fn prepare(&self, config: &Config, runtime: &Path) -> Result<PreparedDesktop> {
        let user = user_config_home()?;
        let (entry, format) = self.installation.configuration(&user)?;
        let (width, height) = config.dimensions()?;
        let mut paths = vec![runtime.join("bin"), self.installation.root.join("bin")];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        let path = std::env::join_paths(paths)?
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 session PATH"))?;
        prepare_background(&self.installation.root)?;
        let environment = vec![
            (
                "OMARCHY_PATH".into(),
                self.installation.root.to_string_lossy().into_owned(),
            ),
            ("PATH".into(), path.clone()),
            (
                "CHROME_CONFIG_HOME".into(),
                crate::applications::profile_root(runtime)?
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "DBUS_SESSION_BUS_ADDRESS".into(),
                format!("unix:path={}/bus", runtime.display()),
            ),
            (
                "PULSE_SERVER".into(),
                format!("unix:{}/pulse/native", runtime.display()),
            ),
            (
                "PIPEWIRE_RUNTIME_DIR".into(),
                runtime.to_string_lossy().into_owned(),
            ),
        ];
        let (name, source) = match format {
            ConfigFormat::Lua => {
                platform_detect::verify_lua_support(Path::new("Hyprland"))?;
                (
                    "hyprland.lua",
                    desktop_config::omarchy_lua(&entry, width, height, config.fps, &path),
                )
            }
            ConfigFormat::Conf => {
                let mut variables: BTreeMap<_, _> = std::env::vars().collect();
                variables.extend(environment.iter().cloned());
                variables.insert(
                    "XDG_CONFIG_HOME".into(),
                    user.to_string_lossy().into_owned(),
                );
                let mut source = desktop_config::native_conf(&entry, &variables)?;
                source.push_str(&format!(
                    "\nmonitor = ,{width}x{height}@{},auto,1\nenv = GDK_SCALE,1\n",
                    config.fps
                ));
                for (key, value) in &environment {
                    source.push_str(&format!("env = {key},{value}\n"));
                }
                ("hyprland.conf", source)
            }
        };
        let configuration =
            desktop_config::write(&configuration_directory(runtime)?, name, &source)?;
        platform_detect::verify_config(
            Path::new("Hyprland"),
            &configuration,
            runtime,
            &environment,
        )
        .with_context(|| {
            format!(
                "native Omarchy configuration {} is incompatible",
                entry.display()
            )
        })?;
        tracing::info!(entry = %entry.display(), ?format, "loaded native Omarchy configuration with remote overrides");
        Ok(PreparedDesktop {
            configuration,
            omarchy: Some(self.installation.root.clone()),
            environment,
        })
    }
}

// Omarchy's background command and shell both use this persistent user path.
// The command assumes its parent already exists: otherwise IPC changes only
// the live shell and the choice disappears at the next session startup.
fn prepare_background(installation: &Path) -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME required for Omarchy state")?);
    let current = home.join(".local/state/omarchy/current");
    fs::create_dir_all(&current).context("prepare persistent Omarchy background state")?;
    let background = current.join("background");
    match fs::symlink_metadata(&background) {
        Ok(_) => return Ok(()), // Preserve even an unavailable user selection.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let theme = if current.join("theme/backgrounds").is_dir() {
        current.join("theme/backgrounds")
    } else {
        installation.join("themes/tokyo-night/backgrounds")
    };
    let Ok(entries) = fs::read_dir(theme) else {
        return Ok(());
    };
    let mut images = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|extension| {
                    matches!(
                        extension.to_string_lossy().to_ascii_lowercase().as_str(),
                        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp"
                    )
                })
        })
        .collect::<Vec<_>>();
    images.sort();
    if let Some(image) = images.first() {
        let image = fs::canonicalize(image)?;
        match std::os::unix::fs::symlink(image, background) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
