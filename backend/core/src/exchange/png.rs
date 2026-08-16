//! PNG 角色卡:`chara` tEXt chunk 的读写。
//!
//! PNG 同时含 `chara` 与 `ccv3` 时只读 `chara`(计划 §6);写出时保留原图
//! 的图像数据,只替换文本 chunk。

use base64::Engine;

use crate::CoreError;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const CHARA: &[u8] = b"chara";

/// 取出 `chara` chunk 中的 CCv2 JSON。
pub fn read_card(png: &[u8]) -> Result<Vec<u8>, CoreError> {
    for (keyword, value) in text_chunks(png)? {
        if keyword == CHARA {
            return base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(|error| CoreError::InvalidExchangePayload {
                    reason: error.to_string(),
                });
        }
    }
    Err(CoreError::InvalidExchangePayload {
        reason: "png has no chara text chunk".to_owned(),
    })
}

/// 把角色卡 JSON 写进 PNG:替换既有 `chara`/`ccv3`,保留其余 chunk。
pub fn write_card(png: &[u8], card_json: &[u8]) -> Result<Vec<u8>, CoreError> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(card_json);
    let mut output = Vec::with_capacity(png.len() + encoded.len());
    output.extend_from_slice(&SIGNATURE);

    let mut inserted = false;
    for chunk in chunks(png)? {
        let kind = &png[chunk.start + 4..chunk.start + 8];
        if kind == b"tEXt" {
            let keyword = keyword_of(&png[chunk.start + 8..chunk.start + 8 + chunk.len]);
            // 既有卡数据整块丢弃,避免同一 PNG 里出现两份角色。
            if keyword == CHARA || keyword == b"ccv3" {
                continue;
            }
        }
        // 文本 chunk 必须在 IEND 之前。
        if kind == b"IEND" && !inserted {
            output.extend_from_slice(&text_chunk(CHARA, encoded.as_bytes()));
            inserted = true;
        }
        output.extend_from_slice(&png[chunk.start..chunk.start + 12 + chunk.len]);
    }
    if !inserted {
        return Err(CoreError::InvalidExchangePayload {
            reason: "png has no IEND chunk".to_owned(),
        });
    }
    Ok(output)
}

struct Chunk {
    start: usize,
    len: usize,
}

fn chunks(png: &[u8]) -> Result<Vec<Chunk>, CoreError> {
    if !png.starts_with(&SIGNATURE) {
        return Err(CoreError::InvalidExchangePayload {
            reason: "not a png".to_owned(),
        });
    }
    let mut chunks = Vec::new();
    let mut pos = SIGNATURE.len();
    while pos + 12 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().expect("4 bytes")) as usize;
        if pos + 12 + len > png.len() {
            return Err(CoreError::InvalidExchangePayload {
                reason: "truncated png chunk".to_owned(),
            });
        }
        chunks.push(Chunk { start: pos, len });
        pos += 12 + len;
    }
    Ok(chunks)
}

fn text_chunks(png: &[u8]) -> Result<Vec<(&[u8], &[u8])>, CoreError> {
    let mut found = Vec::new();
    for chunk in chunks(png)? {
        if &png[chunk.start + 4..chunk.start + 8] != b"tEXt" {
            continue;
        }
        let payload = &png[chunk.start + 8..chunk.start + 8 + chunk.len];
        if let Some(split) = payload.iter().position(|byte| *byte == 0) {
            found.push((&payload[..split], &payload[split + 1..]));
        }
    }
    Ok(found)
}

fn keyword_of(payload: &[u8]) -> &[u8] {
    match payload.iter().position(|byte| *byte == 0) {
        Some(split) => &payload[..split],
        None => payload,
    }
}

fn text_chunk(keyword: &[u8], value: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(keyword.len() + 1 + value.len());
    payload.extend_from_slice(keyword);
    payload.push(0);
    payload.extend_from_slice(value);

    let mut chunk = Vec::with_capacity(payload.len() + 12);
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(&payload);
    let mut crc_input = Vec::with_capacity(4 + payload.len());
    crc_input.extend_from_slice(b"tEXt");
    crc_input.extend_from_slice(&payload);
    chunk.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    chunk
}

/// PNG 的 CRC-32(IEEE),避免为一个 chunk 引入额外依赖。
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}
