//! Datagram encode/decode with timestamps.
//!
//! Wire format: `[dir_seq:8][encrypted([timestamp:2][ts_reply:2][fragment_data...])]`

use crate::crypto::{Direction, MoshCrypto};
use crate::fragment::{Fragment, FRAGMENT_HEADER_SIZE};
use crate::MoshError;

/// Minimum wire datagram: 8 (dir_seq) + 16 (OCB tag) = 24 bytes.
pub const MIN_DATAGRAM_SIZE: usize = 24;

/// Encode a fragment into a wire datagram with timestamps.
pub fn encode_datagram(
    crypto: &MoshCrypto,
    seq: &mut u64,
    direction: Direction,
    fragment: &Fragment,
    timestamp_ms: u16,
    ts_reply: u16,
) -> Result<Vec<u8>, MoshError> {
    *seq += 1;

    let frag_wire = fragment.marshal();

    // Build plaintext: [timestamp:2][ts_reply:2][fragment_data...]
    let mut plaintext = Vec::with_capacity(4 + frag_wire.len());
    plaintext.extend_from_slice(&timestamp_ms.to_be_bytes());
    plaintext.extend_from_slice(&ts_reply.to_be_bytes());
    plaintext.extend_from_slice(&frag_wire);

    crypto.encrypt(*seq, direction, &plaintext)
}

/// Decoded datagram contents.
pub struct DecodedDatagram {
    pub seq: u64,
    pub timestamp: u16,
    pub ts_reply: u16,
    pub fragment: Fragment,
}

/// Decode a wire datagram into its components.
pub fn decode_datagram(
    crypto: &MoshCrypto,
    direction: Direction,
    data: &[u8],
) -> Result<DecodedDatagram, MoshError> {
    if data.len() < MIN_DATAGRAM_SIZE {
        return Err(MoshError::DatagramTooShort);
    }

    let (seq, plaintext) = crypto.decrypt(direction, data)?;

    if plaintext.len() < 4 + FRAGMENT_HEADER_SIZE {
        return Err(MoshError::DatagramTooShort);
    }

    let timestamp = u16::from_be_bytes(plaintext[0..2].try_into().unwrap());
    let ts_reply = u16::from_be_bytes(plaintext[2..4].try_into().unwrap());
    let fragment = Fragment::unmarshal(&plaintext[4..])?;

    Ok(DecodedDatagram {
        seq,
        timestamp,
        ts_reply,
        fragment,
    })
}

/// Get current timestamp as 16-bit truncated milliseconds.
pub fn timestamp_now() -> u16 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    (ms & 0xffff) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_round_trip() {
        use base64::Engine;
        let key = [0x42u8; 16];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(key);
        let crypto = MoshCrypto::new(&key_b64).unwrap();

        let frag = Fragment {
            id: 1,
            fragment_num: 0,
            is_final: true,
            payload: b"test payload".to_vec(),
        };

        let mut seq = 0u64;
        let wire =
            encode_datagram(&crypto, &mut seq, Direction::ToServer, &frag, 1234, 5678).unwrap();
        assert_eq!(seq, 1);

        let decoded = decode_datagram(&crypto, Direction::ToServer, &wire).unwrap();
        assert_eq!(decoded.seq, 1);
        assert_eq!(decoded.timestamp, 1234);
        assert_eq!(decoded.ts_reply, 5678);
        assert_eq!(decoded.fragment.id, 1);
        assert_eq!(decoded.fragment.payload, b"test payload");
    }
}
