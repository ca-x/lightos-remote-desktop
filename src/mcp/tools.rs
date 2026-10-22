use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{mpsc, watch};

use super::store::{Principal, Scope, TokenStore};
use crate::wayland::{Backend, Frame, InputCommand};

const DEADLINE: Duration = Duration::from_secs(5);
const MAX_FRAME_AGE: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub(super) struct Desktop {
    pub(super) frames: watch::Receiver<Option<Arc<Frame>>>,
    pub(super) input: mpsc::Sender<InputCommand>,
    pub(super) controller: Arc<AtomicBool>,
    pub(super) owned: bool,
    pub(super) release_input: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Desktop {
    pub(super) fn new(backend: Arc<Backend>, controller: Arc<AtomicBool>, owned: bool) -> Self {
        Self {
            frames: backend.frames.clone(),
            input: backend.input.clone(),
            controller,
            owned,
            release_input: Arc::new(move || {
                let released = backend.release_input();
                if !released {
                    backend.stop();
                }
                released
            }),
        }
    }

    fn frame(&self) -> Result<Arc<Frame>> {
        let frame = self
            .frames
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Desktop capture unavailable"))?;
        ensure!(
            frame.captured_at.elapsed() <= MAX_FRAME_AGE,
            "Desktop capture is stale; wait for a fresh frame"
        );
        ensure!(
            frame.width > 1 && frame.height > 1,
            "Desktop capture dimensions unavailable"
        );
        Ok(frame)
    }

    fn acquire(&self) -> Result<ControlLease> {
        ensure!(
            self.owned,
            "MCP control is restricted to the owned virtual desktop; physical desktops are read-only"
        );
        self.controller
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                anyhow::anyhow!(
                    "Desktop busy; Disconnect the browser controller before using MCP input"
                )
            })?;
        Ok(ControlLease(Some(self.clone())))
    }

    async fn send(&self, principal: &Principal, command: InputCommand) -> Result<()> {
        ensure!(
            !principal.cancellation.is_cancelled(),
            "MCP token disabled or revoked"
        );
        command.validate()?;
        self.input
            .send(command)
            .await
            .map_err(|_| anyhow::anyhow!("Desktop input unavailable"))
    }
}

// The gate stays held until the existing input worker has acknowledged releasing
// its keys/buttons (or has been stopped). Drop also handles canceled HTTP calls.
struct ControlLease(Option<Desktop>);
impl ControlLease {
    async fn release(mut self) -> Result<()> {
        if let Some(desktop) = self.0.take() {
            ensure!(
                tokio::task::spawn_blocking(move || release(desktop)).await?,
                "Desktop input cleanup was not acknowledged; the input worker was stopped"
            );
        }
        Ok(())
    }
}
fn release(desktop: Desktop) -> bool {
    let released = (desktop.release_input)();
    desktop.controller.store(false, Ordering::Release);
    released
}
impl Drop for ControlLease {
    fn drop(&mut self) {
        if let Some(desktop) = self.0.take() {
            tokio::task::spawn_blocking(move || release(desktop));
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Position {
    /// Horizontal whole pixel in the latest screenshot (0 <= x < width).
    x: u32,
    /// Vertical whole pixel in the latest screenshot (0 <= y < height).
    y: u32,
}
impl Position {
    fn command(&self, frame: &Frame) -> Result<InputCommand> {
        ensure!(
            self.x < frame.width && self.y < frame.height,
            "Position is outside the latest screenshot"
        );
        Ok(InputCommand::Move {
            x: self.x as f64 / (frame.width - 1) as f64,
            y: self.y as f64 / (frame.height - 1) as f64,
        })
    }
}

#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Button {
    #[default]
    Left,
    Middle,
    Right,
}
impl Button {
    fn code(self) -> u32 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}
fn one() -> u8 {
    1
}
fn drag_duration() -> u64 {
    250
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Click {
    x: u32,
    y: u32,
    #[serde(default)]
    button: Button,
    /// One click or a double click (1 or 2).
    #[serde(default = "one")]
    #[schemars(range(min = 1, max = 2))]
    count: u8,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Drag {
    from_x: u32,
    from_y: u32,
    to_x: u32,
    to_y: u32,
    #[serde(default)]
    button: Button,
    /// Drag duration in milliseconds, between 50 and 1000.
    #[serde(default = "drag_duration")]
    #[schemars(range(min = 50, max = 1000))]
    duration_ms: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Scroll {
    /// Horizontal Wayland scroll amount, -1000..1000; positive scrolls right.
    x: f64,
    /// Vertical Wayland scroll amount, -1000..1000; positive scrolls down.
    y: f64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KeyPress {
    /// Linux evdev key codes (1..767), modifiers first. At most eight distinct codes; e.g. Ctrl+C is [29, 46]. All are released after the chord.
    #[schemars(length(min = 1, max = 8))]
    codes: Vec<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Text {
    /// Unicode text, at most 16 KiB UTF-8. Newline, carriage return and tab are supported; other control characters are rejected.
    text: String,
}

#[derive(Clone)]
pub(super) struct DesktopTools {
    store: Arc<TokenStore>,
    desktop: Desktop,
}
impl DesktopTools {
    pub(super) fn new(store: Arc<TokenStore>, desktop: Desktop) -> Self {
        Self { store, desktop }
    }

    async fn run<F, Fut>(
        &self,
        context: RequestContext<RoleServer>,
        scope: Scope,
        action: F,
    ) -> Result<CallToolResult, ErrorData>
    where
        F: FnOnce(Principal) -> Fut,
        Fut: Future<Output = Result<CallToolResult>>,
    {
        let principal = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>())
            .cloned()
            .filter(|p| self.store.authorized(p, scope));
        let Some(principal) = principal else {
            return Ok(tool_error(
                "MCP token lacks the required scope or was disabled/revoked",
            ));
        };
        let cancellation = principal.cancellation.clone();
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Ok(tool_error("MCP token disabled or revoked")),
            _ = context.ct.cancelled() => Ok(tool_error("MCP request canceled")),
            result = tokio::time::timeout(DEADLINE, action(principal.clone())) => {
                if !self.store.authorized(&principal, scope) { return Ok(tool_error("MCP token disabled or revoked")); }
                Ok(match result {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => tool_error(&error.to_string()),
                    Err(_) => tool_error("Desktop operation exceeded the five-second deadline"),
                })
            }
        }
    }

    async fn commands(
        &self,
        principal: Principal,
        commands: Vec<InputCommand>,
    ) -> Result<CallToolResult> {
        for command in &commands {
            command.validate()?;
        }
        let lease = self.desktop.acquire()?;
        for command in commands {
            self.desktop.send(&principal, command).await?;
        }
        lease.release().await?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            "Input applied; all keys and buttons released.",
        )]))
    }
}
fn tool_error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

#[tool_router]
impl DesktopTools {
    #[tool(
        description = "Inspect the current capture size and desktop. Coordinates for all mouse tools are whole pixels of the screenshot, independent of browser zoom or desktop scale.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn desktop_info(
        &self,
        Parameters(_): Parameters<Empty>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Observe, |_| async {
            let frame = self.desktop.frame()?;
            Ok(CallToolResult::success(vec![ContentBlock::json(json!({
                "width":frame.width,"height":frame.height,"coordinate_space":"screenshot_pixels",
                "desktop":if self.desktop.owned {"virtual"} else {"physical"},
                "control_allowed":self.desktop.owned,
                "controller_busy":self.desktop.controller.load(Ordering::Acquire),
                "frame_age_ms":frame.captured_at.elapsed().as_millis() as u64,
                "image_format":"image/jpeg"
            }))?]))
        })
        .await
    }

    #[tool(
        description = "Take a fresh JPEG screenshot of the current remote desktop. Returns an MCP image and its pixel dimensions. Read-only; also works while the browser controls the desktop.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn screenshot(
        &self,
        Parameters(_): Parameters<Empty>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Observe, |_| async {
            let frame = self.desktop.frame()?;
            let (width, height) = (frame.width, frame.height);
            let image = tokio::task::spawn_blocking(move || frame.jpeg(85)).await??;
            Ok(CallToolResult::success(vec![
                ContentBlock::text(
                    json!({"width":width,"height":height,"coordinate_space":"screenshot_pixels"})
                        .to_string(),
                ),
                ContentBlock::image(STANDARD.encode(&*image), "image/jpeg"),
            ]))
        })
        .await
    }

    #[tool(
        description = "Move the pointer to a whole screenshot pixel. Requires control scope, an owned virtual desktop, and no browser controller.",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn mouse_move(
        &self,
        Parameters(position): Parameters<Position>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Control, |p| async move {
            self.commands(p, vec![position.command(self.desktop.frame()?.as_ref())?])
                .await
        })
        .await
    }

    #[tool(
        description = "Click a screenshot pixel with left, middle or right button. count=2 double-clicks. All buttons are released after this bounded action.",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn mouse_click(
        &self,
        Parameters(click): Parameters<Click>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Control, |p| async move {
            ensure!((1..=2).contains(&click.count), "Click count must be 1 or 2");
            let mut commands = vec![
                Position {
                    x: click.x,
                    y: click.y,
                }
                .command(self.desktop.frame()?.as_ref())?,
            ];
            for _ in 0..click.count {
                commands.extend([
                    InputCommand::Button {
                        button: click.button.code(),
                        pressed: true,
                    },
                    InputCommand::Button {
                        button: click.button.code(),
                        pressed: false,
                    },
                ]);
            }
            self.commands(p, commands).await
        })
        .await
    }

    #[tool(
        description = "Drag between two screenshot pixels over 50–1000 milliseconds. Holds one mouse button during movement, then releases it. Requires exclusive control of the owned virtual desktop.",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn mouse_drag(
        &self,
        Parameters(drag): Parameters<Drag>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Control, |p| async move {
            ensure!(
                (50..=1000).contains(&drag.duration_ms),
                "Drag duration must be 50–1000 milliseconds"
            );
            let frame = self.desktop.frame()?;
            let from = Position {
                x: drag.from_x,
                y: drag.from_y,
            }
            .command(&frame)?;
            let to = Position {
                x: drag.to_x,
                y: drag.to_y,
            }
            .command(&frame)?;
            let lease = self.desktop.acquire()?;
            self.desktop.send(&p, from).await?;
            self.desktop
                .send(
                    &p,
                    InputCommand::Button {
                        button: drag.button.code(),
                        pressed: true,
                    },
                )
                .await?;
            let InputCommand::Move { x, y } = to else {
                unreachable!()
            };
            let steps = (drag.duration_ms / 16).clamp(1, 32);
            for step in 1..=steps {
                tokio::time::sleep(Duration::from_millis(drag.duration_ms / steps)).await;
                let ratio = step as f64 / steps as f64;
                let from_x = drag.from_x as f64 / (frame.width - 1) as f64;
                let from_y = drag.from_y as f64 / (frame.height - 1) as f64;
                self.desktop
                    .send(
                        &p,
                        InputCommand::Move {
                            x: from_x + (x - from_x) * ratio,
                            y: from_y + (y - from_y) * ratio,
                        },
                    )
                    .await?;
            }
            self.desktop
                .send(
                    &p,
                    InputCommand::Button {
                        button: drag.button.code(),
                        pressed: false,
                    },
                )
                .await?;
            lease.release().await?;
            Ok(CallToolResult::success(vec![ContentBlock::text(
                "Drag applied; all keys and buttons released.",
            )]))
        })
        .await
    }

    #[tool(
        description = "Scroll the remote pointer. x/y use the existing Wayland axis units in -1000..1000; positive x scrolls right, positive y scrolls down.",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn scroll(
        &self,
        Parameters(scroll): Parameters<Scroll>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Control, |p| async move {
            self.commands(
                p,
                vec![InputCommand::Scroll {
                    x: scroll.x,
                    y: scroll.y,
                }],
            )
            .await
        })
        .await
    }

    #[tool(
        description = "Press and release a chord of 1–8 distinct Linux evdev codes in 1..767. Put modifiers first; Ctrl+C=[29,46], Enter=[28], Escape=[1], Super=[125]. Keys are released in reverse order.",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn key_press(
        &self,
        Parameters(keys): Parameters<KeyPress>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Control, |p| async move {
            ensure!(
                !keys.codes.is_empty() && keys.codes.len() <= 8,
                "Key chord must contain 1–8 codes"
            );
            ensure!(
                keys.codes
                    .iter()
                    .enumerate()
                    .all(|(i, k)| !keys.codes[..i].contains(k)),
                "Key chord contains duplicate codes"
            );
            let commands = keys
                .codes
                .iter()
                .map(|code| InputCommand::Key {
                    code: *code,
                    pressed: true,
                })
                .chain(keys.codes.iter().rev().map(|code| InputCommand::Key {
                    code: *code,
                    pressed: false,
                }))
                .collect();
            self.commands(p, commands).await
        })
        .await
    }

    #[tool(
        description = "Type Unicode text through the existing virtual keyboard path. Maximum 16 KiB UTF-8; newline/tab are allowed. This does not use a shell or the host clipboard.",
        annotations(destructive_hint = true, open_world_hint = true)
    )]
    async fn type_text(
        &self,
        Parameters(text): Parameters<Text>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(context, Scope::Control, |p| async move {
            self.commands(p, vec![InputCommand::Text { text: text.text }])
                .await
        })
        .await
    }
}

#[tool_handler]
impl ServerHandler for DesktopTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lightos-remote-desktop", env!("CARGO_PKG_VERSION")))
            .with_instructions("Observe the screenshot before acting. Mouse coordinates are screenshot pixels. Input requires a control token and exclusive access to the owned virtual desktop; Disconnect the browser to hand over input. Physical desktops are read-only. This server has no shell or agent inference runtime.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_worker_ack_is_reported_as_a_tool_failure() {
        let (_captures, frames) = watch::channel(None);
        let (input, _commands) = mpsc::channel(4);
        let controller = Arc::new(AtomicBool::new(false));
        let desktop = Desktop {
            frames,
            input,
            controller: controller.clone(),
            owned: true,
            release_input: Arc::new(|| false),
        };
        let directory = tempfile::tempdir().unwrap();
        let tools = DesktopTools::new(
            Arc::new(TokenStore::open(Some(directory.path().join("mcp.json")))),
            desktop,
        );
        let result = tools
            .commands(
                Principal {
                    id: "test".into(),
                    scope: Scope::Control,
                    cancellation: tokio_util::sync::CancellationToken::new(),
                },
                vec![InputCommand::Key {
                    code: 28,
                    pressed: true,
                }],
            )
            .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cleanup was not acknowledged")
        );
        assert!(!controller.load(Ordering::Acquire));
    }
}
