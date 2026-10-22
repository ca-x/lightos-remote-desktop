//! Desktop-specific preparation. Process ownership belongs to Session.
mod generic;
mod omarchy;

use crate::config::{Config, DesktopShell};
use anyhow::Result;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct PreparedDesktop {
    pub configuration: PathBuf,
    pub omarchy: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
}

pub trait DesktopInitializer {
    fn prepare(&self, config: &Config, runtime: &Path) -> Result<PreparedDesktop>;
}

pub fn prepare(config: &Config, runtime: &Path) -> Result<PreparedDesktop> {
    if let Some(path) = &config.desktop_config {
        anyhow::ensure!(path.is_file(), "desktop configuration is missing");
        return Ok(PreparedDesktop {
            configuration: std::fs::canonicalize(path)?,
            omarchy: None,
            environment: vec![],
        });
    }
    let initializer: Box<dyn DesktopInitializer> = match config.desktop_shell {
        DesktopShell::Minimal => Box::new(generic::GenericInitializer),
        _ => match platform_detect::OmarchyInstallation::detect() {
            Some(installation) => Box::new(omarchy::OmarchyInitializer { installation }),
            None => {
                anyhow::ensure!(
                    config.desktop_shell != DesktopShell::Omarchy,
                    "Omarchy shell is not installed"
                );
                Box::new(generic::GenericInitializer)
            }
        },
    };
    initializer.prepare(config, runtime)
}

fn configuration_directory(runtime: &Path) -> Result<PathBuf> {
    Ok(tempfile::Builder::new()
        .prefix("config.")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(runtime)?
        .keep())
}
