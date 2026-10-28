use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};

#[derive(Args)]
pub struct ServiceOptions {
    #[arg(
        long,
        help = "Show installation layout without changing files or services"
    )]
    pub dry_run: bool,
    #[arg(long, help = "Install files without activating services")]
    pub stage_only: bool,
    #[arg(long, hide = true)]
    pub test_root: Option<PathBuf>,
    #[arg(long, value_enum, conflicts_with_all = ["package_managed", "snapshot", "remove", "legacy", "dry_run", "stage_only", "test_root"]) ]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub print_unit: Option<ServiceUnit>,
    #[arg(long, conflicts_with_all = ["snapshot", "remove", "legacy", "dry_run"]) ]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub package_managed: bool,
    #[arg(long, conflicts_with_all = ["package_managed", "remove", "legacy", "dry_run", "stage_only"]) ]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub snapshot: bool,
    #[arg(long, conflicts_with_all = ["package_managed", "snapshot", "legacy", "dry_run", "stage_only"]) ]
    pub remove: bool,
    #[arg(long, help = "Use the explicit legacy LightOS transactional installer")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub legacy: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ServiceUnit {
    Transport,
    Session,
}

#[cfg(target_os = "linux")]
mod linux;

pub trait ServiceInstaller {
    fn install(&self, options: ServiceOptions, rollback: bool) -> Result<()>;
}
struct NativeInstaller;
impl ServiceInstaller for NativeInstaller {
    fn install(&self, options: ServiceOptions, rollback: bool) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            linux::service(options, rollback)
        }
        #[cfg(target_os = "windows")]
        {
            crate::platform::windows::install_service(options, rollback)
        }
        #[cfg(target_os = "macos")]
        {
            crate::platform::macos::install_service(options, rollback)
        }
    }
}
pub fn service(options: ServiceOptions, rollback: bool) -> Result<()> {
    NativeInstaller.install(options, rollback)
}

/// Parsed only inside the host. Credentials never become process arguments.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSettings {
    pub arguments: Vec<String>,
}
impl HostSettings {
    pub fn from_environment() -> Result<Self> {
        use clap::{CommandFactory, Parser};
        let config = crate::config::Config::try_parse_from(["lightos-remoted"])?;
        config.validate()?;
        let command = crate::config::Config::command();
        let mut arguments = Vec::new();
        for argument in command.get_arguments() {
            let (Some(name), Some(env)) = (argument.get_long(), argument.get_env()) else {
                continue;
            };
            if let Some(value) = std::env::var_os(env) {
                let value = value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("service configuration must be valid Unicode"))?;
                if argument.get_action().takes_values() {
                    arguments.push(format!("--{name}"));
                    arguments.push(value);
                } else if value == "true" || value == "1" {
                    arguments.push(format!("--{name}"));
                }
            }
        }
        Ok(Self { arguments })
    }
    pub fn config(&self) -> Result<crate::config::Config> {
        use clap::{CommandFactory, FromArgMatches};
        let matches = crate::config::Config::command()
            .mut_args(|argument| argument.env(None::<&str>))
            .try_get_matches_from(
                std::iter::once("lightos-remoted").chain(self.arguments.iter().map(String::as_str)),
            )?;
        let config = crate::config::Config::from_arg_matches(&matches)?;
        config.validate()?;
        Ok(config)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = crate::private_storage::current()
            .read(path, 64 * 1024)?
            .context("service configuration missing")?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.config()?;
        crate::private_storage::current().write(path, &serde_json::to_vec(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_config_validates_credentials_and_rejects_unknown_options() {
        let mut settings = HostSettings {
            arguments: vec![
                "--password".into(),
                "service-test-password".into(),
                "--port".into(),
                "18766".into(),
            ],
        };
        let config = settings.config().unwrap();
        assert_eq!(config.port, 18766);
        assert_eq!(config.password, "service-test-password");
        settings.arguments.push("--unknown-option".into());
        assert!(settings.config().is_err());
        assert!(HostSettings { arguments: vec![] }.config().is_err());
    }
    #[test]
    fn installed_credentials_use_private_storage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private/host.json");
        let settings = HostSettings {
            arguments: vec!["--password".into(), "service-test-password".into()],
        };
        settings.save(&path).unwrap();
        assert_eq!(
            HostSettings::load(&path)
                .unwrap()
                .config()
                .unwrap()
                .password,
            "service-test-password"
        );
    }
}
