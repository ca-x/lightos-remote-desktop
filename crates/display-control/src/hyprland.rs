use crate::{DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput, WaylandDisplay};
use anyhow::{Context, Result, bail, ensure};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    time::{sleep, timeout},
};

const IPC_TIMEOUT: Duration = Duration::from_secs(2);
const RESIZE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REPLY: u64 = 1024 * 1024;
const LUA_REQUIRED: &str = "keyword can't work with non-legacy parsers. Use eval.";

/// Available only for an owned virtual desktop. Physical sessions use WaylandDisplay.
pub struct HyprlandDisplay {
    wayland: WaylandDisplay,
    ipc: PathBuf,
}

#[derive(Deserialize)]
struct Monitor {
    name: String,
    width: u32,
    height: u32,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    transform: u32,
    #[serde(default = "default_scale")]
    scale: f64,
}
fn default_scale() -> f64 {
    1.0
}
impl Monitor {
    fn resolution(&self) -> DisplayMode {
        let (width, height) = if self.transform % 2 == 1 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        DisplayMode {
            width,
            height,
            scale: self.scale,
        }
    }
}

impl HyprlandDisplay {
    pub fn new(socket: PathBuf, runtime: PathBuf, signature: &str) -> Result<Self> {
        ensure!(
            !signature.is_empty()
                && signature
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "invalid compositor signature"
        );
        Ok(Self {
            wayland: WaylandDisplay::new(socket),
            ipc: runtime.join("hypr").join(signature).join(".socket.sock"),
        })
    }

    async fn request(&self, request: &str) -> Result<String> {
        timeout(IPC_TIMEOUT, async {
            let mut stream = UnixStream::connect(&self.ipc)
                .await
                .context("compositor IPC unavailable")?;
            stream.write_all(request.as_bytes()).await?;
            let mut reply = Vec::new();
            stream.take(MAX_REPLY + 1).read_to_end(&mut reply).await?;
            ensure!(
                reply.len() as u64 <= MAX_REPLY,
                "compositor IPC reply too large"
            );
            String::from_utf8(reply).context("invalid compositor IPC response")
        })
        .await
        .context("compositor IPC timed out")?
    }

    async fn monitors(&self) -> Result<Vec<Monitor>> {
        let reply = self.request("j/monitors").await?;
        let monitors: Vec<Monitor> =
            serde_json::from_str(&reply).context("unsupported compositor monitor query")?;
        ensure!(!monitors.is_empty(), "compositor has no active outputs");
        Ok(monitors)
    }

    async fn apply(
        &self,
        output: Option<&str>,
        resolution: DisplayMode,
        fps: u32,
    ) -> Result<DisplayMode> {
        resolution.validate()?;
        ensure!((1..=120).contains(&fps), "invalid refresh rate");
        let monitors = self.monitors().await?;
        let selected = match output {
            Some(name) => monitors
                .iter()
                .find(|m| m.name == name)
                .context("selected output unavailable")?,
            None if monitors.len() == 1 => &monitors[0],
            None => bail!("select an output before resizing a multi-output desktop"),
        };
        let name = &selected.name;
        ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "unsupported output identifier"
        );
        if selected.resolution() != resolution {
            let DisplayMode {
                width,
                height,
                scale,
            } = resolution;
            let (width, height) = if selected.transform % 2 == 1 {
                (height, width)
            } else {
                (width, height)
            };
            let position = format!("{}x{}", selected.x, selected.y);
            let transform = selected.transform;
            let mut reply = self
                .request(&format!(
                    "keyword monitor {name},{width}x{height}@{fps},{position},{scale},transform,{transform}"
                ))
                .await?;
            if reply.trim() == LUA_REQUIRED {
                reply = self.request(&format!("eval hl.monitor({{ output = \"{name}\", mode = \"{width}x{height}@{fps}\", position = \"{position}\", scale = {scale}, transform = {transform} }})")).await?;
            }
            ensure!(
                reply.trim() == "ok",
                "desktop resize rejected: {}",
                reply.trim()
            );
        }
        loop {
            let monitors = self.monitors().await?;
            let monitor = monitors
                .iter()
                .find(|m| m.name == *name)
                .context("selected output disconnected")?;
            if monitor.resolution() == resolution
                && monitor.x == selected.x
                && monitor.y == selected.y
                && monitor.transform == selected.transform
            {
                return Ok(resolution);
            }
            sleep(Duration::from_millis(25)).await;
        }
    }
}

impl DisplayController for HyprlandDisplay {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        self.wayland.outputs()
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            match self.monitors().await {
                Ok(_) => DisplayCapabilities {
                    resize: true,
                    resize_scale: true,
                    reason: None,
                },
                Err(error) => DisplayCapabilities::unavailable(format!("{error:#}")),
            }
        })
    }
    fn resize<'a>(
        &'a self,
        output: Option<&'a str>,
        resolution: DisplayMode,
        fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        Box::pin(async move {
            timeout(RESIZE_TIMEOUT, self.apply(output, resolution, fps))
                .await
                .context("desktop resize did not apply before deadline")?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;
    const OLD: &str = r#"[{"name":"WAYLAND-1","width":1280,"height":720}]"#;
    const NEW: &str = r#"[{"name":"WAYLAND-1","width":800,"height":600}]"#;

    fn controller() -> (tempfile::TempDir, HyprlandDisplay, UnixListener) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hypr/test");
        std::fs::create_dir_all(&path).unwrap();
        let listener = UnixListener::bind(path.join(".socket.sock")).unwrap();
        let controller =
            HyprlandDisplay::new(dir.path().join("wayland-1"), dir.path().into(), "test").unwrap();
        (dir, controller, listener)
    }
    async fn reply(listener: &UnixListener, expected: &str, response: &str) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let size = stream.read(&mut request).await.unwrap();
        assert_eq!(std::str::from_utf8(&request[..size]).unwrap(), expected);
        stream.write_all(response.as_bytes()).await.unwrap();
    }
    fn size() -> DisplayMode {
        DisplayMode {
            width: 800,
            height: 600,
            scale: 1.0,
        }
    }

    #[tokio::test]
    async fn legacy_resize_targets_one_output_and_confirms_dimensions() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", OLD).await;
            reply(
                &listener,
                "keyword monitor WAYLAND-1,800x600@30,0x0,1,transform,0",
                "ok\n",
            )
            .await;
            reply(&listener, "j/monitors", NEW).await;
        });
        assert_eq!(control.resize(None, size(), 30).await.unwrap(), size());
        server.await.unwrap();
    }
    async fn resize_preserves_layout_and_transform(lua: bool) {
        for (transform, native_width, native_height) in [
            (1, 600, 800),
            (0, 800, 600),
            (2, 800, 600),
            (3, 600, 800),
            (4, 800, 600),
            (5, 600, 800),
            (6, 800, 600),
            (7, 600, 800),
        ] {
            let (_dir, control, listener) = controller();
            let server = tokio::spawn(async move {
                let state = |width, height| {
                    serde_json::json!([
                        {"name":"WAYLAND-1", "width":width, "height":height,
                         "x":-1280, "y":-120, "transform":transform},
                        {"name":"DP-2", "width":1920, "height":1080,
                         "x":0, "y":0, "transform":0}
                    ])
                    .to_string()
                };
                let old = state(1280, 720);
                let new = state(native_width, native_height);
                reply(&listener, "j/monitors", &old).await;
                reply(
                        &listener,
                        &format!("keyword monitor WAYLAND-1,{native_width}x{native_height}@30,-1280x-120,1,transform,{transform}"),
                        if lua { LUA_REQUIRED } else { "ok" },
                    ).await;
                if lua {
                    reply(
                            &listener,
                            &format!("eval hl.monitor({{ output = \"WAYLAND-1\", mode = \"{native_width}x{native_height}@30\", position = \"-1280x-120\", scale = 1, transform = {transform} }})"),
                            "ok",
                        ).await;
                }
                reply(&listener, "j/monitors", &old).await;
                reply(&listener, "j/monitors", &new).await;
            });
            let result = control.resize(Some("WAYLAND-1"), size(), 30).await;
            server.await.unwrap();
            assert_eq!(result.unwrap(), size(), "transform={transform}, lua={lua}");
        }
    }
    #[tokio::test]
    async fn legacy_resize_preserves_layout_and_transform() {
        resize_preserves_layout_and_transform(false).await;
    }
    #[tokio::test]
    async fn lua_resize_preserves_layout_and_transform() {
        resize_preserves_layout_and_transform(true).await;
    }
    #[tokio::test]
    async fn matching_dimensions_cannot_hide_changed_position_or_transform() {
        for (x, transform) in [(0, 0), (-1280, 2)] {
            let (_dir, control, listener) = controller();
            let server = tokio::spawn(async move {
                let old =
                    r#"[{"name":"WAYLAND-1","width":1280,"height":720,"x":-1280,"transform":0}]"#;
                let changed = serde_json::json!([
                    {"name":"WAYLAND-1", "width":800, "height":600, "x":x, "transform":transform}
                ])
                .to_string();
                reply(&listener, "j/monitors", old).await;
                reply(
                    &listener,
                    "keyword monitor WAYLAND-1,800x600@30,-1280x0,1,transform,0",
                    "ok",
                )
                .await;
                reply(&listener, "j/monitors", &changed).await;
            });
            assert!(control.resize(Some("WAYLAND-1"), size(), 30).await.is_err());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn lua_resize_uses_eval_only_for_explicit_capability_reply() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", OLD).await;
            reply(
                &listener,
                "keyword monitor WAYLAND-1,800x600@30,0x0,1,transform,0",
                LUA_REQUIRED,
            )
            .await;
            reply(&listener, "eval hl.monitor({ output = \"WAYLAND-1\", mode = \"800x600@30\", position = \"0x0\", scale = 1, transform = 0 })", "ok").await;
            reply(&listener, "j/monitors", NEW).await;
        });
        assert_eq!(control.resize(None, size(), 30).await.unwrap(), size());
        server.await.unwrap();
    }
    #[tokio::test]
    async fn identical_pixels_with_new_scale_are_applied_and_verified() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", NEW).await;
            reply(
                &listener,
                "keyword monitor WAYLAND-1,800x600@30,0x0,2,transform,0",
                LUA_REQUIRED,
            )
            .await;
            reply(&listener, "eval hl.monitor({ output = \"WAYLAND-1\", mode = \"800x600@30\", position = \"0x0\", scale = 2, transform = 0 })", "ok").await;
            reply(&listener, "j/monitors", NEW).await;
            reply(
                &listener,
                "j/monitors",
                r#"[{"name":"WAYLAND-1","width":800,"height":600,"scale":2}]"#,
            )
            .await;
        });
        let mode = DisplayMode {
            width: 800,
            height: 600,
            scale: 2.0,
        };
        assert_eq!(control.resize(None, mode, 30).await.unwrap(), mode);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn rejection_is_not_treated_as_success_or_lua_capability() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", OLD).await;
            reply(
                &listener,
                "keyword monitor WAYLAND-1,800x600@30,0x0,1,transform,0",
                "invalid monitor mode",
            )
            .await;
            assert!(
                timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        assert!(
            control
                .resize(None, size(), 30)
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid monitor mode")
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn already_matching_dimensions_do_not_write_configuration() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", NEW).await;
            reply(&listener, "j/monitors", NEW).await;
        });
        assert_eq!(control.resize(None, size(), 30).await.unwrap(), size());
        server.await.unwrap();
    }
    #[tokio::test]
    async fn missing_ipc_disables_resize_with_a_reason() {
        let (dir, control, listener) = controller();
        drop(listener);
        std::fs::remove_file(dir.path().join("hypr/test/.socket.sock")).unwrap();
        let capability = control.capabilities().await;
        assert!(!capability.resize);
        assert!(capability.reason.unwrap().contains("IPC unavailable"));
    }
    #[tokio::test]
    async fn invalid_resolution_and_unknown_output_are_rejected() {
        let (_dir, control, listener) = controller();
        assert!(
            control
                .resize(
                    None,
                    DisplayMode {
                        width: 0,
                        height: 600,
                        scale: 1.0
                    },
                    30
                )
                .await
                .is_err()
        );
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", OLD).await;
        });
        assert!(
            control
                .resize(Some("DP-2"), size(), 30)
                .await
                .unwrap_err()
                .to_string()
                .contains("unavailable")
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn ipc_timeout_is_bounded_and_reported() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            sleep(Duration::from_secs(5)).await;
        });
        let capability = control.capabilities().await;
        assert!(!capability.resize);
        assert!(capability.reason.unwrap().contains("timed out"));
        server.abort();
    }
    #[tokio::test]
    async fn ok_reply_without_dimension_change_is_not_success() {
        let (_dir, control, listener) = controller();
        let server = tokio::spawn(async move {
            reply(&listener, "j/monitors", OLD).await;
            reply(
                &listener,
                "keyword monitor WAYLAND-1,800x600@30,0x0,1,transform,0",
                "ok",
            )
            .await;
            loop {
                reply(&listener, "j/monitors", OLD).await;
            }
        });
        assert!(
            control
                .resize(None, size(), 30)
                .await
                .unwrap_err()
                .to_string()
                .contains("did not apply")
        );
        server.abort();
    }
    #[test]
    fn rejects_signature_paths_and_reports_rotated_dimensions() {
        assert!(HyprlandDisplay::new("/socket".into(), "/run".into(), "../host").is_err());
        let rotated = Monitor {
            name: "DP-1".into(),
            width: 600,
            height: 800,
            x: 0,
            y: 0,
            transform: 1,
            scale: 1.0,
        };
        assert_eq!(rotated.resolution(), size());
    }
}
