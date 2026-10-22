mod encoder;

use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use encoder::NativeEncoder;
use futures_util::{SinkExt, StreamExt};
use media_codec::{
    EncoderRate, default_bitrate,
    worker_protocol::{self as wire, Reply, Request},
};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    ffmpeg_next::init()?;
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Error);
    let codec = || {
        LengthDelimitedCodec::builder()
            .max_frame_length(wire::MAX_MESSAGE)
            .new_codec()
    };
    let mut input = FramedRead::new(tokio::io::stdin(), codec());
    let mut output = FramedWrite::new(tokio::io::stdout(), codec());
    let mut encoder: Option<NativeEncoder> = None;
    while let Some(bytes) = input.next().await {
        let request = wire::unpack(&bytes?)?;
        let result = handle(request, &mut encoder);
        let failed = result.is_err();
        let reply = result.unwrap_or_else(|error| Reply::Error(format!("{error:#}")));
        output.send(Bytes::from(wire::pack(&reply)?)).await?;
        if failed {
            break;
        }
    }
    Ok(())
}
fn handle(request: Request, encoder: &mut Option<NativeEncoder>) -> Result<Reply> {
    match request {
        Request::Probe { backend } => {
            ensure!(encoder.is_none(), "probe cannot replace an active encoder");
            Ok(Reply::Probe {
                codec: encoder::probe(backend)?,
            })
        }
        Request::Init { backend, settings } => {
            ensure!(encoder.is_none(), "encoder already initialized");
            let rate = EncoderRate {
                bitrate_bps: default_bitrate(settings.width, settings.height),
                fps: settings.fps,
            };
            *encoder = Some(NativeEncoder::new(backend, settings, rate)?);
            Ok(Reply::Ready)
        }
        Request::Frame { pixels, rate } => {
            let encoder = encoder.as_mut().context("encoder is not initialized")?;
            let encoded = encoder.encode(&pixels, rate)?;
            Ok(Reply::Frame {
                data: encoded.data,
                key: encoded.key,
                codec: encoded.codec,
            })
        }
    }
}
