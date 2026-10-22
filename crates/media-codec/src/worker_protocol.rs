//! Local native-worker protocol. Both peers bound lengths before deserializing.
use crate::{EncoderBackend, EncoderRate, EncoderSettings};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const VERSION: u16 = 1;
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Probe {
        backend: EncoderBackend,
    },
    Init {
        backend: EncoderBackend,
        settings: EncoderSettings,
    },
    Frame {
        pixels: Vec<u8>,
        rate: EncoderRate,
    },
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Reply {
    Probe {
        codec: String,
    },
    Ready,
    Frame {
        data: Vec<u8>,
        key: bool,
        codec: Option<String>,
    },
    Error(String),
}

pub fn pack(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = bincode::serde::encode_to_vec((VERSION, value), config())?;
    ensure!(bytes.len() <= MAX_MESSAGE, "worker message too large");
    Ok(bytes)
}
pub fn unpack<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    ensure!(bytes.len() <= MAX_MESSAGE, "worker message too large");
    let ((version, value), read): ((u16, T), _) =
        bincode::serde::decode_from_slice(bytes, config())?;
    ensure!(version == VERSION, "incompatible codec worker protocol");
    ensure!(read == bytes.len(), "trailing codec worker data");
    Ok(value)
}
fn config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<MAX_MESSAGE>()
}
pub fn validate_settings(settings: EncoderSettings) -> Result<()> {
    ensure!(
        (320..=3840).contains(&settings.width) && settings.width.is_multiple_of(2),
        "invalid hardware width"
    );
    ensure!(
        (200..=2160).contains(&settings.height) && settings.height.is_multiple_of(2),
        "invalid hardware height"
    );
    ensure!((1..=60).contains(&settings.fps), "invalid hardware FPS");
    Ok(())
}
pub fn validate_pixels(settings: EncoderSettings, pixels: &[u8]) -> Result<()> {
    validate_settings(settings)?;
    ensure!(
        pixels.len() == settings.width as usize * settings.height as usize * 4,
        "invalid hardware pixel buffer"
    );
    Ok(())
}
