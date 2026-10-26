#[cfg(target_os = "linux")]
mod applications;
pub mod audio;
pub mod audio_channel;
pub mod backend;
mod clipboard;
pub mod config;
mod cursor;
#[cfg(target_os = "linux")]
pub mod desktop;
#[cfg(target_os = "linux")]
pub mod diagnostics;
#[cfg(target_os = "linux")]
pub mod handoff;
#[cfg(target_os = "linux")]
mod input_method;
pub mod install;
pub mod mcp;
pub mod rdp;
pub mod server;
#[cfg(target_os = "linux")]
pub mod session;
pub mod video;
#[cfg(target_os = "linux")]
pub mod wayland;

pub mod platform;
pub mod private_storage;
