use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

pub const MAGIC: &[u8; 4] = b"JRVS";
pub const PROTOCOL_VERSION: u16 = 2;

pub const MSG_TYPE_CONTROL_JSON: u16 = 0x0001;
pub const MSG_TYPE_AUDIO_PCM: u16 = 0x0002;

pub const HEADER_LEN: usize = 16;
pub const MAX_CONTROL_JSON_SIZE: usize = 256 * 1024; // 256 KiB
pub const MAX_AUDIO_PCM_SIZE: usize = 64 * 1024;     // 64 KiB

pub fn max_payload_for_type(msg_type: u16) -> usize {
    match msg_type {
        MSG_TYPE_CONTROL_JSON => MAX_CONTROL_JSON_SIZE,
        MSG_TYPE_AUDIO_PCM => MAX_AUDIO_PCM_SIZE,
        _ => 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcFrame {
    pub msg_type: u16,
    pub turn_id: u32,
    pub payload: Bytes,
}

impl IpcFrame {
    #[allow(dead_code)]
    pub fn control_json(payload: impl Into<Bytes>, turn_id: u32) -> Self {
        Self {
            msg_type: MSG_TYPE_CONTROL_JSON,
            turn_id,
            payload: payload.into(),
        }
    }

    #[allow(dead_code)]
    pub fn audio_pcm(payload: impl Into<Bytes>, turn_id: u32) -> Self {
        Self {
            msg_type: MSG_TYPE_AUDIO_PCM,
            turn_id,
            payload: payload.into(),
        }
    }
}

#[derive(Default)]
pub struct JarvisFrameCodec;

impl Decoder for JarvisFrameCodec {
    type Item = IpcFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let header = &src[..HEADER_LEN];

        // 1. Validate 4-byte ASCII magic "JRVS"
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&header[0..4]);
        if &magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid IPC header magic: {:?}", magic),
            ));
        }

        // 2. Validate Protocol Version
        let version = (&header[4..6]).get_u16_le();
        if version != PROTOCOL_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported protocol version: {version} (expected {PROTOCOL_VERSION})"),
            ));
        }

        // 3. Validate Message Type
        let msg_type = (&header[6..8]).get_u16_le();
        let max_allowed = max_payload_for_type(msg_type);
        if max_allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown message type: 0x{:04x}", msg_type),
            ));
        }

        // 4. Extract Turn ID
        let turn_id = (&header[8..12]).get_u32_le();

        // 5. Validate Payload Length BEFORE allocating or reserving memory
        let payload_len = (&header[12..16]).get_u32_le() as usize;
        if payload_len > max_allowed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "IPC payload length {payload_len} exceeds max {max_allowed} for type 0x{msg_type:04x}"
                ),
            ));
        }

        // Wait until full frame is available in the buffer
        if src.len() < HEADER_LEN + payload_len {
            src.reserve(HEADER_LEN + payload_len - src.len());
            return Ok(None);
        }

        src.advance(HEADER_LEN);
        let payload = src.split_to(payload_len).freeze();

        Ok(Some(IpcFrame {
            msg_type,
            turn_id,
            payload,
        }))
    }
}

impl Encoder<IpcFrame> for JarvisFrameCodec {
    type Error = io::Error;

    fn encode(&mut self, item: IpcFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let max_allowed = max_payload_for_type(item.msg_type);
        if max_allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Unknown IPC message type: 0x{:04x}", item.msg_type),
            ));
        }
        if item.payload.len() > max_allowed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Payload length {} exceeds max {} for type 0x{:04x}",
                    item.payload.len(),
                    max_allowed,
                    item.msg_type
                ),
            ));
        }
        let len: u32 = item.payload.len().try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "IPC payload exceeds u32::MAX")
        })?;

        dst.reserve(HEADER_LEN + item.payload.len());
        dst.extend_from_slice(MAGIC);
        dst.put_u16_le(PROTOCOL_VERSION);
        dst.put_u16_le(item.msg_type);
        dst.put_u32_le(item.turn_id);
        dst.put_u32_le(len);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_v2_round_trip() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        let original_frame = IpcFrame {
            msg_type: MSG_TYPE_CONTROL_JSON,
            turn_id: 42,
            payload: Bytes::from_static(b"{\"ping\":true}"),
        };

        codec.encode(original_frame.clone(), &mut buffer).unwrap();
        assert_eq!(buffer.len(), 16 + b"{\"ping\":true}".len());
        assert_eq!(&buffer[0..4], b"JRVS");

        let decoded = codec.decode(&mut buffer).unwrap().expect("frame decoded");
        assert_eq!(decoded, original_frame);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_codec_bad_magic() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        let frame = IpcFrame {
            msg_type: MSG_TYPE_CONTROL_JSON,
            turn_id: 1,
            payload: Bytes::from_static(b"{}"),
        };
        codec.encode(frame, &mut buffer).unwrap();

        // Corrupt magic
        buffer[0] = b'X';

        let err = codec.decode(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("Invalid IPC header magic"));
    }

    #[test]
    fn test_codec_wrong_version() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        let frame = IpcFrame {
            msg_type: MSG_TYPE_CONTROL_JSON,
            turn_id: 1,
            payload: Bytes::from_static(b"{}"),
        };
        codec.encode(frame, &mut buffer).unwrap();

        // Alter version to 3
        buffer[4] = 3;
        buffer[5] = 0;

        let err = codec.decode(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("Unsupported protocol version"));
    }

    #[test]
    fn test_codec_unknown_msg_type() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        let frame = IpcFrame {
            msg_type: MSG_TYPE_CONTROL_JSON,
            turn_id: 1,
            payload: Bytes::from_static(b"{}"),
        };
        codec.encode(frame, &mut buffer).unwrap();

        // Alter msg_type to 0x00FF
        buffer[6] = 0xFF;
        buffer[7] = 0x00;

        let err = codec.decode(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("Unknown message type"));
    }

    #[test]
    fn test_codec_oversized_json() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        // Header claiming 256 KiB + 1 bytes payload
        buffer.extend_from_slice(MAGIC);
        buffer.put_u16_le(PROTOCOL_VERSION);
        buffer.put_u16_le(MSG_TYPE_CONTROL_JSON);
        buffer.put_u32_le(10);
        buffer.put_u32_le((MAX_CONTROL_JSON_SIZE + 1) as u32);

        let err = codec.decode(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("exceeds max 262144"));
    }

    #[test]
    fn test_codec_oversized_pcm() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        // Header claiming 64 KiB + 1 bytes payload
        buffer.extend_from_slice(MAGIC);
        buffer.put_u16_le(PROTOCOL_VERSION);
        buffer.put_u16_le(MSG_TYPE_AUDIO_PCM);
        buffer.put_u32_le(10);
        buffer.put_u32_le((MAX_AUDIO_PCM_SIZE + 1) as u32);

        let err = codec.decode(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("exceeds max 65536"));
    }

    #[test]
    fn test_codec_u32_max_no_oom() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        // Header with 0xFFFFFFFF payload_len
        buffer.extend_from_slice(MAGIC);
        buffer.put_u16_le(PROTOCOL_VERSION);
        buffer.put_u16_le(MSG_TYPE_CONTROL_JSON);
        buffer.put_u32_le(0);
        buffer.put_u32_le(0xFFFFFFFF);

        // Verification: Must reject immediately and NOT panic or allocate ~4 GiB
        let err = codec.decode(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("exceeds max"));
    }

    #[test]
    fn test_codec_partial_reads() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::new();

        let frame = IpcFrame {
            msg_type: MSG_TYPE_AUDIO_PCM,
            turn_id: 100,
            payload: Bytes::from_static(b"0123456789abcdef"),
        };
        codec.encode(frame.clone(), &mut buffer).unwrap();

        // Feed 1 byte at a time until full frame is reached
        let total_bytes = buffer.len();
        let mut stream_buf = BytesMut::new();

        for i in 0..total_bytes - 1 {
            stream_buf.put_u8(buffer[i]);
            assert!(
                codec.decode(&mut stream_buf).unwrap().is_none(),
                "Premature decode at byte {i}"
            );
        }

        // Put last byte
        stream_buf.put_u8(buffer[total_bytes - 1]);
        let decoded = codec.decode(&mut stream_buf).unwrap().expect("full frame decoded");
        assert_eq!(decoded, frame);
        assert!(stream_buf.is_empty());
    }

    #[test]
    fn test_live_operational_framing_correctness_1000_frames() {
        let mut codec = JarvisFrameCodec;
        let mut buffer = BytesMut::with_capacity(65536);

        for i in 0..1000 {
            let payload = format!("{{\"seq\": {}, \"status\": \"operational\"}}", i);
            let frame = IpcFrame {
                msg_type: MSG_TYPE_CONTROL_JSON,
                turn_id: i,
                payload: Bytes::from(payload),
            };
            // 1. Successful encoding
            codec.encode(frame.clone(), &mut buffer).expect("encoding must succeed");

            // 2. Successful decoding
            let decoded = codec.decode(&mut buffer).expect("decoding must succeed").expect("must decode full frame");

            // 3. Verify Message Type
            assert_eq!(decoded.msg_type, MSG_TYPE_CONTROL_JSON, "message type must match MSG_TYPE_CONTROL_JSON");

            // 4. Verify Turn ID
            assert_eq!(decoded.turn_id, i, "turn ID must match encoded turn ID");

            // 5. Verify Complete Payload
            assert_eq!(decoded.payload, frame.payload, "decoded payload must match original payload exactly");

            // 6. Verify Expected Buffer State (buffer must be completely consumed)
            assert!(buffer.is_empty(), "buffer must be empty after complete frame is consumed");
        }
    }
}
