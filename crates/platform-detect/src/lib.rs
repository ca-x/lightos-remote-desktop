use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Distribution {
    pub id: String,
    pub id_like: Vec<String>,
    pub version: Option<String>,
}

impl Distribution {
    pub fn parse(source: &str) -> Self {
        let values: BTreeMap<_, _> = source
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some((key.trim(), value.trim().trim_matches(['\'', '"'])))
            })
            .collect();
        Self {
            id: values.get("ID").copied().unwrap_or("unknown").to_owned(),
            id_like: values
                .get("ID_LIKE")
                .copied()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            version: values.get("VERSION_ID").map(|value| (*value).to_owned()),
        }
    }

    pub fn detect() -> Self {
        std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .map(|source| Self::parse(&source))
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigFormat {
    Lua,
    Conf,
}

#[derive(Clone, Debug, Serialize)]
pub struct OmarchyInstallation {
    pub root: PathBuf,
    pub version: Option<String>,
}

impl OmarchyInstallation {
    pub fn at(root: PathBuf) -> Option<Self> {
        root.join("shell/shell.qml").is_file().then(|| {
            let version = std::fs::read_to_string(root.join("version"))
                .ok()
                .map(|v| v.trim().to_owned());
            Self { root, version }
        })
    }

    pub fn detect() -> Option<Self> {
        let root = std::env::var_os("OMARCHY_PATH")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/share/omarchy"));
        Self::at(root)
    }

    pub fn configuration(&self, user_config: &Path) -> Result<(PathBuf, ConfigFormat)> {
        let major = self.version.as_deref().and_then(|v| {
            v.trim_start_matches('v')
                .split('.')
                .next()?
                .parse::<u32>()
                .ok()
        });
        let formats = if major.is_none_or(|v| v >= 4) {
            [ConfigFormat::Lua, ConfigFormat::Conf]
        } else {
            [ConfigFormat::Conf, ConfigFormat::Lua]
        };
        for format in formats {
            let name = match format {
                ConfigFormat::Lua => "hyprland.lua",
                ConfigFormat::Conf => "hyprland.conf",
            };
            for path in [
                user_config.join("hypr").join(name),
                self.root.join("config/hypr").join(name),
            ] {
                if path.is_file() {
                    return Ok((path, format));
                }
            }
        }
        anyhow::bail!("Omarchy is installed but no hyprland.lua or hyprland.conf entry exists")
    }
}

/// Resolve a regular executable; merely finding a path is insufficient.
pub fn find_executable(name: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if name.components().count() > 1 {
        return executable(name).then(|| name.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| executable(candidate))
    })
}

/// Validate syntax using the actual compositor, bounded in time, without starting a desktop.
pub fn verify_config(
    program: &Path,
    config: &Path,
    runtime: &Path,
    environment: &[(String, String)],
) -> Result<()> {
    let program = find_executable(program).with_context(|| {
        format!(
            "missing executable for configuration probe: {}",
            program.display()
        )
    })?;
    let timeout = find_executable(Path::new("timeout"))
        .context("missing executable for bounded configuration probe: timeout")?;
    let output = Command::new(timeout)
        .args(["--kill-after=1", "10"])
        .arg(program)
        .arg("--verify-config")
        .arg("--config")
        .arg(config)
        .env("XDG_RUNTIME_DIR", runtime)
        .envs(environment.iter().cloned())
        .stdin(Stdio::null())
        .output()
        .context("run compositor configuration verifier")?;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        output.status.success() && diagnostics.contains("config ok"),
        "Hyprland rejected {}: {}",
        config.display(),
        diagnostics
            .chars()
            .rev()
            .take(4096)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
    Ok(())
}

pub fn verify_lua_support(program: &Path) -> Result<()> {
    let directory = tempfile::Builder::new().prefix("lrd-probe.").tempdir()?;
    let path = directory.path().join("probe.lua");
    std::fs::write(&path, "hl.config({})\n")?;
    verify_config(program, &path, directory.path(), &[])
        .context("Lua capability probe failed; do not fall back to conf while a Lua entry exists")
}

pub fn user_config_home() -> Result<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".config")))
        .context("HOME or XDG_CONFIG_HOME is required")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arch_family_is_not_a_desktop_profile() {
        for id in ["arch", "endeavouros", "manjaro", "omarchy"] {
            let distro = Distribution::parse(&format!(
                "ID={id}\nID_LIKE=\"arch linux\"\nVERSION_ID='4.0.6'\n"
            ));
            assert_eq!(distro.id, id);
            assert_eq!(distro.id_like, ["arch", "linux"]);
            assert_eq!(distro.version.as_deref(), Some("4.0.6"));
        }
    }
    #[test]
    fn omarchy_four_prefers_lua_and_only_falls_back_when_missing() {
        let root = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("shell")).unwrap();
        std::fs::write(root.path().join("shell/shell.qml"), "").unwrap();
        std::fs::write(root.path().join("version"), "4.0.0.alpha").unwrap();
        std::fs::create_dir_all(user.path().join("hypr")).unwrap();
        let lua = user.path().join("hypr/hyprland.lua");
        let conf = user.path().join("hypr/hyprland.conf");
        std::fs::write(&lua, "invalid lua must not trigger conf fallback").unwrap();
        std::fs::write(&conf, "").unwrap();
        let install = OmarchyInstallation::at(root.path().to_owned()).unwrap();
        assert_eq!(
            install.configuration(user.path()).unwrap(),
            (lua.clone(), ConfigFormat::Lua)
        );
        std::fs::remove_file(lua).unwrap();
        assert_eq!(
            install.configuration(user.path()).unwrap(),
            (conf, ConfigFormat::Conf)
        );
        assert!(OmarchyInstallation::at(user.path().to_owned()).is_none());
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn executable_probe_rejects_missing_non_executable_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool");
        assert!(find_executable(&file).is_none());
        std::fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(find_executable(&file).is_none());
        assert!(find_executable(dir.path()).is_none());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(find_executable(&file), Some(file));
        let error = verify_config(
            &dir.path().join("missing"),
            &dir.path().join("config.lua"),
            dir.path(),
            &[],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing executable for configuration probe")
        );
    }
}
