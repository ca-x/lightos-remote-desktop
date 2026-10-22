//! wlr-output-management through the maintained generated wayland-protocols-wlr bindings.
use crate::{DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput};
use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
use std::{collections::HashMap, path::PathBuf};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_output, wl_registry},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

pub struct WlrootsDisplay {
    socket: PathBuf,
}
#[derive(Default)]
struct Head {
    object: Option<ZwlrOutputHeadV1>,
    name: String,
    description: String,
    enabled: bool,
    current: Option<ZwlrOutputModeV1>,
    x: i32,
    y: i32,
    scale: f64,
    transform: Option<wl_output::Transform>,
    modes: Vec<ZwlrOutputModeV1>,
}
#[derive(Default)]
struct Mode {
    width: i32,
    height: i32,
    refresh: i32,
}
#[derive(Default)]
struct State {
    heads: HashMap<u32, Head>,
    modes: HashMap<u32, Mode>,
    serial: Option<u32>,
    finished: bool,
    result: Option<bool>,
}
impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.heads.insert(
                    head.id().protocol_id(),
                    Head {
                        object: Some(head),
                        scale: 1.0,
                        transform: Some(wl_output::Transform::Normal),
                        ..Default::default()
                    },
                );
            }
            zwlr_output_manager_v1::Event::Done { serial } => state.serial = Some(serial),
            zwlr_output_manager_v1::Event::Finished => state.finished = true,
            _ => {}
        }
        let _ = qh;
    }
    wayland_client::event_created_child!(State,ZwlrOutputManagerV1,[0=>(ZwlrOutputHeadV1,())]);
}
impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = head.id().protocol_id();
        if matches!(event, zwlr_output_head_v1::Event::Finished) {
            state.heads.remove(&id);
            return;
        }
        let Some(h) = state.heads.get_mut(&id) else {
            return;
        };
        match event {
            zwlr_output_head_v1::Event::Name { name } => h.name = name,
            zwlr_output_head_v1::Event::Description { description } => h.description = description,
            zwlr_output_head_v1::Event::Enabled { enabled } => h.enabled = enabled != 0,
            zwlr_output_head_v1::Event::Mode { mode } => {
                state.modes.insert(mode.id().protocol_id(), Mode::default());
                h.modes.push(mode);
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => h.current = Some(mode),
            zwlr_output_head_v1::Event::Position { x, y } => {
                h.x = x;
                h.y = y;
            }
            zwlr_output_head_v1::Event::Scale { scale } => h.scale = scale,
            zwlr_output_head_v1::Event::Transform { transform } => {
                h.transform = match transform {
                    WEnum::Value(v) => Some(v),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(State,ZwlrOutputHeadV1,[3=>(ZwlrOutputModeV1,())]);
}
impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        object: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = object.id().protocol_id();
        if matches!(event, zwlr_output_mode_v1::Event::Finished) {
            state.modes.remove(&id);
            return;
        }
        let Some(m) = state.modes.get_mut(&id) else {
            return;
        };
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                m.width = width;
                m.height = height;
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => m.refresh = refresh,
            _ => {}
        }
    }
}
impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_configuration_v1::Event::Succeeded => state.result = Some(true),
            zwlr_output_configuration_v1::Event::Failed
            | zwlr_output_configuration_v1::Event::Cancelled => state.result = Some(false),
            _ => {}
        }
    }
}
delegate_noop!(State:ignore ZwlrOutputConfigurationHeadV1);
fn rotated(transform: wl_output::Transform) -> bool {
    matches!(
        transform,
        wl_output::Transform::_90
            | wl_output::Transform::_270
            | wl_output::Transform::Flipped90
            | wl_output::Transform::Flipped270
    )
}
impl State {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        ensure!(!self.finished, "output management is no longer available");
        let outputs = self
            .heads
            .values()
            .filter(|h| h.enabled)
            .map(|h| {
                let m = self
                    .modes
                    .get(
                        &h.current
                            .as_ref()
                            .context("wlroots output has no current mode")?
                            .id()
                            .protocol_id(),
                    )
                    .context("wlroots current mode missing")?;
                let (w, t) = if rotated(h.transform.context("unsupported output transform")?) {
                    (m.height, m.width)
                } else {
                    (m.width, m.height)
                };
                ensure!(
                    w > 0 && t > 0 && h.scale > 0.0 && h.scale.is_finite(),
                    "invalid wlroots output dimensions/scale"
                );
                Ok(DisplayOutput {
                    name: h.name.clone(),
                    description: h.description.clone(),
                    width: w as u32,
                    height: t as u32,
                    x: h.x,
                    y: h.y,
                    scale: h.scale,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(!outputs.is_empty(), "wlroots has no active outputs");
        Ok(outputs)
    }
    fn configure(
        &self,
        manager: &ZwlrOutputManagerV1,
        qh: &QueueHandle<Self>,
        name: &str,
        mode: DisplayMode,
        fps: u32,
    ) -> Result<ZwlrOutputConfigurationV1> {
        // Resolve and validate everything before creating a protocol configuration.
        let mut heads = Vec::new();
        for h in self.heads.values() {
            if !h.enabled {
                continue;
            }
            let transform = h.transform.context("unsupported output transform")?;
            let selected = if h.name == name {
                h.modes
                    .iter()
                    .filter(|id| {
                        self.modes.get(&id.id().protocol_id()).is_some_and(|m| {
                            let (w, t) = if rotated(transform) {
                                (m.height, m.width)
                            } else {
                                (m.width, m.height)
                            };
                            w as u32 == mode.width && t as u32 == mode.height
                        })
                    })
                    .min_by_key(|id| {
                        (self.modes[&id.id().protocol_id()].refresh - fps as i32 * 1000).abs()
                    })
            } else {
                h.current.as_ref()
            }
            .context("requested wlroots mode is unavailable")?;
            heads.push((h, selected, transform));
        }
        let config =
            manager.create_configuration(self.serial.context("missing wlroots serial")?, qh, ());
        for h in self.heads.values().filter(|h| !h.enabled) {
            config.disable_head(h.object.as_ref().context("missing head")?);
        }
        for (h, selected, transform) in heads {
            let settings = config.enable_head(h.object.as_ref().context("missing head")?, qh, ());
            settings.set_mode(selected);
            settings.set_position(h.x, h.y);
            settings.set_transform(transform);
            settings.set_scale(if h.name == name { mode.scale } else { h.scale });
        }
        Ok(config)
    }
}
impl WlrootsDisplay {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
    fn operate(
        &self,
        request: Option<(Option<&str>, DisplayMode, u32)>,
    ) -> Result<Vec<DisplayOutput>> {
        crate::wayland::with_connection(&self.socket, |connection| {
            let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
            let qh = queue.handle();
            let manager: ZwlrOutputManagerV1 = globals
                .bind(&qh, 1..=1, ())
                .context("compositor does not expose wlr-output-management")?;
            let mut state = State::default();
            queue.roundtrip(&mut state)?;
            let outputs = state.outputs()?;
            if let Some((name, mode, fps)) = request {
                mode.validate()?;
                ensure!((1..=120).contains(&fps), "invalid refresh rate");
                let selected = crate::select_output(&outputs, name)?;
                if crate::output_matches(selected, mode) {
                    return Ok(outputs);
                }
                let name = selected.name.clone();
                for test in [true, false] {
                    state.result = None;
                    let config = state.configure(&manager, &qh, &name, mode, fps)?;
                    if test {
                        config.test();
                    } else {
                        config.apply();
                    }
                    while state.result.is_none() && !state.finished {
                        queue.blocking_dispatch(&mut state)?;
                    }
                    config.destroy();
                    ensure!(
                        state.result == Some(true),
                        "wlroots rejected or cancelled output configuration"
                    );
                    queue.roundtrip(&mut state)?;
                }
                let outputs = state.outputs()?;
                ensure!(
                    outputs
                        .iter()
                        .any(|o| o.name == name && crate::output_matches(o, mode)),
                    "wlroots acknowledged change without applying mode and scale"
                );
                Ok(outputs)
            } else {
                Ok(outputs)
            }
        })
    }
}
impl DisplayController for WlrootsDisplay {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        self.operate(None)
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        let socket = self.socket.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || Self::new(socket).outputs()).await {
                Ok(Ok(_)) => DisplayCapabilities {
                    resize: true,
                    resize_scale: true,
                    reason: None,
                },
                Ok(Err(e)) => DisplayCapabilities::unavailable(format!("{e:#}")),
                Err(e) => DisplayCapabilities::unavailable(e),
            }
        })
    }
    fn resize<'a>(
        &'a self,
        name: Option<&'a str>,
        mode: DisplayMode,
        fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        let socket = self.socket.clone();
        let name = name.map(str::to_owned);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                Self::new(socket).operate(Some((name.as_deref(), mode, fps)))
            })
            .await??;
            Ok(mode)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn missing_selected_socket_cannot_be_replaced_by_the_host_session() {
        let display = WlrootsDisplay::new("/nonexistent/selected-session/wayland".into());
        let capability = display.capabilities().await;
        assert!(!capability.resize);
        assert!(capability.reason.is_some());
        assert!(
            display
                .resize(
                    None,
                    DisplayMode {
                        width: 800,
                        height: 600,
                        scale: 1.0
                    },
                    30
                )
                .await
                .is_err()
        );
    }
    #[test]
    fn unsupported_and_empty_protocol_state_is_not_advertised_as_usable() {
        assert!(State::default().outputs().is_err());
        let state = State {
            finished: true,
            ..Default::default()
        };
        assert!(
            state
                .outputs()
                .unwrap_err()
                .to_string()
                .contains("no longer available")
        );
    }
}
