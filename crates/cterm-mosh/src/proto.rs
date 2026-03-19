//! Hand-rolled protobuf for mosh wire format.
//!
//! Field numbers match upstream mobile-shell/mosh exactly.

/// Wire types for protobuf encoding.
const WIRE_VARINT: u64 = 0;
const WIRE_BYTES: u64 = 2;

// ---------------------------------------------------------------------------
// TransportInstruction
// field 1: protocol_version (uint32)
// field 2: old_num (uint64)
// field 3: new_num (uint64)
// field 4: ack_num (uint64)
// field 5: throwaway_num (uint64)
// field 6: diff (bytes)
// field 7: chaff (bytes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TransportInstruction {
    pub protocol_version: u64,
    pub old_num: u64,
    pub new_num: u64,
    pub ack_num: u64,
    pub throwaway_num: u64,
    pub diff: Option<Vec<u8>>,
    pub chaff: Option<Vec<u8>>,
}

impl TransportInstruction {
    pub fn marshal(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        if self.protocol_version != 0 {
            append_tag_varint(&mut buf, 1, self.protocol_version);
        }
        append_tag_varint(&mut buf, 2, self.old_num);
        append_tag_varint(&mut buf, 3, self.new_num);
        append_tag_varint(&mut buf, 4, self.ack_num);
        append_tag_varint(&mut buf, 5, self.throwaway_num);
        if let Some(ref diff) = self.diff {
            if !diff.is_empty() {
                append_tag_bytes(&mut buf, 6, diff);
            }
        }
        if let Some(ref chaff) = self.chaff {
            if !chaff.is_empty() {
                append_tag_bytes(&mut buf, 7, chaff);
            }
        }
        buf
    }

    pub fn unmarshal(data: &[u8]) -> Result<Self, ProtoError> {
        let mut ti = TransportInstruction::default();
        let mut offset = 0;
        while offset < data.len() {
            let (field, wtype, size) = decode_tag(data, offset)?;
            offset += size;
            match field {
                1 => {
                    let (v, sz) = decode_varint(data, offset)?;
                    ti.protocol_version = v;
                    offset += sz;
                }
                2 => {
                    let (v, sz) = decode_varint(data, offset)?;
                    ti.old_num = v;
                    offset += sz;
                }
                3 => {
                    let (v, sz) = decode_varint(data, offset)?;
                    ti.new_num = v;
                    offset += sz;
                }
                4 => {
                    let (v, sz) = decode_varint(data, offset)?;
                    ti.ack_num = v;
                    offset += sz;
                }
                5 => {
                    let (v, sz) = decode_varint(data, offset)?;
                    ti.throwaway_num = v;
                    offset += sz;
                }
                6 => {
                    let (bytes, sz) = decode_length_delimited(data, offset)?;
                    ti.diff = Some(bytes.to_vec());
                    offset += sz;
                }
                7 => {
                    let (bytes, sz) = decode_length_delimited(data, offset)?;
                    ti.chaff = Some(bytes.to_vec());
                    offset += sz;
                }
                _ => {
                    let sz = skip_field(data, offset, wtype)?;
                    offset += sz;
                }
            }
        }
        Ok(ti)
    }
}

// ---------------------------------------------------------------------------
// HostMessage / HostInstruction
// Outer: repeated field 1 = HostInstruction (length-delimited)
// Inner HostInstruction:
//   field 2: HostBytes { field 4: hoststring (bytes) }
//   field 3: ResizeMessage { field 5: width (varint), field 6: height (varint) }
//   field 7: EchoAck { field 8: echo_ack_num (varint) }
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum HostMessage {
    HostBytes(Vec<u8>),
    Resize(u16, u16),
    EchoAck(u64),
}

pub fn marshal_host_messages(msgs: &[HostMessage]) -> Vec<u8> {
    let mut buf = Vec::new();
    for msg in msgs {
        let sub = marshal_host_instruction(msg);
        append_tag_bytes(&mut buf, 1, &sub);
    }
    buf
}

pub fn unmarshal_host_messages(data: &[u8]) -> Result<Vec<HostMessage>, ProtoError> {
    let mut msgs = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (field, _wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        if field != 1 {
            return Err(ProtoError::UnexpectedField(field));
        }
        let (inner, sz) = decode_length_delimited(data, offset)?;
        offset += sz;
        let instructions = unmarshal_host_instruction(inner)?;
        msgs.extend(instructions);
    }
    Ok(msgs)
}

fn marshal_host_instruction(msg: &HostMessage) -> Vec<u8> {
    let mut buf = Vec::new();
    match msg {
        HostMessage::HostBytes(data) => {
            let mut sub = Vec::new();
            append_tag_bytes(&mut sub, 4, data);
            append_tag_bytes(&mut buf, 2, &sub);
        }
        HostMessage::Resize(w, h) => {
            let mut sub = Vec::new();
            append_tag_varint(&mut sub, 5, *w as u64);
            append_tag_varint(&mut sub, 6, *h as u64);
            append_tag_bytes(&mut buf, 3, &sub);
        }
        HostMessage::EchoAck(num) => {
            let mut sub = Vec::new();
            append_tag_varint(&mut sub, 8, *num);
            append_tag_bytes(&mut buf, 7, &sub);
        }
    }
    buf
}

fn unmarshal_host_instruction(data: &[u8]) -> Result<Vec<HostMessage>, ProtoError> {
    let mut msgs = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (field, _wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        match field {
            2 => {
                // HostBytes
                let (inner, sz) = decode_length_delimited(data, offset)?;
                offset += sz;
                if let Some(hoststring) = unmarshal_hoststring(inner)? {
                    msgs.push(HostMessage::HostBytes(hoststring));
                }
            }
            3 => {
                // ResizeMessage
                let (inner, sz) = decode_length_delimited(data, offset)?;
                offset += sz;
                let (w, h) = unmarshal_resize(inner)?;
                msgs.push(HostMessage::Resize(w, h));
            }
            7 => {
                // EchoAck
                let (inner, sz) = decode_length_delimited(data, offset)?;
                offset += sz;
                let num = unmarshal_echo_ack(inner)?;
                msgs.push(HostMessage::EchoAck(num));
            }
            _ => {
                let sz = skip_field(data, offset, _wtype)?;
                offset += sz;
            }
        }
    }
    Ok(msgs)
}

fn unmarshal_hoststring(data: &[u8]) -> Result<Option<Vec<u8>>, ProtoError> {
    let mut offset = 0;
    while offset < data.len() {
        let (field, wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        if field == 4 {
            let (bytes, _sz) = decode_length_delimited(data, offset)?;
            return Ok(Some(bytes.to_vec()));
        } else {
            let sz = skip_field(data, offset, wtype)?;
            offset += sz;
        }
    }
    Ok(None)
}

fn unmarshal_echo_ack(data: &[u8]) -> Result<u64, ProtoError> {
    let mut offset = 0;
    while offset < data.len() {
        let (field, wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        if field == 8 {
            let (v, _sz) = decode_varint(data, offset)?;
            return Ok(v);
        } else {
            let sz = skip_field(data, offset, wtype)?;
            offset += sz;
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// UserMessage / UserInstruction
// Outer: repeated field 1 = UserInstruction (length-delimited)
// Inner UserInstruction:
//   field 2: Keystroke { field 4: keys (bytes) }
//   field 3: ResizeMessage { field 5: width (varint), field 6: height (varint) }
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum UserMessage {
    Keystroke(Vec<u8>),
    Resize(u16, u16),
}

pub fn marshal_user_messages(msgs: &[UserMessage]) -> Vec<u8> {
    let mut buf = Vec::new();
    for msg in msgs {
        let sub = marshal_user_instruction(msg);
        append_tag_bytes(&mut buf, 1, &sub);
    }
    buf
}

pub fn unmarshal_user_messages(data: &[u8]) -> Result<Vec<UserMessage>, ProtoError> {
    let mut msgs = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (field, _wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        if field != 1 {
            return Err(ProtoError::UnexpectedField(field));
        }
        let (inner, sz) = decode_length_delimited(data, offset)?;
        offset += sz;
        let instructions = unmarshal_user_instruction(inner)?;
        msgs.extend(instructions);
    }
    Ok(msgs)
}

fn marshal_user_instruction(msg: &UserMessage) -> Vec<u8> {
    let mut buf = Vec::new();
    match msg {
        UserMessage::Keystroke(keys) => {
            let mut sub = Vec::new();
            append_tag_bytes(&mut sub, 4, keys);
            append_tag_bytes(&mut buf, 2, &sub);
        }
        UserMessage::Resize(w, h) => {
            let mut sub = Vec::new();
            append_tag_varint(&mut sub, 5, *w as u64);
            append_tag_varint(&mut sub, 6, *h as u64);
            append_tag_bytes(&mut buf, 3, &sub);
        }
    }
    buf
}

fn unmarshal_user_instruction(data: &[u8]) -> Result<Vec<UserMessage>, ProtoError> {
    let mut msgs = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (field, _wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        match field {
            2 => {
                // Keystroke
                let (inner, sz) = decode_length_delimited(data, offset)?;
                offset += sz;
                if let Some(keys) = unmarshal_keystroke(inner)? {
                    msgs.push(UserMessage::Keystroke(keys));
                }
            }
            3 => {
                // ResizeMessage
                let (inner, sz) = decode_length_delimited(data, offset)?;
                offset += sz;
                let (w, h) = unmarshal_resize(inner)?;
                msgs.push(UserMessage::Resize(w, h));
            }
            _ => {
                let sz = skip_field(data, offset, _wtype)?;
                offset += sz;
            }
        }
    }
    Ok(msgs)
}

fn unmarshal_keystroke(data: &[u8]) -> Result<Option<Vec<u8>>, ProtoError> {
    let mut keys = None;
    let mut offset = 0;
    while offset < data.len() {
        let (field, wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        if field == 4 {
            let (bytes, sz) = decode_length_delimited(data, offset)?;
            offset += sz;
            match &mut keys {
                None => keys = Some(bytes.to_vec()),
                Some(existing) => existing.extend_from_slice(bytes),
            }
        } else {
            let sz = skip_field(data, offset, wtype)?;
            offset += sz;
        }
    }
    Ok(keys)
}

fn unmarshal_resize(data: &[u8]) -> Result<(u16, u16), ProtoError> {
    let mut width = 0u16;
    let mut height = 0u16;
    let mut offset = 0;
    while offset < data.len() {
        let (field, wtype, tag_sz) = decode_tag(data, offset)?;
        offset += tag_sz;
        match field {
            5 => {
                let (v, sz) = decode_varint(data, offset)?;
                width = v as u16;
                offset += sz;
            }
            6 => {
                let (v, sz) = decode_varint(data, offset)?;
                height = v as u16;
                offset += sz;
            }
            _ => {
                let sz = skip_field(data, offset, wtype)?;
                offset += sz;
            }
        }
    }
    Ok((width, height))
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

fn append_tag(buf: &mut Vec<u8>, field: u64, wtype: u64) {
    append_varint(buf, (field << 3) | wtype);
}

fn append_varint(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        if v < 0x80 {
            buf.push(v as u8);
            break;
        }
        buf.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
}

fn append_tag_varint(buf: &mut Vec<u8>, field: u64, v: u64) {
    append_tag(buf, field, WIRE_VARINT);
    append_varint(buf, v);
}

fn append_tag_bytes(buf: &mut Vec<u8>, field: u64, data: &[u8]) {
    append_tag(buf, field, WIRE_BYTES);
    append_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

// ---------------------------------------------------------------------------
// Decoding helpers
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("truncated protobuf data")]
    Truncated,
    #[error("unexpected field number {0}")]
    UnexpectedField(u64),
}

fn decode_varint(data: &[u8], offset: usize) -> Result<(u64, usize), ProtoError> {
    let mut v: u64 = 0;
    for i in 0..10 {
        if offset + i >= data.len() {
            return Err(ProtoError::Truncated);
        }
        let c = data[offset + i];
        v |= ((c & 0x7f) as u64) << (7 * i);
        if c < 0x80 {
            return Ok((v, i + 1));
        }
    }
    Err(ProtoError::Truncated)
}

fn decode_tag(data: &[u8], offset: usize) -> Result<(u64, u64, usize), ProtoError> {
    let (v, sz) = decode_varint(data, offset)?;
    Ok((v >> 3, v & 7, sz))
}

fn decode_length_delimited(data: &[u8], offset: usize) -> Result<(&[u8], usize), ProtoError> {
    let (length, len_sz) = decode_varint(data, offset)?;
    let start = offset + len_sz;
    let end = start + length as usize;
    if end > data.len() {
        return Err(ProtoError::Truncated);
    }
    Ok((&data[start..end], len_sz + length as usize))
}

fn skip_field(data: &[u8], offset: usize, wtype: u64) -> Result<usize, ProtoError> {
    match wtype {
        0 => {
            // varint
            let (_, sz) = decode_varint(data, offset)?;
            Ok(sz)
        }
        2 => {
            // length-delimited
            let (_, sz) = decode_length_delimited(data, offset)?;
            Ok(sz)
        }
        5 => Ok(4), // 32-bit
        1 => Ok(8), // 64-bit
        _ => Err(ProtoError::Truncated),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_instruction_round_trip() {
        let ti = TransportInstruction {
            protocol_version: 2,
            old_num: 0,
            new_num: 1,
            ack_num: 0,
            throwaway_num: 0,
            diff: Some(b"hello".to_vec()),
            chaff: None,
        };
        let data = ti.marshal();
        let ti2 = TransportInstruction::unmarshal(&data).unwrap();
        assert_eq!(ti2.protocol_version, 2);
        assert_eq!(ti2.new_num, 1);
        assert_eq!(ti2.diff.as_deref(), Some(b"hello".as_slice()));
    }

    #[test]
    fn host_messages_round_trip() {
        let msgs = vec![
            HostMessage::HostBytes(b"terminal output".to_vec()),
            HostMessage::Resize(80, 24),
            HostMessage::EchoAck(42),
        ];
        let data = marshal_host_messages(&msgs);
        let msgs2 = unmarshal_host_messages(&data).unwrap();
        assert_eq!(msgs2.len(), 3);
        match &msgs2[0] {
            HostMessage::HostBytes(d) => assert_eq!(d, b"terminal output"),
            _ => panic!("expected HostBytes"),
        }
        match &msgs2[1] {
            HostMessage::Resize(w, h) => {
                assert_eq!(*w, 80);
                assert_eq!(*h, 24);
            }
            _ => panic!("expected Resize"),
        }
        match &msgs2[2] {
            HostMessage::EchoAck(n) => assert_eq!(*n, 42),
            _ => panic!("expected EchoAck"),
        }
    }

    #[test]
    fn user_messages_round_trip() {
        let msgs = vec![
            UserMessage::Keystroke(b"ls\n".to_vec()),
            UserMessage::Resize(120, 40),
        ];
        let data = marshal_user_messages(&msgs);
        let msgs2 = unmarshal_user_messages(&data).unwrap();
        assert_eq!(msgs2.len(), 2);
        match &msgs2[0] {
            UserMessage::Keystroke(k) => assert_eq!(k, b"ls\n"),
            _ => panic!("expected Keystroke"),
        }
        match &msgs2[1] {
            UserMessage::Resize(w, h) => {
                assert_eq!(*w, 120);
                assert_eq!(*h, 40);
            }
            _ => panic!("expected Resize"),
        }
    }

    #[test]
    fn varint_encoding() {
        let mut buf = Vec::new();
        append_varint(&mut buf, 300);
        let (val, sz) = decode_varint(&buf, 0).unwrap();
        assert_eq!(val, 300);
        assert_eq!(sz, 2);
    }
}
