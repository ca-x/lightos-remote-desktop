//! Desktop-specific preparation. Process ownership belongs to Session.
mod generic;
mod omarchy;

use crate::config::{Compositor, Config, DesktopShell};
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
    config.validate_compositor()?;
    if config.compositor == Compositor::Niri {
        return Ok(PreparedDesktop {
            configuration: config.niri_config()?,
            omarchy: None,
            environment: vec![],
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn native_niri_preserves_the_supplied_path_and_never_generates_a_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.kdl");
        let linked = directory.path().join("config.kdl");
        let runtime = directory.path().join("runtime");
        std::fs::write(&original, "include \"personal.kdl\"\n").unwrap();
        std::os::unix::fs::symlink(&original, &linked).unwrap();
        std::fs::create_dir(&runtime).unwrap();
        let config = Config::try_parse_from([
            "test",
            "--compositor",
            "niri",
            "--desktop-config",
            linked.to_str().unwrap(),
        ])
        .unwrap();
        let prepared = prepare(&config, &runtime).unwrap();
        assert_eq!(prepared.configuration, linked);
        assert!(prepared.omarchy.is_none());
        assert!(prepared.environment.is_empty());
        assert!(std::fs::read_dir(&runtime).unwrap().next().is_none());
        assert_eq!(
            std::fs::read_to_string(&original).unwrap(),
            "include \"personal.kdl\"\n"
        );
    }
}
