//! Opt-in wire experiment; encoded video bytes are never serialized again.
use anyhow::{Result, bail, ensure};
use prost::Message;

use crate::backend::InputCommand;

pub const FRAMES_PROTOCOL: &str = "lightos.frames.v1";
pub const PROTOBUF_PROTOCOL: &str = "lightos.protobuf.v1";
pub const MAX_BINARY_COMMAND: usize = 128;
const MAX_SAFE_ID: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WireMode {
    #[default]
    Legacy,
    Frames,
    Protobuf,
}
impl WireMode {
    pub fn negotiated(protocol: Option<&str>) -> Self {
        match protocol {
            Some(FRAMES_PROTOCOL) => Self::Frames,
            Some(PROTOBUF_PROTOCOL) => Self::Protobuf,
            _ => Self::Legacy,
        }
    }
    pub fn inline_ids(self) -> bool {
        self != Self::Legacy
    }
    pub fn jpeg(self, id: u64, payload: &[u8]) -> Vec<u8> {
        let mut packet = Vec::with_capacity(payload.len() + if self.inline_ids() { 9 } else { 1 });
        packet.push(if self.inline_ids() { 5 } else { 1 });
        if self.inline_ids() {
            packet.extend_from_slice(&id.to_le_bytes());
        }
        packet.extend_from_slice(payload);
        packet
    }
    pub fn h264(self, id: u64, key: bool, timestamp: u64, payload: &[u8]) -> Vec<u8> {
        let mut packet =
            Vec::with_capacity(payload.len() + if self.inline_ids() { 18 } else { 10 });
        packet.extend_from_slice(&[if self.inline_ids() { 6 } else { 3 }, u8::from(key)]);
        packet.extend_from_slice(&timestamp.to_le_bytes());
        if self.inline_ids() {
            packet.extend_from_slice(&id.to_le_bytes());
        }
        packet.extend_from_slice(payload);
        packet
    }
}

// Mirrors proto/wire.proto. The small schema is shared with generated browser
// code; cross-language golden vectors pin the field numbers and numeric types.
#[derive(Clone, PartialEq, Message)]
pub struct Command {
    #[prost(oneof = "command::Kind", tags = "1, 2, 3, 4, 5, 6")]
    pub kind: Option<command::Kind>,
}
pub mod command {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        #[prost(uint64, tag = "1")]
        FrameAck(u64),
        #[prost(message, tag = "2")]
        Move(super::Point),
        #[prost(message, tag = "3")]
        Button(super::Button),
        #[prost(message, tag = "4")]
        Key(super::Key),
        #[prost(message, tag = "5")]
        Scroll(super::Point),
        #[prost(bool, tag = "6")]
        Release(bool),
    }
}
#[derive(Clone, PartialEq, Message)]
pub struct Point {
    #[prost(double, tag = "1")]
    pub x: f64,
    #[prost(double, tag = "2")]
    pub y: f64,
}
#[derive(Clone, PartialEq, Message)]
pub struct Button {
    #[prost(uint32, tag = "1")]
    pub button: u32,
    #[prost(bool, tag = "2")]
    pub pressed: bool,
}
#[derive(Clone, PartialEq, Message)]
pub struct Key {
    #[prost(uint32, tag = "1")]
    pub code: u32,
    #[prost(bool, tag = "2")]
    pub pressed: bool,
}

#[derive(Debug)]
pub enum DecodedCommand {
    Ack(u64),
    Input(InputCommand),
}

pub fn decode_command(bytes: &[u8]) -> Result<DecodedCommand> {
    ensure!(
        bytes.len() <= MAX_BINARY_COMMAND,
        "binary command too large"
    );
    let command = Command::decode(bytes)?;
    use command::Kind;
    let input = match command.kind {
        Some(Kind::FrameAck(id)) => {
            ensure!((1..=MAX_SAFE_ID).contains(&id), "invalid ACK id");
            return Ok(DecodedCommand::Ack(id));
        }
        Some(Kind::Move(Point { x, y })) => InputCommand::Move { x, y },
        Some(Kind::Button(Button { button, pressed })) => InputCommand::Button { button, pressed },
        Some(Kind::Key(Key { code, pressed })) => InputCommand::Key { code, pressed },
        Some(Kind::Scroll(Point { x, y })) => InputCommand::Scroll { x, y },
        Some(Kind::Release(true)) => InputCommand::Release,
        _ => bail!("missing or unsupported binary command"),
    };
    input.validate()?;
    Ok(DecodedCommand::Input(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn browser_and_rust_agree_on_golden_commands() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/fixtures/wire-v1.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let hex = case["hex"].as_str().unwrap();
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let decoded = match decode_command(&bytes).unwrap() {
                DecodedCommand::Ack(id) => json!({"type":"frame_ack","id":id}),
                DecodedCommand::Input(input) => serde_json::to_value(input).unwrap(),
            };
            let expected = if case["command"]["type"] == "frame_ack" {
                case["command"].clone()
            } else {
                serde_json::to_value(
                    serde_json::from_value::<InputCommand>(case["command"].clone()).unwrap(),
                )
                .unwrap()
            };
            assert_eq!(decoded, expected);
            assert_eq!(
                Command::decode(bytes.as_slice()).unwrap().encode_to_vec(),
                bytes
            );
        }
    }

    #[test]
    fn malformed_or_invalid_binary_commands_are_rejected() {
        for bytes in [
            vec![],
            vec![255],
            vec![18, 18, 9],
            vec![48, 0],
            vec![0; MAX_BINARY_COMMAND + 1],
        ] {
            assert!(decode_command(&bytes).is_err());
        }
        use command::Kind;
        for kind in [
            Kind::FrameAck(0),
            Kind::FrameAck(MAX_SAFE_ID + 1),
            Kind::Move(Point {
                x: f64::NAN,
                y: 0.0,
            }),
            Kind::Move(Point { x: 1.01, y: 0.0 }),
            Kind::Scroll(Point { x: 1001.0, y: 0.0 }),
            Kind::Key(Key {
                code: 0,
                pressed: true,
            }),
            Kind::Key(Key {
                code: 768,
                pressed: false,
            }),
            Kind::Button(Button {
                button: 3,
                pressed: false,
            }),
        ] {
            assert!(decode_command(&Command { kind: Some(kind) }.encode_to_vec()).is_err());
        }
    }

    #[test]
    fn inline_ids_keep_the_compressed_payload_identical() {
        let payload = [0, 0, 1, 101, 0, 255, 42];
        for mode in [WireMode::Legacy, WireMode::Frames, WireMode::Protobuf] {
            let jpeg = mode.jpeg(1000, &payload);
            let h264 = mode.h264(1000, true, 42, &payload);
            let (j, h) = if mode.inline_ids() { (9, 18) } else { (1, 10) };
            assert_eq!(&jpeg[j..], payload);
            assert_eq!(&h264[h..], payload);
            if mode.inline_ids() {
                assert_eq!(&jpeg[1..9], &1000_u64.to_le_bytes());
                assert_eq!(&h264[10..18], &1000_u64.to_le_bytes());
            }
            assert_eq!(&h264[2..10], &42_u64.to_le_bytes());
        }
        assert_eq!(WireMode::negotiated(None), WireMode::Legacy);
        assert_eq!(WireMode::negotiated(Some("other")), WireMode::Legacy);
    }
}
