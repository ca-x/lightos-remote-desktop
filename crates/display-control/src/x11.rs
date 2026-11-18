//! Native RandR via x11rb-async; supports X11 desktop environments independently of distribution.
use crate::{
    DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput, SessionTarget, command,
};
use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
use std::{path::PathBuf, time::Duration};
use x11rb::rust_connection::DefaultStream;
use x11rb_async::{
    connection::Connection,
    protocol::{
        randr::{self, ConnectionExt as _, Rotation, SetConfig},
        xproto::{ConnectionExt as _, CreateWindowAux, WindowClass},
    },
    rust_connection::{RustConnection, StreamAdaptor},
};

type Resources = randr::GetScreenResourcesCurrentReply;
type Crtc = randr::GetCrtcInfoReply;
struct Output {
    id: u32,
    info: DisplayOutput,
    crtc: u32,
    current: Crtc,
    modes: Vec<u32>,
}
pub struct X11Display {
    target: SessionTarget,
    dynamic_modes: bool,
    mutter: Option<crate::MutterDisplay>,
    operations: tokio::sync::Mutex<()>,
}
impl X11Display {
    pub fn new(target: SessionTarget) -> Self {
        Self {
            target,
            dynamic_modes: false,
            mutter: None,
            operations: tokio::sync::Mutex::new(()),
        }
    }
    pub fn virtual_display(target: SessionTarget) -> Self {
        Self::virtual_session(
            target,
            std::env::var("LIGHTOS_REMOTE_DESKTOP_GNOME_X11").as_deref() == Ok("1"),
        )
    }
    pub fn virtual_session(target: SessionTarget, gnome: bool) -> Self {
        let mutter = gnome.then(|| crate::MutterDisplay::with_external_modes(target.clone()));
        Self {
            target,
            dynamic_modes: true,
            mutter,
            operations: tokio::sync::Mutex::new(()),
        }
    }
    async fn connect(
        &self,
    ) -> Result<(
        RustConnection,
        usize,
        impl std::future::Future<
            Output = Result<std::convert::Infallible, x11rb_async::errors::ConnectionError>,
        > + Send,
    )> {
        let display = self
            .target
            .x11_display
            .as_deref()
            .context("selected session has no X11 display")?;
        let (number, screen) = local_display(display)?;
        let mut auth = (Vec::new(), Vec::new());
        if let Some(path) = &self.target.xauthority {
            ensure!(path.is_file(), "selected Xauthority file is missing");
            let text = command::run(
                &command::executable("xauth")?,
                &[
                    "-f".into(),
                    path.to_string_lossy().into_owned(),
                    "list".into(),
                    format!(":{number}"),
                ],
                &self.target,
            )
            .await?;
            let entries: Vec<_> = text
                .lines()
                .filter_map(|line| {
                    let f: Vec<_> = line.split_whitespace().collect();
                    (f.len() == 3 && f[1] == "MIT-MAGIC-COOKIE-1")
                        .then(|| (f[1].to_string(), f[2].to_string()))
                })
                .collect();
            let first = entries
                .first()
                .context("no Xauthority cookie for selected display")?;
            ensure!(
                entries.iter().all(|entry| entry == first),
                "ambiguous Xauthority credentials"
            );
            auth = (
                first.0.as_bytes().to_vec(),
                hex::decode(&first.1).context("invalid Xauthority cookie")?,
            );
        }
        let stream =
            tokio::net::UnixStream::connect(PathBuf::from(format!("/tmp/.X11-unix/X{number}")))
                .await?
                .into_std()?;
        let (stream, _) = DefaultStream::from_unix_stream(stream)?;
        let (connection, drive) = RustConnection::connect_to_stream_with_auth_info(
            StreamAdaptor::new(stream)?,
            screen,
            auth.0,
            auth.1,
        )
        .await?;
        Ok((connection, screen, drive))
    }
    async fn operate(
        &self,
        request: Option<(Option<&str>, DisplayMode, u32)>,
    ) -> Result<Vec<DisplayOutput>> {
        let _operation = self.operations.lock().await;
        let (connection, screen, drive) = self.connect().await?;
        let operation =
            async {
                connection.randr_query_version(1, 3).await?.reply().await?;
                let root = connection
                    .setup()
                    .roots
                    .get(screen)
                    .context("invalid X11 screen")?
                    .root;
                let resources = connection
                    .randr_get_screen_resources_current(root)
                    .await?
                    .reply()
                    .await?;
                let outputs = query(&connection, &resources).await?;
                if let Some((name, mode, fps)) = request {
                    mode.validate()?;
                    ensure!(mode.scale == 1.0, "RandR backend supports scale 1 only");
                    ensure!((1..=120).contains(&fps), "invalid refresh rate");
                    let infos: Vec<_> = outputs.iter().map(|o| o.info.clone()).collect();
                    let selected = crate::select_output(&infos, name)?;
                    if crate::output_matches(selected, mode) {
                        return Ok(infos);
                    }
                    let output = outputs
                        .iter()
                        .find(|o| o.info.name == selected.name)
                        .unwrap();
                    ensure!(
                        output.current.outputs.len() == 1,
                        "cannot resize one output in a cloned RandR CRTC"
                    );
                    let rotated = output
                        .current
                        .rotation
                        .intersects(Rotation::ROTATE90 | Rotation::ROTATE270);
                    let chosen = resources
                        .modes
                        .iter()
                        .filter(|m| {
                            output.modes.contains(&m.id)
                                && if rotated {
                                    (m.height as u32, m.width as u32)
                                } else {
                                    (m.width as u32, m.height as u32)
                                } == (mode.width, mode.height)
                        })
                        .min_by(|a, b| {
                            (refresh(a) - fps as f64)
                                .abs()
                                .total_cmp(&(refresh(b) - fps as f64).abs())
                        })
                        .cloned();
                    let chosen = match chosen {
                        Some(chosen) => chosen,
                        None if self.dynamic_modes => {
                            ensure!(
                                outputs.len() == 1
                                    && output.current.x == 0
                                    && output.current.y == 0
                                    && output.current.rotation == Rotation::ROTATE0,
                                "dynamic virtual modes require one unrotated output"
                            );
                            let range = connection
                                .randr_get_screen_size_range(root)
                                .await?
                                .reply()
                                .await?;
                            ensure!(
                                mode.width >= u32::from(range.min_width)
                                    && mode.width <= u32::from(range.max_width)
                                    && mode.height >= u32::from(range.min_height)
                                    && mode.height <= u32::from(range.max_height),
                                "virtual resolution exceeds the framebuffer bounds"
                            );
                            let name: &[u8] = if mode_name(&resources, output.current.mode)
                                == Some(b"LIGHTOS-AUTO-0")
                            {
                                b"LIGHTOS-AUTO-1"
                            } else {
                                b"LIGHTOS-AUTO-0"
                            };
                            if let Some(old) = resources
                                .modes
                                .iter()
                                .find(|old| mode_name(&resources, old.id) == Some(name))
                            {
                                ensure!(
                                    old.id != output.current.mode,
                                    "refusing to replace an active virtual mode"
                                );
                                if output.modes.contains(&old.id) {
                                    connection
                                        .randr_delete_output_mode(output.id, old.id)
                                        .await?
                                        .check()
                                        .await?;
                                }
                                connection.randr_destroy_mode(old.id).await?.check().await?;
                            }
                            let width = u16::try_from(mode.width)?;
                            let height = u16::try_from(mode.height)?;
                            let mut info = randr::ModeInfo {
                                width,
                                height,
                                hsync_start: width,
                                hsync_end: width,
                                htotal: width,
                                vsync_start: height,
                                vsync_end: height,
                                vtotal: height,
                                dot_clock: u32::try_from(
                                    u64::from(mode.width) * u64::from(mode.height) * u64::from(fps),
                                )?,
                                name_len: u16::try_from(name.len())?,
                                ..Default::default()
                            };
                            info.id = connection
                                .randr_create_mode(root, info, name)
                                .await?
                                .reply()
                                .await?
                                .mode;
                            connection
                                .randr_add_output_mode(output.id, info.id)
                                .await?
                                .check()
                                .await?;
                            info
                        }
                        None => anyhow::bail!("requested RandR mode is unavailable"),
                    };
                    ensure!(
                        outputs.iter().all(|o| o.info.x >= 0 && o.info.y >= 0),
                        "unsupported negative RandR layout"
                    );
                    let width = outputs
                        .iter()
                        .map(|o| {
                            o.info.x as u32
                                + if o.crtc == output.crtc {
                                    mode.width
                                } else {
                                    o.info.width
                                }
                        })
                        .max()
                        .unwrap();
                    let height = outputs
                        .iter()
                        .map(|o| {
                            o.info.y as u32
                                + if o.crtc == output.crtc {
                                    mode.height
                                } else {
                                    o.info.height
                                }
                        })
                        .max()
                        .unwrap();
                    let range = connection
                        .randr_get_screen_size_range(root)
                        .await?
                        .reply()
                        .await?;
                    ensure!(
                        width <= range.max_width as u32 && height <= range.max_height as u32,
                        "RandR layout exceeds screen bounds"
                    );
                    let geometry = connection.get_geometry(root).await?.reply().await?;
                    let target_width = width.max(range.min_width as u32) as u16;
                    let target_height = height.max(range.min_height as u32) as u16;
                    if target_width > geometry.width || target_height > geometry.height {
                        let w = target_width.max(geometry.width);
                        let h = target_height.max(geometry.height);
                        connection
                            .randr_set_screen_size(
                                root,
                                w,
                                h,
                                (w as u32 * 254 / 960).max(1),
                                (h as u32 * 254 / 960).max(1),
                            )
                            .await?
                            .check()
                            .await?;
                    }
                    let config_timestamp = if self.dynamic_modes {
                        connection
                            .randr_get_screen_resources_current(root)
                            .await?
                            .reply()
                            .await?
                            .config_timestamp
                    } else {
                        resources.config_timestamp
                    };
                    let changed = connection
                        .randr_set_crtc_config(
                            output.crtc,
                            0,
                            config_timestamp,
                            output.current.x,
                            output.current.y,
                            chosen.id,
                            output.current.rotation,
                            &output.current.outputs,
                        )
                        .await?
                        .reply()
                        .await?;
                    ensure!(
                        changed.status == SetConfig::SUCCESS,
                        "RandR rejected configuration or serial changed"
                    );
                    connection
                        .randr_set_screen_size(
                            root,
                            target_width,
                            target_height,
                            (target_width as u32 * 254 / 960).max(1),
                            (target_height as u32 * 254 / 960).max(1),
                        )
                        .await?
                        .check()
                        .await?;
                    let current = connection
                        .randr_get_screen_resources_current(root)
                        .await?
                        .reply()
                        .await?;
                    let applied = query(&connection, &current).await?;
                    ensure!(
                        applied.iter().any(|o| o.info.name == output.info.name
                            && crate::output_matches(&o.info, mode)),
                        "RandR acknowledged change without applying dimensions"
                    );
                    if self.dynamic_modes {
                        // Like xrefresh, expose the complete new virtual root.
                        // Older compositors can otherwise retain the previous
                        // drawable's damage clip after a shrink/grow sequence.
                        // No pixels or input are changed on a physical display.
                        let refresh = connection.generate_id().await?;
                        connection
                            .create_window(
                                0,
                                refresh,
                                root,
                                0,
                                0,
                                target_width,
                                target_height,
                                0,
                                WindowClass::INPUT_OUTPUT,
                                0,
                                &CreateWindowAux::new()
                                    .background_pixmap(0)
                                    .override_redirect(1),
                            )
                            .await?
                            .check()
                            .await?;
                        connection.map_window(refresh).await?.check().await?;
                        connection.destroy_window(refresh).await?.check().await?;
                    }
                    Ok(applied.into_iter().map(|o| o.info).collect())
                } else {
                    Ok(outputs.into_iter().map(|o| o.info).collect())
                }
            };
        let transaction = async {
            // Keep the compositor from querying a half-applied screen/CRTC
            // layout, as xrandr does. This connection is private to this call;
            // cancellation or an error closes it and releases the server grab.
            let grabbed = self.dynamic_modes && request.is_some();
            if grabbed {
                connection.grab_server().await?.check().await?;
            }
            let result = operation.await;
            if grabbed {
                connection.ungrab_server().await?.check().await?;
            }
            result
        };
        tokio::select! {result=transaction=>result,error=drive=>{error?;unreachable!()}}
    }
    async fn bounded(
        &self,
        request: Option<(Option<&str>, DisplayMode, u32)>,
    ) -> Result<Vec<DisplayOutput>> {
        tokio::time::timeout(Duration::from_secs(5), self.operate(request))
            .await
            .context("RandR operation timed out")?
    }
}
fn mode_name(resources: &Resources, id: u32) -> Option<&[u8]> {
    let mut offset = 0usize;
    for mode in &resources.modes {
        let end = offset.checked_add(usize::from(mode.name_len))?;
        if mode.id == id {
            return resources.names.get(offset..end);
        }
        offset = end;
    }
    None
}

fn local_display(display: &str) -> Result<(u16, usize)> {
    let value = display
        .strip_prefix(':')
        .context("only explicit local X11 displays are supported")?;
    let (number, screen) = value.split_once('.').unwrap_or((value, "0"));
    Ok((
        number.parse().context("invalid X11 display number")?,
        screen.parse().context("invalid X11 screen")?,
    ))
}
fn refresh(mode: &randr::ModeInfo) -> f64 {
    if mode.htotal == 0 || mode.vtotal == 0 {
        0.0
    } else {
        mode.dot_clock as f64 / mode.htotal as f64 / mode.vtotal as f64
    }
}
async fn query(conn: &RustConnection, resources: &Resources) -> Result<Vec<Output>> {
    let mut outputs = Vec::new();
    for id in &resources.outputs {
        let info = conn
            .randr_get_output_info(*id, resources.config_timestamp)
            .await?
            .reply()
            .await?;
        if info.crtc == 0 || info.connection != randr::Connection::CONNECTED {
            continue;
        }
        let current = conn
            .randr_get_crtc_info(info.crtc, resources.config_timestamp)
            .await?
            .reply()
            .await?;
        let t = conn
            .randr_get_crtc_transform(info.crtc)
            .await?
            .reply()
            .await?
            .current_transform;
        ensure!(
            t.matrix11 == 65536
                && t.matrix22 == 65536
                && t.matrix33 == 65536
                && [
                    t.matrix12, t.matrix13, t.matrix21, t.matrix23, t.matrix31, t.matrix32
                ]
                .iter()
                .all(|v| *v == 0),
            "RandR output uses an unsupported transform; refusing scale assumptions"
        );
        let name = String::from_utf8(info.name).context("invalid RandR output name")?;
        outputs.push(Output {
            id: *id,
            info: DisplayOutput {
                name: name.clone(),
                description: name,
                width: current.width as u32,
                height: current.height as u32,
                x: current.x as i32,
                y: current.y as i32,
                scale: 1.0,
            },
            crtc: info.crtc,
            current,
            modes: info.modes,
        });
    }
    ensure!(!outputs.is_empty(), "RandR has no active outputs");
    Ok(outputs)
}
impl DisplayController for X11Display {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        command::blocking(self.bounded(None))
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            match self.bounded(None).await {
                Ok(_) => DisplayCapabilities {
                    resize: true,
                    resize_scale: false,
                    reason: None,
                },
                Err(e) => DisplayCapabilities::unavailable(format!("{e:#}")),
            }
        })
    }
    fn resize<'a>(
        &'a self,
        name: Option<&'a str>,
        mode: DisplayMode,
        fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        Box::pin(async move {
            self.bounded(Some((name, mode, fps))).await?;
            if let Some(mutter) = &self.mutter {
                let mut last_failure = String::new();
                let result = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        // RandR mode registration is delivered asynchronously
                        // to Mutter; allow its state/serial to catch up.
                        match mutter.resize(name, mode, fps).await {
                            Ok(applied) => return Ok(applied),
                            Err(error) => {
                                last_failure = format!("{error:#}");
                                tokio::time::sleep(Duration::from_millis(40)).await;
                            }
                        }
                    }
                })
                .await;
                match result {
                    Ok(result) => result,
                    Err(_) => anyhow::bail!(
                        "Mutter did not adopt the registered virtual X11 mode: {last_failure}"
                    ),
                }
            } else {
                Ok(mode)
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_local_display_without_ambient_fallback() {
        assert_eq!(local_display(":2.1").unwrap(), (2, 1));
        for value in ["", "host:0", ":bad", ":0.abc"] {
            assert!(local_display(value).is_err());
        }
    }
}
