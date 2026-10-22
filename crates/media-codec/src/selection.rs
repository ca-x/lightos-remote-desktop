use crate::{CodecCapability, VideoCodec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderBackend {
    #[serde(rename = "openh264")]
    OpenH264,
    #[serde(rename = "turbojpeg")]
    TurboJpeg,
    Vaapi,
    Nvenc,
    Amf,
    #[serde(rename = "videotoolbox")]
    VideoToolbox,
}
impl EncoderBackend {
    pub const HARDWARE: [Self; 4] = [Self::Nvenc, Self::Vaapi, Self::Amf, Self::VideoToolbox];
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenH264 => "openh264",
            Self::TurboJpeg => "turbojpeg",
            Self::Vaapi => "vaapi",
            Self::Nvenc => "nvenc",
            Self::Amf => "amf",
            Self::VideoToolbox => "videotoolbox",
        }
    }
    pub fn is_hardware(self) -> bool {
        !matches!(self, Self::OpenH264 | Self::TurboJpeg)
    }
    pub fn supported_platform(self) -> bool {
        match self {
            Self::Vaapi => cfg!(target_os = "linux"),
            Self::Nvenc | Self::Amf => cfg!(any(target_os = "linux", target_os = "windows")),
            Self::VideoToolbox => cfg!(target_os = "macos"),
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendPreference {
    #[default]
    Auto,
    Software,
    Vaapi,
    Nvenc,
    Amf,
    #[serde(rename = "videotoolbox")]
    VideoToolbox,
}
impl BackendPreference {
    pub fn requested_backend(self) -> Option<EncoderBackend> {
        match self {
            Self::Auto => None,
            Self::Software => Some(EncoderBackend::OpenH264),
            Self::Vaapi => Some(EncoderBackend::Vaapi),
            Self::Nvenc => Some(EncoderBackend::Nvenc),
            Self::Amf => Some(EncoderBackend::Amf),
            Self::VideoToolbox => Some(EncoderBackend::VideoToolbox),
        }
    }
}

pub fn valid_avc_codec(value: &str) -> bool {
    value.strip_prefix("avc1.").is_some_and(|profile| {
        profile.len() == 6 && profile.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

/// Only select complete successful probes. Exclusions are per connection, so a
/// failed driver or rejected browser profile is not retried in a fallback loop.
pub fn choose_h264_backend(
    preference: BackendPreference,
    capabilities: &[CodecCapability],
    excluded: &[EncoderBackend],
) -> Option<EncoderBackend> {
    let usable = |backend: EncoderBackend| {
        !excluded.contains(&backend)
            && capabilities.iter().any(|cap| {
                cap.backend == backend.id()
                    && cap.codec == VideoCodec::H264
                    && cap.hardware == backend.is_hardware()
                    && cap.available
                    && cap.reason.is_none()
                    && cap.codec_string.as_deref().is_some_and(valid_avc_codec)
            })
    };
    if let Some(backend) = preference.requested_backend() {
        if usable(backend) {
            return Some(backend);
        }
    } else if let Some(backend) = EncoderBackend::HARDWARE.into_iter().find(|&b| usable(b)) {
        return Some(backend);
    }
    usable(EncoderBackend::OpenH264).then_some(EncoderBackend::OpenH264)
}
