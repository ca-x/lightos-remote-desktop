//! Mutter's typed DisplayConfig API. No gsettings edits or persistent monitors.xml writes.
use crate::{
    DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput, SessionTarget, command,
};
use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
use std::{collections::HashMap, time::Duration};
use zbus::{Connection, Proxy, zvariant::OwnedValue};

type Props = HashMap<String, OwnedValue>;
type Spec = (String, String, String, String);
type Mode = (String, i32, i32, f64, f64, Vec<f64>, Props);
type Monitor = (Spec, Vec<Mode>, Props);
type Logical = (i32, i32, f64, u32, bool, Vec<Spec>, Props);
type State = (u32, Vec<Monitor>, Vec<Logical>, Props);
type Applied = (i32, i32, f64, u32, bool, Vec<(String, String, Props)>);
const SERVICE: &str = "org.gnome.Mutter.DisplayConfig";

pub struct MutterDisplay {
    target: SessionTarget,
}
impl MutterDisplay {
    pub fn new(target: SessionTarget) -> Self {
        Self { target }
    }
    async fn connection(&self) -> Result<Connection> {
        let address = self
            .target
            .bus_address
            .as_deref()
            .context("selected session has no D-Bus address")?;
        Ok(zbus::connection::Builder::address(address)?.build().await?)
    }
    async fn state(&self) -> Result<State> {
        let bus = self.connection().await?;
        let proxy = Proxy::new(&bus, SERVICE, "/org/gnome/Mutter/DisplayConfig", SERVICE).await?;
        Ok(proxy.call("GetCurrentState", &()).await?)
    }
    async fn query(&self) -> Result<Vec<DisplayOutput>> {
        outputs(&self.state().await?)
    }
    async fn apply(
        &self,
        name: Option<&str>,
        requested: DisplayMode,
        fps: u32,
    ) -> Result<DisplayMode> {
        requested.validate()?;
        ensure!((1..=120).contains(&fps), "invalid refresh rate");
        let bus = self.connection().await?;
        let proxy = Proxy::new(&bus, SERVICE, "/org/gnome/Mutter/DisplayConfig", SERVICE).await?;
        ensure!(
            proxy
                .get_property::<bool>("ApplyMonitorsConfigAllowed")
                .await?,
            "Mutter denies display configuration"
        );
        let state: State = proxy.call("GetCurrentState", &()).await?;
        let current = outputs(&state)?;
        let selected = crate::select_output(&current, name)?;
        let name = selected.name.clone();
        if crate::output_matches(selected, requested) {
            return Ok(requested);
        }
        let config = configuration(&state, &name, requested, fps)?;
        // Verify without writing, then apply temporarily. Keep serial to reject concurrent changes.
        let mut properties = Props::new();
        if state
            .3
            .get("supports-changing-layout-mode")
            .and_then(|v| bool::try_from(v).ok())
            == Some(true)
            && let Some(mode) = state.3.get("layout-mode")
        {
            properties.insert("layout-mode".into(), mode.try_clone()?);
        }
        proxy
            .call::<_, _, ()>(
                "ApplyMonitorsConfig",
                &(state.0, 0u32, &config, &properties),
            )
            .await?;
        proxy
            .call::<_, _, ()>(
                "ApplyMonitorsConfig",
                &(state.0, 1u32, &config, &properties),
            )
            .await?;
        loop {
            if self
                .query()
                .await?
                .iter()
                .any(|o| o.name == name && crate::output_matches(o, requested))
            {
                return Ok(requested);
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}

fn outputs(state: &State) -> Result<Vec<DisplayOutput>> {
    let mut outputs = Vec::new();
    for (x, y, scale, transform, _, specs, _) in &state.2 {
        for spec in specs {
            let monitor = state
                .1
                .iter()
                .find(|m| m.0 == *spec)
                .context("Mutter logical monitor has no physical monitor")?;
            let mode = monitor
                .1
                .iter()
                .find(|m| m.6.get("is-current").and_then(|v| bool::try_from(v).ok()) == Some(true))
                .context("Mutter output has no current mode")?;
            let (w, h) = if transform % 2 == 1 {
                (mode.2, mode.1)
            } else {
                (mode.1, mode.2)
            };
            ensure!(
                w > 0 && h > 0 && scale.is_finite() && *scale > 0.0,
                "invalid Mutter output dimensions/scale"
            );
            outputs.push(DisplayOutput {
                name: spec.0.clone(),
                description: format!("{} {}", spec.1, spec.2),
                width: w as u32,
                height: h as u32,
                x: *x,
                y: *y,
                scale: *scale,
            });
        }
    }
    ensure!(!outputs.is_empty(), "Mutter has no active outputs");
    Ok(outputs)
}

fn configuration(
    state: &State,
    name: &str,
    requested: DisplayMode,
    fps: u32,
) -> Result<Vec<Applied>> {
    let mut config = Vec::new();
    for (x, y, scale, transform, primary, specs, _) in &state.2 {
        let changing = specs.iter().any(|s| s.0 == name);
        ensure!(
            !changing || specs.len() == 1,
            "cannot resize one output in a mirrored Mutter group"
        );
        if changing
            && state
                .3
                .get("global-scale-required")
                .and_then(|v| bool::try_from(v).ok())
                == Some(true)
        {
            ensure!(
                state.2.iter().all(|l| l.2 == requested.scale),
                "Mutter requires a global scale; refusing to alter other outputs"
            );
        }
        let mut monitors = Vec::new();
        for spec in specs {
            let monitor = state
                .1
                .iter()
                .find(|m| m.0 == *spec)
                .context("Mutter output disappeared")?;
            let mode = if changing {
                monitor
                    .1
                    .iter()
                    .filter(|m| {
                        let (w, h) = if transform % 2 == 1 {
                            (m.2, m.1)
                        } else {
                            (m.1, m.2)
                        };
                        w as u32 == requested.width
                            && h as u32 == requested.height
                            && m.5.contains(&requested.scale)
                    })
                    .min_by(|a, b| {
                        (a.3 - fps as f64)
                            .abs()
                            .total_cmp(&(b.3 - fps as f64).abs())
                    })
            } else {
                monitor.1.iter().find(|m| {
                    m.6.get("is-current").and_then(|v| bool::try_from(v).ok()) == Some(true)
                })
            }
            .context("requested Mutter mode/scale is unavailable")?;
            let mut props = Props::new();
            for key in ["is-underscanning", "color-mode"] {
                if let Some(value) = monitor.2.get(key) {
                    props.insert(
                        if key == "is-underscanning" {
                            "underscanning"
                        } else {
                            key
                        }
                        .into(),
                        value.try_clone()?,
                    );
                }
            }
            monitors.push((spec.0.clone(), mode.0.clone(), props));
        }
        config.push((
            *x,
            *y,
            if changing { requested.scale } else { *scale },
            *transform,
            *primary,
            monitors,
        ));
    }
    Ok(config)
}
impl DisplayController for MutterDisplay {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        command::blocking(async {
            tokio::time::timeout(Duration::from_secs(3), self.query())
                .await
                .context("Mutter query timed out")?
        })
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let bus = self.connection().await?;
                let proxy =
                    Proxy::new(&bus, SERVICE, "/org/gnome/Mutter/DisplayConfig", SERVICE).await?;
                ensure!(
                    proxy
                        .get_property::<bool>("ApplyMonitorsConfigAllowed")
                        .await?,
                    "Mutter denies display configuration"
                );
                self.query().await?;
                Ok::<_, anyhow::Error>(())
            })
            .await;
            match result {
                Ok(Ok(())) => DisplayCapabilities {
                    resize: true,
                    resize_scale: true,
                    reason: None,
                },
                Ok(Err(e)) => DisplayCapabilities::unavailable(format!("{e:#}")),
                Err(_) => DisplayCapabilities::unavailable("Mutter probe timed out"),
            }
        })
    }
    fn resize<'a>(
        &'a self,
        output: Option<&'a str>,
        mode: DisplayMode,
        fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(5), self.apply(output, mode, fps))
                .await
                .context("Mutter did not apply display mode before deadline")?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> State {
        let make = |name: &str, x: i32| {
            let spec = (
                name.to_string(),
                "Vendor".into(),
                "Panel".into(),
                name.to_string(),
            );
            let mut active = Props::new();
            active.insert("is-current".into(), OwnedValue::from(true));
            let modes = vec![
                ("old".into(), 1280, 720, 60.0, 1.0, vec![1.0, 2.0], active),
                (
                    "new".into(),
                    800,
                    600,
                    30.0,
                    1.0,
                    vec![1.0, 2.0],
                    Props::new(),
                ),
            ];
            (
                (spec.clone(), modes, Props::new()),
                (x, 0, 1.0, 0, x == 0, vec![spec], Props::new()),
            )
        };
        let (a, la) = make("DP-1", 0);
        let (b, lb) = make("DP-2", 1280);
        (42, vec![a, b], vec![la, lb], Props::new())
    }
    #[test]
    fn native_state_preserves_other_outputs_and_uses_supported_scaled_mode() {
        let state = fixture();
        let before = outputs(&state).unwrap();
        assert_eq!(before.len(), 2);
        assert_eq!(before[1].x, 1280);
        let config = configuration(
            &state,
            "DP-1",
            DisplayMode {
                width: 800,
                height: 600,
                scale: 2.0,
            },
            30,
        )
        .unwrap();
        assert_eq!(config[0].2, 2.0);
        assert_eq!(config[0].5[0].1, "new");
        assert!(config[0].4);
        assert_eq!(config[1].0, 1280);
        assert_eq!(config[1].2, 1.0);
        assert_eq!(config[1].5[0].1, "old");
        assert!(!config[1].4);
    }
    #[test]
    fn unsupported_mode_scale_mirroring_and_global_scale_are_explicit_errors() {
        let mut state = fixture();
        assert!(
            configuration(
                &state,
                "DP-1",
                DisplayMode {
                    width: 800,
                    height: 600,
                    scale: 1.5
                },
                30
            )
            .is_err()
        );
        assert!(
            configuration(
                &state,
                "DP-1",
                DisplayMode {
                    width: 900,
                    height: 600,
                    scale: 1.0
                },
                30
            )
            .is_err()
        );
        state
            .3
            .insert("global-scale-required".into(), OwnedValue::from(true));
        assert!(
            configuration(
                &state,
                "DP-1",
                DisplayMode {
                    width: 800,
                    height: 600,
                    scale: 2.0
                },
                30
            )
            .is_err()
        );
        state.3.clear();
        state.2[0].5.push(state.1[1].0.clone());
        assert!(
            configuration(
                &state,
                "DP-1",
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
    fn native_state_reports_rotation_and_scale_without_assuming_size() {
        let mut state = fixture();
        state.2[0].3 = 1;
        state.2[0].2 = 2.0;
        let output = &outputs(&state).unwrap()[0];
        assert_eq!(
            (output.width, output.height, output.scale),
            (720, 1280, 2.0)
        );
    }
    #[tokio::test]
    async fn missing_session_bus_disables_control_instead_of_using_host_bus() {
        let display = MutterDisplay::new(SessionTarget::default());
        let capabilities = display.capabilities().await;
        assert!(!capabilities.resize);
        assert!(capabilities.reason.unwrap().contains("selected session"));
    }
}
