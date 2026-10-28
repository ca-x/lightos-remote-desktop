//! Resize an explicitly owned nested desktop through its session launcher.
use crate::{
    DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput, SessionTarget, command,
};
use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
use std::{path::PathBuf, sync::Arc, time::Duration};

pub(crate) struct NestedDisplay {
    primary: Arc<dyn DisplayController>,
    target: SessionTarget,
    helper: PathBuf,
    operation: tokio::sync::Mutex<()>,
}

impl NestedDisplay {
    pub(crate) fn new(
        primary: Arc<dyn DisplayController>,
        target: SessionTarget,
        helper: PathBuf,
    ) -> Self {
        Self {
            primary,
            target,
            helper,
            operation: tokio::sync::Mutex::new(()),
        }
    }

    async fn query(&self) -> Result<Vec<DisplayOutput>> {
        let primary = self.primary.clone();
        tokio::task::spawn_blocking(move || primary.outputs()).await?
    }

    async fn apply(&self, name: Option<&str>, mode: DisplayMode, fps: u32) -> Result<DisplayMode> {
        let _operation = self.operation.lock().await;
        mode.validate()?;
        ensure!(
            mode.scale == 1.0,
            "nested desktop resizing currently requires scale 1"
        );
        ensure!((1..=120).contains(&fps), "invalid refresh rate");
        ensure!(
            self.helper.is_absolute() && self.helper.is_file(),
            "nested display resize helper is unavailable"
        );
        let outputs = self.query().await?;
        ensure!(
            outputs.len() == 1,
            "nested resizing requires one owned output"
        );
        let selected = crate::select_output(&outputs, name)?;
        if crate::output_matches(selected, mode) {
            return Ok(mode);
        }
        let selected = selected.name.clone();
        command::run(
            &self.helper,
            &[
                mode.width.to_string(),
                mode.height.to_string(),
                fps.to_string(),
            ],
            &self.target,
        )
        .await?;
        loop {
            if self
                .query()
                .await?
                .iter()
                .any(|output| output.name == selected && crate::output_matches(output, mode))
            {
                return Ok(mode);
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}

impl DisplayController for NestedDisplay {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        self.primary.outputs()
    }

    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            match tokio::time::timeout(Duration::from_secs(3), self.query()).await {
                Ok(Ok(outputs))
                    if outputs.len() == 1 && self.helper.is_absolute() && self.helper.is_file() =>
                {
                    DisplayCapabilities {
                        resize: true,
                        resize_scale: false,
                        reason: None,
                    }
                }
                _ => DisplayCapabilities::unavailable(
                    "owned nested output or resize helper is unavailable",
                ),
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
                .context("nested desktop did not apply resolution before deadline")?
        })
    }
}
