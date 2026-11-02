//! The two public unary Shell API calls, framed as grpc-web protobuf.
use anyhow::{Result, bail, ensure};

pub fn field(buffer: &mut Vec<u8>, number: u8, bytes: &[u8]) {
    buffer.push(number << 3 | 2);
    let mut size = bytes.len();
    while size >= 128 {
        buffer.push((size as u8) | 128);
        size >>= 7;
    }
    buffer.push(size as u8);
    buffer.extend_from_slice(bytes);
}

fn varint(bytes: &[u8], cursor: &mut usize) -> Result<usize> {
    let mut value = 0usize;
    for shift in (0..63).step_by(7) {
        let byte = *bytes
            .get(*cursor)
            .ok_or_else(|| anyhow::anyhow!("invalid protobuf"))?;
        *cursor += 1;
        value |= ((byte & 127) as usize) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    bail!("invalid protobuf integer")
}

pub fn string(bytes: &[u8], number: usize) -> Result<String> {
    let mut cursor = 0;
    while cursor < bytes.len() {
        let tag = varint(bytes, &mut cursor)?;
        match tag & 7 {
            0 => {
                varint(bytes, &mut cursor)?;
            }
            2 => {
                let length = varint(bytes, &mut cursor)?;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("invalid protobuf size"))?;
                if tag >> 3 == number {
                    return Ok(String::from_utf8(bytes[cursor..end].to_vec())?);
                }
                cursor = end;
            }
            1 => cursor += 8,
            5 => cursor += 4,
            _ => bail!("unsupported protobuf wire type"),
        }
    }
    bail!("missing protobuf field")
}

pub async fn call(
    client: &reqwest::Client,
    address: &str,
    credential: &str,
    method: &str,
    message: Vec<u8>,
) -> Result<Vec<u8>> {
    let mut body = vec![0];
    body.extend_from_slice(&(message.len() as u32).to_be_bytes());
    body.extend(message);
    let reply = client
        .post(format!(
            "http://{address}/space.heiyu.hportal.shell.{method}"
        ))
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .header("lzc-shellapi-cred", credential)
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        reply.content_length().unwrap_or(0) <= 1024 * 1024,
        "shell response too large"
    );
    let bytes = reply.bytes().await?;
    let mut cursor = 0;
    let mut message = None;
    let mut successful = false;
    while cursor + 5 <= bytes.len() {
        let flags = bytes[cursor];
        let length = u32::from_be_bytes(bytes[cursor + 1..cursor + 5].try_into()?) as usize;
        cursor += 5;
        ensure!(length <= bytes.len() - cursor, "invalid grpc frame");
        let frame = &bytes[cursor..cursor + length];
        if flags & 128 != 0 {
            successful = std::str::from_utf8(frame)?
                .lines()
                .any(|line| line.trim_end_matches('\r') == "grpc-status: 0");
        } else {
            message = Some(frame.to_vec());
        }
        cursor += length;
    }
    ensure!(
        successful && cursor == bytes.len(),
        "shell API request failed"
    );
    Ok(message.unwrap_or_default())
}
