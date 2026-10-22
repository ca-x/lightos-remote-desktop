use crate::{DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput};
use anyhow::{Result, bail};
use futures_util::future::BoxFuture;
use libwayshot::WayshotConnection;
use std::{
    net::Shutdown,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};
use wayland_client::{Connection, protocol::wl_output};

#[derive(Clone)]
pub struct WaylandDisplay {
    socket: PathBuf,
}
impl WaylandDisplay {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
}
impl DisplayController for WaylandDisplay {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        display_outputs(&self.socket)
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            DisplayCapabilities::unavailable(
                "resize requires an owned desktop session with a supported compositor",
            )
        })
    }
    fn resize<'a>(
        &'a self,
        _output: Option<&'a str>,
        _resolution: DisplayMode,
        _fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        Box::pin(async {
            bail!("resize requires an owned desktop session with a supported compositor")
        })
    }
}

fn display_outputs(socket: &Path) -> Result<Vec<DisplayOutput>> {
    with_connection(socket, |connection| {
        let capture = WayshotConnection::from_connection(connection)?;
        let outputs = capture
            .get_all_outputs()
            .iter()
            .map(|output| {
                let size = output.physical_size();
                let logical = output.logical_size();
                let position = output.logical_position();
                let (width, height) =
                    oriented_dimensions(size.width, size.height, output.transform);
                DisplayOutput {
                    name: output.name.clone(),
                    description: output.description.clone(),
                    width,
                    height,
                    x: position.x,
                    y: position.y,
                    scale: f64::from(height) / f64::from(logical.height.max(1)),
                }
            })
            .collect();
        Ok(outputs)
    })
}

pub(crate) fn with_connection<T>(
    socket: &Path,
    query: impl FnOnce(Connection) -> Result<T>,
) -> Result<T> {
    let stream = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?
        .block_on(async {
            tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::UnixStream::connect(socket),
            )
            .await?
        })?
        .into_std()?;
    stream.set_nonblocking(false)?;
    let interrupt = stream.try_clone()?;
    let (done, finished) = mpsc::channel();
    let watchdog = thread::spawn(move || {
        if finished.recv_timeout(Duration::from_secs(3)).is_err() {
            let _ = interrupt.shutdown(Shutdown::Both);
        }
    });
    let result = Connection::from_socket(stream)
        .map_err(anyhow::Error::from)
        .and_then(query);
    let _ = done.send(());
    let _ = watchdog.join();
    result
}

fn oriented_dimensions(width: u32, height: u32, transform: wl_output::Transform) -> (u32, u32) {
    match transform {
        wl_output::Transform::_90
        | wl_output::Transform::_270
        | wl_output::Transform::Flipped90
        | wl_output::Transform::Flipped270 => (height, width),
        _ => (width, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_outputs_report_oriented_pixel_dimensions() {
        for transform in [
            wl_output::Transform::_90,
            wl_output::Transform::_270,
            wl_output::Transform::Flipped90,
            wl_output::Transform::Flipped270,
        ] {
            assert_eq!(oriented_dimensions(1920, 1080, transform), (1080, 1920));
        }
        for transform in [
            wl_output::Transform::Normal,
            wl_output::Transform::_180,
            wl_output::Transform::Flipped,
            wl_output::Transform::Flipped180,
        ] {
            assert_eq!(oriented_dimensions(1920, 1080, transform), (1920, 1080));
        }
    }
}
