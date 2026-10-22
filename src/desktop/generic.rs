use super::{DesktopInitializer, PreparedDesktop, configuration_directory};
use crate::config::Config;
use anyhow::{Context, Result};
use std::path::Path;

pub struct GenericInitializer;
impl DesktopInitializer for GenericInitializer {
    fn prepare(&self, config: &Config, runtime: &Path) -> Result<PreparedDesktop> {
        let (width, height) = config.dimensions()?;
        let terminal = ["xdg-terminal-exec", "foot", "ghostty", "kitty", "alacritty"].into_iter()
            .find_map(|name| platform_detect::find_executable(Path::new(name)))
            .context("generic desktop requires a terminal launcher: install xdg-terminal-exec, foot, ghostty, kitty or alacritty")?;
        let command = format!("'{}'", terminal.to_string_lossy().replace('\'', "'\\''"));
        let source = desktop_config::minimal(width, height, config.fps, &command);
        Ok(PreparedDesktop {
            configuration: desktop_config::write(
                &configuration_directory(runtime)?,
                "hyprland.conf",
                &source,
            )?,
            omarchy: None,
            environment: vec![],
        })
    }
}
