use crate::SessionTarget;
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    time::{Duration, timeout},
};

pub(crate) fn executable(name: &str) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|p| p.join(name))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .with_context(|| format!("missing executable: {name}"))
}

pub(crate) async fn run(program: &Path, args: &[String], target: &SessionTarget) -> Result<String> {
    timeout(Duration::from_secs(3), async {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("XAUTHORITY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env("XDG_RUNTIME_DIR", &target.runtime)
            .env("LC_ALL", "C");
        if let Some(socket) = &target.wayland_socket {
            command.env("WAYLAND_DISPLAY", socket);
        }
        if let Some(bus) = &target.bus_address {
            command.env("DBUS_SESSION_BUS_ADDRESS", bus);
        }
        if let Some(display) = &target.x11_display {
            command.env("DISPLAY", display);
        }
        if let Some(auth) = &target.xauthority {
            command.env("XAUTHORITY", auth);
        }
        let mut child = command.spawn().context("start display helper")?;
        let out = child.stdout.take().context("helper stdout")?;
        let err = child.stderr.take().context("helper stderr")?;
        let read = |pipe: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
            let mut bytes = Vec::new();
            pipe.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
            ensure!(
                bytes.len() <= 1024 * 1024,
                "display helper response too large"
            );
            Ok::<_, anyhow::Error>(bytes)
        };
        let (stdout, stderr, status) =
            tokio::try_join!(read(Box::new(out)), read(Box::new(err)), async {
                Ok::<_, anyhow::Error>(child.wait().await?)
            })?;
        ensure!(
            status.success(),
            "display helper rejected query/change: {}",
            String::from_utf8_lossy(&stderr)
        );
        String::from_utf8(stdout).context("non-UTF8 display helper response")
    })
    .await
    .context("display helper timed out")?
}

pub(crate) fn blocking<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn helper_receives_only_selected_session_coordinates() {
        let target = SessionTarget {
            runtime: "/private/runtime".into(),
            wayland_socket: Some("/private/runtime/wayland".into()),
            bus_address: Some("unix:path=/private/runtime/bus".into()),
            ..Default::default()
        };
        let output=run(Path::new("/bin/sh"),&["-c".into(),"printf '%s|%s|%s|%s|%s' \"$XDG_RUNTIME_DIR\" \"$WAYLAND_DISPLAY\" \"$DBUS_SESSION_BUS_ADDRESS\" \"${DISPLAY-unset}\" \"${XAUTHORITY-unset}\"".into()],&target).await.unwrap();
        assert_eq!(
            output,
            "/private/runtime|/private/runtime/wayland|unix:path=/private/runtime/bus|unset|unset"
        );
    }
    #[tokio::test]
    async fn missing_rejected_and_hung_helpers_return_errors() {
        let target = SessionTarget::default();
        assert!(
            run(Path::new("/does/not/exist"), &[], &target)
                .await
                .is_err()
        );
        assert!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "exit 1".into()],
                &target
            )
            .await
            .is_err()
        );
        let started = std::time::Instant::now();
        assert!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "exec sleep 30".into()],
                &target
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
