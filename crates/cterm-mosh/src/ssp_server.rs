//! Server-side SSP (State Synchronization Protocol) state machine.
//!
//! Mirror of `SspState` with flipped directions:
//! - Encrypts with `Direction::ToClient`
//! - Decrypts with `Direction::ToServer`
//! - Sends `HostMessage`, receives `UserMessage`

use std::time::{Duration, Instant};

use crate::crypto::{Direction, MoshCrypto};
use crate::fragment::{fragmentize, FragmentAssembler};
use crate::proto::{self, HostMessage, TransportInstruction, UserMessage};
use crate::transport;
use crate::MoshError;

const MIN_RTO: Duration = Duration::from_millis(250);
const MAX_RTO: Duration = Duration::from_secs(10);

/// SSP state machine for the mosh server side.
pub struct SspServerState {
    crypto: MoshCrypto,

    // Outgoing (server → client)
    sent_num: u64,
    acked_num: u64,
    pending_diff: Option<Vec<u8>>,
    pending_old_num: u64,
    has_pending: bool,

    // Incoming (client → server)
    recv_num: u64,
    sent_ack_num: u64,
    pending_data_ack: bool,

    // Crypto sequences
    seq_out: u64,
    seq_in_max: u64,
    seq_in_max_set: bool,

    // Timestamps
    last_send: Instant,
    last_recv: Instant,
    last_remote_ts: u16,

    // RTT estimation
    srtt: Duration,
    rttvar: Duration,
    rto: Duration,
    rtt_init: bool,

    assembler: FragmentAssembler,
}

impl SspServerState {
    pub fn new(crypto: MoshCrypto) -> Self {
        let now = Instant::now();
        Self {
            crypto,
            sent_num: 0,
            acked_num: 0,
            pending_diff: None,
            pending_old_num: 0,
            has_pending: false,
            recv_num: 0,
            sent_ack_num: 0,
            pending_data_ack: false,
            seq_out: 0,
            seq_in_max: 0,
            seq_in_max_set: false,
            last_send: now,
            last_recv: now,
            last_remote_ts: 0,
            srtt: Duration::ZERO,
            rttvar: Duration::ZERO,
            rto: Duration::from_secs(1),
            rtt_init: false,
            assembler: FragmentAssembler::new(),
        }
    }

    /// Process a received wire datagram from the client.
    /// Returns decoded user messages (keystrokes, resize) if a complete
    /// transport instruction was reassembled.
    pub fn recv(&mut self, data: &[u8]) -> Result<Option<Vec<UserMessage>>, MoshError> {
        // Server decrypts with Direction::ToServer (client encrypted with ToServer)
        let decoded = transport::decode_datagram(&self.crypto, Direction::ToServer, data)?;

        // Reject replayed/out-of-order packets
        if self.seq_in_max_set && decoded.seq <= self.seq_in_max {
            return Ok(None);
        }

        self.seq_in_max = decoded.seq;
        self.seq_in_max_set = true;
        self.last_recv = Instant::now();
        self.last_remote_ts = decoded.timestamp;

        if decoded.ts_reply != 0 {
            self.update_rtt(decoded.ts_reply);
        }

        // Reassemble fragments
        let msg = match self.assembler.add(&decoded.fragment) {
            Some(m) => m,
            None => return Ok(None),
        };

        // Decompress
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let mut decoder = ZlibDecoder::new(&msg[..]);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|_| MoshError::DecompressionFailed)?;

        let ti =
            TransportInstruction::unmarshal(&decompressed).map_err(|_| MoshError::ProtobufError)?;

        // Update ack state
        if ti.ack_num > self.acked_num {
            self.acked_num = ti.ack_num;
            if self.acked_num >= self.sent_num {
                self.has_pending = false;
                self.pending_diff = None;
            }
        }
        if ti.new_num > self.recv_num {
            self.recv_num = ti.new_num;
        }

        // Parse diff into user messages (keystrokes, resize)
        match ti.diff {
            Some(ref diff) if !diff.is_empty() => {
                self.pending_data_ack = true;
                let msgs =
                    proto::unmarshal_user_messages(diff).map_err(|_| MoshError::ProtobufError)?;
                Ok(Some(msgs))
            }
            _ => Ok(Some(Vec::new())),
        }
    }

    /// Queue host messages to send (terminal output, resize ack, echo ack).
    pub fn queue(&mut self, messages: &[HostMessage]) {
        let diff = proto::marshal_host_messages(messages);
        self.sent_num += 1;
        self.pending_diff = Some(diff);
        self.pending_old_num = self.acked_num;
        self.has_pending = true;
    }

    /// Produce wire datagrams to send to the client. Call this on timer tick.
    pub fn tick(&mut self) -> Result<Vec<Vec<u8>>, MoshError> {
        let have_diff = self.has_pending && self.pending_diff.is_some();
        let need_ack = self.recv_num > self.sent_ack_num;
        let since_last_send = self.last_send.elapsed();
        let expired = since_last_send >= self.rto;
        let urgent_ack = self.pending_data_ack;

        if !have_diff && !need_ack && !expired && !urgent_ack {
            return Ok(Vec::new());
        }

        self.pending_data_ack = false;

        let ti = TransportInstruction {
            protocol_version: 2,
            old_num: if have_diff {
                self.pending_old_num
            } else {
                self.acked_num
            },
            new_num: self.sent_num,
            ack_num: self.recv_num,
            throwaway_num: 0,
            diff: if have_diff {
                self.pending_diff.clone()
            } else {
                None
            },
            chaff: None,
        };
        self.sent_ack_num = self.recv_num;

        let pb_data = ti.marshal();

        // Compress with zlib
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&pb_data)
            .map_err(|_| MoshError::CompressionFailed)?;
        let compressed = encoder.finish().map_err(|_| MoshError::CompressionFailed)?;

        let frags = fragmentize(self.sent_num, &compressed);
        let ts = transport::timestamp_now();

        // Server encrypts with Direction::ToClient
        let mut datagrams = Vec::with_capacity(frags.len());
        for f in &frags {
            let wire = transport::encode_datagram(
                &self.crypto,
                &mut self.seq_out,
                Direction::ToClient,
                f,
                ts,
                self.last_remote_ts,
            )?;
            datagrams.push(wire);
        }

        self.last_send = Instant::now();
        Ok(datagrams)
    }

    /// Returns when the next tick should be called.
    pub fn next_deadline(&self) -> Duration {
        let elapsed = self.last_send.elapsed();
        if elapsed >= self.rto {
            Duration::ZERO
        } else {
            self.rto - elapsed
        }
    }

    /// Duration since last received packet.
    pub fn idle_time(&self) -> Duration {
        self.last_recv.elapsed()
    }

    /// Force the next tick to send immediately.
    pub fn force_next_send(&mut self) {
        self.last_send = Instant::now() - self.rto - Duration::from_millis(1);
    }

    fn update_rtt(&mut self, ts_reply: u16) {
        let now16 = transport::timestamp_now();
        let mut rtt_ms = (now16 as i32) - (ts_reply as i32);
        if rtt_ms < 0 {
            rtt_ms += 65536;
        }
        if rtt_ms > 30000 {
            return;
        }
        let rtt = Duration::from_millis(rtt_ms as u64);

        if !self.rtt_init {
            self.srtt = rtt;
            self.rttvar = rtt / 2;
            self.rtt_init = true;
        } else {
            let delta = self.srtt.abs_diff(rtt);
            self.rttvar = (self.rttvar * 3 + delta) / 4;
            self.srtt = (self.srtt * 7 + rtt) / 8;
        }

        self.rto = self.srtt + self.rttvar * 4;
        if self.rto < MIN_RTO {
            self.rto = MIN_RTO;
        }
        if self.rto > MAX_RTO {
            self.rto = MAX_RTO;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssp::SspState;

    fn make_crypto() -> MoshCrypto {
        use base64::Engine;
        let key = [0x42u8; 16];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(key);
        MoshCrypto::new(&key_b64).unwrap()
    }

    #[test]
    fn server_initial_tick_sends_keepalive() {
        let crypto = make_crypto();
        let mut ssp = SspServerState::new(crypto);
        ssp.force_next_send();
        let datagrams = ssp.tick().unwrap();
        assert!(!datagrams.is_empty());
    }

    #[test]
    fn server_queue_and_tick() {
        let crypto = make_crypto();
        let mut ssp = SspServerState::new(crypto);
        ssp.queue(&[HostMessage::HostBytes(b"hello".to_vec())]);
        let datagrams = ssp.tick().unwrap();
        assert!(!datagrams.is_empty());
    }

    #[test]
    fn client_server_round_trip() {
        let crypto_client = make_crypto();
        let crypto_server = make_crypto();

        let mut client = SspState::new(crypto_client);
        let mut server = SspServerState::new(crypto_server);

        // Client sends keystroke
        client.queue(&[UserMessage::Keystroke(b"x".to_vec())]);
        let client_datagrams = client.tick().unwrap();
        assert!(!client_datagrams.is_empty());

        // Server receives it
        for dg in &client_datagrams {
            let msgs = server.recv(dg).unwrap();
            if let Some(msgs) = msgs {
                assert!(msgs.iter().any(|m| matches!(m, UserMessage::Keystroke(_))));
            }
        }

        // Server sends host output
        server.queue(&[HostMessage::HostBytes(b"X".to_vec())]);
        let server_datagrams = server.tick().unwrap();
        assert!(!server_datagrams.is_empty());

        // Client receives it
        for dg in &server_datagrams {
            let msgs = client.recv(dg).unwrap();
            if let Some(msgs) = msgs {
                assert!(msgs.iter().any(|m| matches!(m, HostMessage::HostBytes(_))));
            }
        }
    }
}
