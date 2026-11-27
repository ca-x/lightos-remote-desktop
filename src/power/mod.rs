//! Persistent host power policy. Only an explicit management request reaches these commands.
use anyhow::{Result, bail, ensure};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Read,
    Enable,
    Restore,
}

pub(crate) fn execute(op: Operation) -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        fn read() -> Result<bool> {
            let output = Command::new("/usr/bin/pmset").arg("-g").output()?;
            ensure!(output.status.success(), "pmset query failed");
            let raw = String::from_utf8(output.stdout)?;
            for line in raw.lines() {
                let mut fields = line.split_whitespace();
                if fields
                    .next()
                    .is_some_and(|k| k.eq_ignore_ascii_case("SleepDisabled"))
                {
                    return match fields.next() {
                        Some("0") => Ok(false),
                        Some("1") => Ok(true),
                        _ => anyhow::bail!("invalid pmset sleep state"),
                    };
                }
            }
            bail!("pmset did not report SleepDisabled")
        }
        let current = read()?;
        if op == Operation::Read || current == (op == Operation::Enable) {
            return Ok(current);
        }
        let value = u8::from(op == Operation::Enable);
        let script = format!(
            "do shell script \"/usr/bin/pmset -a disablesleep {value}\" with administrator privileges"
        );
        ensure!(
            Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .status()?
                .success(),
            "power change was not approved"
        );
        let actual = read()?;
        ensure!(
            actual == (op == Operation::Enable),
            "power setting was not applied"
        );
        Ok(actual)
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let action = match op {
            Operation::Read => "read",
            Operation::Enable => "enable",
            Operation::Restore => "restore",
        };
        #[cfg(target_os = "linux")]
        let output = {
            let args = ["-c", include_str!("linux.sh"), "lazycat-power", action];
            let result = Command::new("/bin/sh").args(args).output()?;
            if op != Operation::Read && result.status.code() == Some(77) {
                Command::new("/usr/bin/pkexec")
                    .arg("/bin/sh")
                    .args(args)
                    .output()?
            } else {
                result
            }
        };
        #[cfg(target_os = "windows")]
        let output = {
            use base64::{Engine, engine::general_purpose::STANDARD};
            use std::os::windows::process::CommandExt;
            let encode = |text: &str| {
                STANDARD.encode(
                    text.encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>(),
                )
            };
            let system = std::env::var_os("SystemRoot")
                .ok_or_else(|| anyhow::anyhow!("missing Windows system directory"))?;
            let executable = std::path::PathBuf::from(system)
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
            let run = |option: &str, script: &str| {
                Command::new(&executable)
                    .args(["-NoProfile", "-NonInteractive", option, script])
                    .creation_flags(0x08000000)
                    .output()
            };
            let policy = include_str!("windows.ps1");
            if op != Operation::Read {
                // Only the fixed, embedded policy is elevated. The host owns the UAC prompt.
                let elevated = encode(&policy.replace("__ACTION__", action));
                let launch = format!(
                    "$ErrorActionPreference='Stop'; $p=Start-Process -FilePath ([Environment]::SystemDirectory+'\\WindowsPowerShell\\v1.0\\powershell.exe') -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList '-NoProfile -NonInteractive -EncodedCommand {elevated}'; exit $p.ExitCode"
                );
                ensure!(
                    run("-Command", &launch)?.status.success(),
                    "Windows declined the power change"
                );
            }
            run(
                "-EncodedCommand",
                &encode(&policy.replace("__ACTION__", "read")),
            )?
        };
        ensure!(
            output.status.success(),
            "system power policy could not be read or changed"
        );
        let actual = match std::str::from_utf8(&output.stdout)?.trim() {
            "enabled" => true,
            "disabled" => false,
            _ => bail!("unknown system power state"),
        };
        ensure!(
            op == Operation::Read || actual == (op == Operation::Enable),
            "power setting was not applied"
        );
        Ok(actual)
    }
}
