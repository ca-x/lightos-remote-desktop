//! Test fixture for a backend that was probed successfully but fails later.
//! Never package or install this executable as a real codec worker.
use anyhow::Result;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use media_codec::worker_protocol::{self as wire, Reply, Request};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[tokio::main]
async fn main() -> Result<()> {
    let codec = || {
        LengthDelimitedCodec::builder()
            .max_frame_length(wire::MAX_MESSAGE)
            .new_codec()
    };
    let mut input = FramedRead::new(tokio::io::stdin(), codec());
    let mut output = FramedWrite::new(tokio::io::stdout(), codec());
    while let Some(bytes) = input.next().await {
        let reply = match wire::unpack::<Request>(&bytes?)? {
            Request::Probe { .. } => Reply::Probe {
                codec: "avc1.42c01f".into(),
            },
            _ => Reply::Error("test fixture: driver disappeared after probe".into()),
        };
        output.send(Bytes::from(wire::pack(&reply)?)).await?;
    }
    Ok(())
}
