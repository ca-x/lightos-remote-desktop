mod encoder;

use anyhow::{Context, Result, ensure};
use encoder::NativeEncoder;
use media_codec::{
    EncoderRate, default_bitrate,
    worker_protocol::{self as wire, Reply, RequestRef as Request},
};
use std::io::{self, Read, Write};

fn main() -> Result<()> {
    ffmpeg_next::init()?;
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Error);
    // One caller, one outstanding request: receive directly into reusable
    // storage instead of copying full RGBA frames through async stdio buffers.
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut buffer = Vec::new();
    #[cfg(target_os = "linux")]
    let shared = wire::shared_frame::inherited()?;
    #[cfg(target_os = "linux")]
    let shared_pixels = shared.as_deref();
    #[cfg(not(target_os = "linux"))]
    let shared_pixels = None;
    let mut encoder: Option<NativeEncoder> = None;
    loop {
        let mut header = [0; 4];
        if input.read(&mut header[..1])? == 0 {
            break;
        }
        input.read_exact(&mut header[1..])?;
        let length = u32::from_be_bytes(header) as usize;
        ensure!(length <= wire::MAX_MESSAGE, "worker message too large");
        buffer.resize(length, 0);
        input.read_exact(&mut buffer)?;
        let request = wire::unpack(&buffer)?;
        let result = handle(request, &mut encoder, shared_pixels);
        let failed = result.is_err();
        let reply = result.unwrap_or_else(|error| Reply::Error(format!("{error:#}")));
        let bytes = wire::pack(&reply)?;
        output.write_all(&(bytes.len() as u32).to_be_bytes())?;
        output.write_all(&bytes)?;
        output.flush()?;
        if failed {
            break;
        }
    }
    Ok(())
}
fn handle(
    request: Request<'_>,
    encoder: &mut Option<NativeEncoder>,
    shared: Option<&[u8]>,
) -> Result<Reply> {
    let (request, device) = match request {
        Request::ProbeDevice { device } => {
            device.validate()?;
            (
                Request::Probe {
                    backend: device.backend(),
                },
                Some(device),
            )
        }
        Request::InitDevice { device, settings } => {
            device.validate()?;
            (
                Request::Init {
                    backend: device.backend(),
                    settings,
                },
                Some(device),
            )
        }
        request => (request, None),
    };
    match request {
        Request::Probe { backend } => {
            ensure!(encoder.is_none(), "probe cannot replace an active encoder");
            Ok(Reply::Probe {
                codec: encoder::probe(backend, device.as_ref())?,
            })
        }
        Request::Init { backend, settings } => {
            ensure!(encoder.is_none(), "encoder already initialized");
            let rate = EncoderRate {
                bitrate_bps: default_bitrate(settings.width, settings.height),
                fps: settings.fps,
            };
            *encoder = Some(NativeEncoder::new(
                backend,
                settings,
                rate,
                device.as_ref(),
            )?);
            #[cfg(target_os = "linux")]
            if shared.is_some()
                && std::env::var_os(wire::shared_frame::NEGOTIATE_ENV).as_deref()
                    == Some(std::ffi::OsStr::new("1"))
            {
                return Ok(Reply::ReadySharedFrame);
            }
            Ok(Reply::Ready)
        }
        Request::Frame { pixels, rate } => {
            let encoder = encoder.as_mut().context("encoder is not initialized")?;
            let pixels = if pixels.is_empty() {
                shared.context("missing shared codec frame")?
            } else {
                pixels
            };
            let encoded = encoder.encode(pixels, rate)?;
            Ok(Reply::Frame {
                data: encoded.data,
                key: encoded.key,
                codec: encoded.codec,
            })
        }
        Request::ProbeDevice { .. } | Request::InitDevice { .. } => {
            unreachable!("device request normalized")
        }
    }
}
