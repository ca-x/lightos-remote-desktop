//! KScreen is operated through its supported kscreen-doctor helper; no compatible Rust KScreen library exists.
use crate::{
    DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput, SessionTarget, command,
};
use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use std::time::Duration;

pub struct KScreenDisplay {
    target: SessionTarget,
}
#[derive(Deserialize)]
struct Config {
    outputs: Vec<Output>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    id: u32,
    name: String,
    connected: bool,
    #[serde(default = "enabled")]
    enabled: bool,
    current_mode_id: String,
    scale: f64,
    pos: Position,
    rotation: u32,
    modes: Vec<Mode>,
}
fn enabled() -> bool {
    true
}
#[derive(Deserialize)]
struct Position {
    x: i32,
    y: i32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mode {
    id: String,
    size: Size,
    refresh_rate: f64,
}
#[derive(Deserialize)]
struct Size {
    width: u32,
    height: u32,
}
impl Output {
    fn dimensions(&self, m: &Mode) -> (u32, u32) {
        if matches!(self.rotation, 2 | 8) {
            (m.size.height, m.size.width)
        } else {
            (m.size.width, m.size.height)
        }
    }
}
impl Config {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        let result = self
            .outputs
            .iter()
            .filter(|o| o.connected && o.enabled)
            .map(|o| {
                let mode = o
                    .modes
                    .iter()
                    .find(|m| m.id == o.current_mode_id)
                    .context("KScreen active output has no current mode")?;
                let (width, height) = o.dimensions(mode);
                ensure!(
                    width > 0 && height > 0 && o.scale.is_finite() && o.scale > 0.0,
                    "invalid KScreen mode"
                );
                Ok(DisplayOutput {
                    name: o.name.clone(),
                    description: o.name.clone(),
                    width,
                    height,
                    x: o.pos.x,
                    y: o.pos.y,
                    scale: o.scale,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(!result.is_empty(), "KScreen has no active outputs");
        Ok(result)
    }
    fn arguments(&self, name: &str, requested: DisplayMode, fps: u32) -> Result<Vec<String>> {
        let output = self
            .outputs
            .iter()
            .find(|o| o.name == name && o.connected && o.enabled)
            .context("selected KScreen output unavailable")?;
        let mode = output
            .modes
            .iter()
            .filter(|m| output.dimensions(m) == (requested.width, requested.height))
            .min_by(|a, b| {
                (a.refresh_rate - fps as f64)
                    .abs()
                    .total_cmp(&(b.refresh_rate - fps as f64).abs())
            })
            .context("requested KScreen mode is unavailable")?;
        // doctor uses dot-separated syntax. Identifiers cannot contain separators.
        ensure!(
            !mode.id.is_empty()
                && mode
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "unsupported KScreen mode identifier"
        );
        Ok(vec![
            format!("output.{}.mode.{}", output.id, mode.id),
            format!("output.{}.scale.{}", output.id, requested.scale),
        ])
    }
}
impl KScreenDisplay {
    pub fn new(target: SessionTarget) -> Self {
        Self { target }
    }
    async fn query(&self) -> Result<Config> {
        ensure!(
            self.target.wayland_socket.is_some() && self.target.bus_address.is_some(),
            "KScreen requires an explicit Wayland session and D-Bus address"
        );
        let text = command::run(
            &command::executable("kscreen-doctor")?,
            &["--json".into()],
            &self.target,
        )
        .await?;
        let config: Config =
            serde_json::from_str(&text).context("unsupported KScreen JSON response")?;
        config.outputs()?;
        Ok(config)
    }
    async fn apply(
        &self,
        name: Option<&str>,
        requested: DisplayMode,
        fps: u32,
    ) -> Result<DisplayMode> {
        requested.validate()?;
        ensure!((1..=120).contains(&fps), "invalid refresh rate");
        let config = self.query().await?;
        let outputs = config.outputs()?;
        let output = crate::select_output(&outputs, name)?;
        let name = output.name.clone();
        if crate::output_matches(output, requested) {
            return Ok(requested);
        }
        command::run(
            &command::executable("kscreen-doctor")?,
            &config.arguments(&name, requested, fps)?,
            &self.target,
        )
        .await?;
        loop {
            if self
                .query()
                .await?
                .outputs()?
                .iter()
                .any(|o| o.name == name && crate::output_matches(o, requested))
            {
                return Ok(requested);
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}
impl DisplayController for KScreenDisplay {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        command::blocking(async { self.query().await?.outputs() })
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            match self.query().await {
                Ok(_) => DisplayCapabilities {
                    resize: true,
                    resize_scale: true,
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
            tokio::time::timeout(Duration::from_secs(5), self.apply(name, mode, fps))
                .await
                .context("KScreen did not apply mode before deadline")?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const STATE: &str = r#"{"outputs":[{"id":1,"name":"DP.1","connected":true,"enabled":true,"currentModeId":"1","scale":1,"pos":{"x":0,"y":0},"rotation":1,"modes":[{"id":"1","size":{"width":1280,"height":720},"refreshRate":60},{"id":"2","size":{"width":800,"height":600},"refreshRate":30}]},{"id":2,"name":"OFF","connected":true,"enabled":false,"currentModeId":"","scale":1,"pos":{"x":1280,"y":0},"rotation":1,"modes":[]}]}"#;
    #[test]
    fn selects_native_mode_by_id_without_touching_disabled_or_other_outputs() {
        let config: Config = serde_json::from_str(STATE).unwrap();
        let outputs = config.outputs().unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "DP.1");
        assert_eq!(
            config
                .arguments(
                    "DP.1",
                    DisplayMode {
                        width: 800,
                        height: 600,
                        scale: 2.0
                    },
                    30
                )
                .unwrap(),
            vec!["output.1.mode.2", "output.1.scale.2"]
        );
        assert!(
            config
                .arguments(
                    "DP.1",
                    DisplayMode {
                        width: 900,
                        height: 600,
                        scale: 1.0
                    },
                    30
                )
                .is_err()
        );
        assert!(
            config
                .arguments(
                    "OFF",
                    DisplayMode {
                        width: 800,
                        height: 600,
                        scale: 1.0
                    },
                    30
                )
                .is_err()
        );
    }
    #[test]
    fn rotation_and_unexpected_schema_do_not_create_false_capabilities() {
        let mut config: Config = serde_json::from_str(STATE).unwrap();
        config.outputs[0].rotation = 2;
        assert_eq!(
            (
                config.outputs().unwrap()[0].width,
                config.outputs().unwrap()[0].height
            ),
            (720, 1280)
        );
        assert!(serde_json::from_str::<Config>("{}").is_err());
        config.outputs[0].current_mode_id = "missing".into();
        assert!(config.outputs().is_err());
    }
    #[tokio::test]
    async fn missing_session_coordinates_do_not_probe_host_environment() {
        let display = KScreenDisplay::new(SessionTarget::default());
        assert!(!display.capabilities().await.resize);
    }
}
