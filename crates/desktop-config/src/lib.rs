use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// A Lua string literal, including non-ASCII paths and control characters.
fn lua_string(value: &str) -> String {
    let mut result = String::from("\"");
    for byte in value.bytes() {
        match byte {
            b'"' => result.push_str("\\\""),
            b'\\' => result.push_str("\\\\"),
            32..=126 => result.push(byte as char),
            _ => result.push_str(&format!("\\{byte:03}")),
        }
    }
    result.push('"');
    result
}

pub fn exec_dispatch(command: &str, lua: bool) -> String {
    if lua {
        format!("dispatch hl.dsp.exec_cmd({})", lua_string(command))
    } else {
        format!("dispatch exec {command}")
    }
}

pub fn omarchy_lua(entry: &Path, width: u32, height: u32, fps: u32, path: &str) -> String {
    format!(
        r#"-- Generated remote entry. Native Omarchy defaults and user overrides remain authoritative.
-- Bootstrap clears package.loaded during reload; preload survives it.
for _, name in ipairs({{ "default.hypr.autostart", "hypr.autostart", "hypr.monitors", "default.hypr.nvidia" }}) do
  package.preload[name] = function() return {{}} end
end
dofile({entry})
-- Remote-session overrides are applied after all native configuration.
hl.monitor({{ output = "", mode = "{width}x{height}@{fps}", position = "auto", scale = 1 }})
hl.env("GDK_SCALE", "1")
hl.env("PATH", {path})
hl.env("XDG_RUNTIME_DIR", os.getenv("XDG_RUNTIME_DIR"))
hl.env("DBUS_SESSION_BUS_ADDRESS", os.getenv("DBUS_SESSION_BUS_ADDRESS"))
local chrome_config = os.getenv("CHROME_CONFIG_HOME")
if chrome_config then hl.env("CHROME_CONFIG_HOME", chrome_config) end
for _, name in ipairs({{ "XDG_STATE_HOME", "PULSE_SERVER", "PULSE_SINK", "PIPEWIRE_RUNTIME_DIR" }}) do
  local value = os.getenv(name)
  if value then hl.env(name, value) end
end
"#,
        entry = lua_string(&entry.to_string_lossy()),
        path = lua_string(path)
    )
}

pub fn minimal(width: u32, height: u32, fps: u32, terminal: &str) -> String {
    include_str!("../templates/hyprland.conf")
        .replace("$terminal = foot", &format!("$terminal = {terminal}"))
        .replace("1280x720@30", &format!("{width}x{height}@{fps}"))
}

/// Resolve native conf includes into a private file so host autostart and monitor
/// declarations are excluded without modifying or maintaining a copy of defaults.
pub fn native_conf(entry: &Path, variables: &BTreeMap<String, String>) -> Result<String> {
    let mut result = String::new();
    let mut variables = variables.clone();
    load_conf(entry, &mut variables, &mut Vec::new(), &mut result)?;
    Ok(result)
}

fn expand(value: &str, variables: &BTreeMap<String, String>) -> Result<String> {
    let mut value = value.trim().trim_matches(['\'', '"']).to_owned();
    if value.starts_with("~/") {
        value = format!(
            "{}/{}",
            variables
                .get("HOME")
                .context("HOME required in config source")?,
            &value[2..]
        );
    }
    for _ in 0..32 {
        let Some(start) = value.find('$') else {
            return Ok(value);
        };
        let rest = &value[start + 1..];
        let (key, length) = if let Some(rest) = rest.strip_prefix('{') {
            let end = rest
                .find('}')
                .context("unclosed variable in config source")?;
            (&rest[..end], end + 2)
        } else {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            (&rest[..end], end)
        };
        let replacement = variables
            .get(key)
            .with_context(|| format!("unknown config source variable ${key}"))?;
        value.replace_range(start..start + 1 + length, replacement);
    }
    anyhow::bail!("recursive variables in config source")
}

fn load_conf(
    entry: &Path,
    variables: &mut BTreeMap<String, String>,
    stack: &mut Vec<PathBuf>,
    output: &mut String,
) -> Result<()> {
    ensure!(
        stack.len() < 32 && output.len() < 4 * 1024 * 1024,
        "native config include limit exceeded"
    );
    let entry = fs::canonicalize(entry)
        .with_context(|| format!("read native config {}", entry.display()))?;
    ensure!(
        !stack.contains(&entry),
        "recursive config include: {}",
        entry.display()
    );
    ensure!(
        fs::metadata(&entry)?.len() <= 1024 * 1024,
        "native config exceeds 1 MiB"
    );
    stack.push(entry.clone());
    let mut skipped_continuation = false;
    for line in fs::read_to_string(&entry)?.lines() {
        if skipped_continuation {
            skipped_continuation = line.trim_end().ends_with('\\');
            continue;
        }
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            if key.starts_with('$') {
                variables.insert(
                    key.trim_start_matches('$').to_owned(),
                    expand(value, variables)?,
                );
            }
            if matches!(key, "exec" | "exec-once" | "exec-shutdown" | "monitor") {
                skipped_continuation = line.trim_end().ends_with('\\');
                continue;
            }
            if key == "source" {
                let source = expand(value.split('#').next().unwrap_or_default(), variables)?;
                let source = PathBuf::from(source);
                let source = if source.is_absolute() {
                    source
                } else {
                    entry.parent().unwrap().join(source)
                };
                let pattern = source.to_str().context("non-UTF8 config include")?;
                let mut matched = false;
                for (index, path) in glob::glob(pattern)?.enumerate() {
                    ensure!(index < 1024, "too many config includes");
                    let path = path?;
                    matched = true;
                    if !matches!(
                        path.file_stem().and_then(|s| s.to_str()),
                        Some("autostart" | "monitors")
                    ) {
                        load_conf(&path, variables, stack, output)?;
                    }
                }
                ensure!(matched, "native config include does not exist: {pattern}");
                continue;
            }
        }
        output.push_str(line);
        output.push('\n');
    }
    stack.pop();
    Ok(())
}

/// Write only inside an already-created private runtime, refusing symlink targets.
pub fn write(runtime: &Path, name: &str, source: &str) -> Result<PathBuf> {
    use std::{
        io::Write,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    ensure!(
        matches!(name, "hyprland.conf" | "hyprland.lua"),
        "invalid generated config name"
    );
    let metadata = fs::symlink_metadata(runtime)?;
    ensure!(
        metadata.is_dir() && metadata.mode() & 0o077 == 0,
        "config runtime must be a private directory"
    );
    let path = runtime.join(name);
    // O_NOFOLLOW, as on the other runtime writes, prevents following a user-supplied link.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("create fresh private config {}", path.display()))?;
    file.write_all(source.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lua_loads_native_entry_and_overrides_only_session_boundaries() {
        let source = omarchy_lua(
            std::path::Path::new("/home/test/hyprland.lua"),
            1600,
            900,
            30,
            "/private/bin:/usr/bin",
        );
        assert!(source.contains("/home/test/hyprland.lua"));
        assert!(source.contains("package.preload"));
        assert!(source.contains("hypr.autostart"));
        assert!(source.contains("hypr.monitors"));
        assert!(source.contains("1600x900@30"));
        assert!(!source.contains("o.bind("));
        assert!(!source.contains("default.hypr.omarchy\")"));
    }
    #[test]
    fn conf_loads_native_includes_without_host_autostart_or_monitors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("bindings.conf"),
            "bind = SUPER, T, togglefloating,\nexec-once = host-pollution\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("monitors.conf"),
            "monitor = HDMI-A-1,preferred,auto,2\n",
        )
        .unwrap();
        let entry = dir.path().join("hyprland.conf");
        std::fs::write(
            &entry,
            "$root = .\nsource = $root/bindings.conf\nsource = $root/monitors.conf\n",
        )
        .unwrap();
        let source = native_conf(&entry, &Default::default()).unwrap();
        assert!(source.contains("SUPER, T"));
        assert!(!source.contains("host-pollution"));
        assert!(!source.contains("HDMI-A-1"));
    }
}

#[cfg(all(test, unix))]
mod boundary_tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn native_conf_handles_globs_and_rejects_cycles() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.conf"), "source = b.conf\n").unwrap();
        fs::write(d.path().join("b.conf"), "source = a.conf\n").unwrap();
        assert!(native_conf(&d.path().join("a.conf"), &BTreeMap::new()).is_err());
        fs::create_dir(d.path().join("parts")).unwrap();
        fs::write(
            d.path().join("parts/one.conf"),
            "bind = SUPER, X, killactive,\n",
        )
        .unwrap();
        fs::write(
            d.path().join("parts/two.conf"),
            "bind = SUPER, Y, togglefloating,\n",
        )
        .unwrap();
        fs::write(d.path().join("a.conf"), "source = parts/*.conf\n").unwrap();
        let text = native_conf(&d.path().join("a.conf"), &BTreeMap::new()).unwrap();
        assert!(text.contains("SUPER, X") && text.contains("SUPER, Y"));
    }
    #[test]
    fn writer_refuses_symlinks_and_existing_configuration() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let host = d.path().join("host");
        fs::write(&host, "preserve").unwrap();
        symlink(&host, d.path().join("hyprland.lua")).unwrap();
        assert!(write(d.path(), "hyprland.lua", "overwrite").is_err());
        assert_eq!(fs::read_to_string(&host).unwrap(), "preserve");
        let path = write(d.path(), "hyprland.conf", "new").unwrap();
        assert!(write(d.path(), "hyprland.conf", "overwrite").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
    }
}
