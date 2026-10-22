use super::{DesktopInitializer, PreparedDesktop, configuration_directory};
use crate::config::Config;
use anyhow::{Context, Result};
use platform_detect::{ConfigFormat, OmarchyInstallation, user_config_home};
use std::{collections::BTreeMap, path::Path};

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
        let state = runtime.join("state");
        std::fs::create_dir_all(&state)?;
        let environment = vec![
            (
                "OMARCHY_PATH".into(),
                self.installation.root.to_string_lossy().into_owned(),
            ),
            (
                "XDG_STATE_HOME".into(),
                state.to_string_lossy().into_owned(),
            ),
            ("PATH".into(), path.clone()),
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
