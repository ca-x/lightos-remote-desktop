//! Local native-worker protocol. Both peers bound lengths before deserializing.
use crate::{EncoderBackend, EncoderRate, EncoderSettings};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[cfg(target_os = "linux")]
pub mod shared_frame;

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(
    serialize = "P: serde_bytes::Serialize",
    deserialize = "P: serde_bytes::Deserialize<'de>"
))]
pub enum RequestPayload<P> {
    Probe {
        backend: EncoderBackend,
    },
    Init {
        backend: EncoderBackend,
        settings: EncoderSettings,
    },
    Frame {
        #[serde(with = "serde_bytes")]
        pixels: P,
        rate: EncoderRate,
    },
}
pub type Request = RequestPayload<Vec<u8>>;
pub type RequestRef<'a> = RequestPayload<&'a [u8]>;

#[derive(Debug, Serialize, Deserialize)]
pub enum Reply {
    Probe {
        codec: String,
    },
    Ready,
    Frame {
        #[serde(with = "serde_bytes")]
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
pub fn unpack<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T> {
    ensure!(bytes.len() <= MAX_MESSAGE, "worker message too large");
    let ((version, value), read): ((u16, T), _) =
        bincode::serde::borrow_decode_from_slice(bytes, config())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_reply_is_compatible_with_legacy_worker_in_both_directions() {
        #[derive(Debug, Serialize, Deserialize)]
        enum LegacyReply {
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
        let data: Vec<u8> = (0..=255).collect();
        let codec = Some("avc1.42E01E".to_owned());
        let legacy = pack(&LegacyReply::Frame {
            data: data.clone(),
            key: true,
            codec: codec.clone(),
        })
        .unwrap();
        let current = pack(&Reply::Frame {
            data: data.clone(),
            key: true,
            codec: codec.clone(),
        })
        .unwrap();
        assert_eq!(legacy, current);
        assert!(matches!(unpack::<Reply>(&legacy).unwrap(),
            Reply::Frame { data: decoded, key: true, codec: profile }
                if decoded == data && profile == codec));
        assert!(matches!(unpack::<LegacyReply>(&current).unwrap(),
            LegacyReply::Frame { data: decoded, key: true, codec: profile }
                if decoded == data && profile == codec));
    }

    #[test]
    fn borrowed_frame_preserves_v1_wire_format_and_input() {
        let pixels = vec![37; 640 * 360 * 4];
        let rate = EncoderRate {
            bitrate_bps: 1_000_000,
            fps: 30,
        };
        let borrowed = pack(&RequestRef::Frame {
            pixels: &pixels,
            rate,
        })
        .unwrap();
        #[derive(Serialize, Deserialize)]
        enum LegacyRequest {
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
        let owned = pack(&LegacyRequest::Frame {
            pixels: pixels.clone(),
            rate,
        })
        .unwrap();
        assert_eq!(borrowed, owned);
        let old: LegacyRequest = unpack(&borrowed).unwrap();
        assert!(matches!(old, LegacyRequest::Frame { pixels: old, .. } if old == pixels));
        let Request::Frame {
            pixels: decoded,
            rate: actual,
        } = unpack(&borrowed).unwrap()
        else {
            panic!("expected frame")
        };
        assert_eq!(decoded, pixels);
        assert_eq!(actual, rate);
        assert!(pixels.iter().all(|&p| p == 37));
        for request in [
            RequestRef::Probe {
                backend: EncoderBackend::Vaapi,
            },
            RequestRef::Init {
                backend: EncoderBackend::Nvenc,
                settings: EncoderSettings {
                    width: 640,
                    height: 360,
                    fps: 30,
                    quality: 75,
                },
            },
        ] {
            let decoded: Request = unpack(&pack(&request).unwrap()).unwrap();
            assert_eq!(pack(&request).unwrap(), pack(&decoded).unwrap());
        }
    }
}
