//! AES-128-OCB3 encryption/decryption for the mosh protocol (RFC 7253).

use aead::{Aead, KeyInit};
use aes::Aes128;
use ocb3::Ocb3;

use crate::MoshError;

/// Direction of communication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Client → Server (bit 63 = 0)
    ToServer,
    /// Server → Client (bit 63 = 1)
    ToClient,
}

impl Direction {
    /// Returns the direction bit as a u64 value (for the high bit of the 8-byte nonce header).
    pub fn bit(self) -> u64 {
        match self {
            Direction::ToServer => 0,
            Direction::ToClient => 1u64 << 63,
        }
    }
}

/// AES-128-OCB3 authenticated encryption for mosh datagrams.
pub struct MoshCrypto {
    cipher: Ocb3<Aes128>,
}

impl MoshCrypto {
    /// Create from a base64-encoded 16-byte key (as returned by `mosh-server`).
    pub fn new(key_base64: &str) -> Result<Self, MoshError> {
        use base64::Engine;
        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(key_base64)
            .map_err(|_| MoshError::InvalidKey)?;
        if key_bytes.len() != 16 {
            return Err(MoshError::InvalidKey);
        }
        let key = aes::cipher::generic_array::GenericArray::from_slice(&key_bytes);
        let cipher = Ocb3::<Aes128>::new(key);
        Ok(Self { cipher })
    }

    /// Build the 12-byte OCB3 nonce from sequence number and direction.
    ///
    /// Layout: `[0,0,0,0][dir_bit(63) | seq(0..62) as 8 bytes BE]` = 12 bytes
    fn make_nonce(
        seq: u64,
        direction: Direction,
    ) -> aead::generic_array::GenericArray<u8, aead::generic_array::typenum::U12> {
        let dir_seq = direction.bit() | (seq & ((1u64 << 63) - 1));
        let mut nonce = [0u8; 12];
        nonce[4..12].copy_from_slice(&dir_seq.to_be_bytes());
        *aead::generic_array::GenericArray::from_slice(&nonce)
    }

    /// Encrypt plaintext into a wire datagram.
    ///
    /// Returns: `[dir_seq:8][ciphertext + tag]`
    pub fn encrypt(
        &self,
        seq: u64,
        direction: Direction,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, MoshError> {
        let dir_seq = direction.bit() | (seq & ((1u64 << 63) - 1));
        let nonce = Self::make_nonce(seq, direction);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| MoshError::EncryptionFailed)?;

        let mut wire = Vec::with_capacity(8 + ct.len());
        wire.extend_from_slice(&dir_seq.to_be_bytes());
        wire.extend_from_slice(&ct);
        Ok(wire)
    }

    /// Decrypt a wire datagram.
    ///
    /// Input: `[dir_seq:8][ciphertext + tag]`
    /// Returns: `(seq, plaintext)`
    pub fn decrypt(&self, direction: Direction, data: &[u8]) -> Result<(u64, Vec<u8>), MoshError> {
        if data.len() < 24 {
            return Err(MoshError::DatagramTooShort);
        }

        let dir_seq = u64::from_be_bytes(data[0..8].try_into().unwrap());
        let expected_dir = direction.bit();
        if (dir_seq & (1u64 << 63)) != (expected_dir & (1u64 << 63)) {
            return Err(MoshError::WrongDirection);
        }

        let seq = dir_seq & ((1u64 << 63) - 1);
        let nonce = Self::make_nonce(seq, direction);

        let plaintext = self
            .cipher
            .decrypt(&nonce, &data[8..])
            .map_err(|_| MoshError::DecryptionFailed)?;

        Ok((seq, plaintext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        use base64::Engine;
        let key = [0x42u8; 16];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(key);
        let crypto = MoshCrypto::new(&key_b64).unwrap();

        let plaintext = b"hello mosh";
        let wire = crypto.encrypt(1, Direction::ToServer, plaintext).unwrap();
        let (seq, decrypted) = crypto.decrypt(Direction::ToServer, &wire).unwrap();

        assert_eq!(seq, 1);
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn wrong_direction_fails() {
        use base64::Engine;
        let key = [0x42u8; 16];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(key);
        let crypto = MoshCrypto::new(&key_b64).unwrap();

        let wire = crypto.encrypt(1, Direction::ToServer, b"test").unwrap();
        assert!(crypto.decrypt(Direction::ToClient, &wire).is_err());
    }

    #[test]
    fn tampered_data_fails() {
        use base64::Engine;
        let key = [0x42u8; 16];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(key);
        let crypto = MoshCrypto::new(&key_b64).unwrap();

        let mut wire = crypto.encrypt(1, Direction::ToServer, b"test").unwrap();
        // Tamper with ciphertext
        if let Some(last) = wire.last_mut() {
            *last ^= 0xff;
        }
        assert!(crypto.decrypt(Direction::ToServer, &wire).is_err());
    }
}
