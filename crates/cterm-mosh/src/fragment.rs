//! Fragment splitting and reassembly for mosh datagrams.
//!
//! Wire format:
//!   `[instruction_id:8 BE][fragment_num(15 bits)|final_flag(1 bit):2 BE][payload]`

/// Header size: 8 (instruction_id) + 2 (fragment_num + final flag)
pub const FRAGMENT_HEADER_SIZE: usize = 10;

/// Maximum payload per fragment.
pub const MAX_FRAGMENT_PAYLOAD: usize = 1300;

/// A single fragment of a larger message.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub id: u64,
    pub fragment_num: u16,
    pub is_final: bool,
    pub payload: Vec<u8>,
}

impl Fragment {
    /// Serialize to wire format.
    pub fn marshal(&self) -> Vec<u8> {
        let mut buf = vec![0u8; FRAGMENT_HEADER_SIZE + self.payload.len()];
        buf[0..8].copy_from_slice(&self.id.to_be_bytes());
        // Mosh layout: [final:1 bit][fragment_num:15 bits] (big-endian uint16)
        let mut num_and_final = self.fragment_num & 0x7fff;
        if self.is_final {
            num_and_final |= 0x8000;
        }
        buf[8..10].copy_from_slice(&num_and_final.to_be_bytes());
        buf[FRAGMENT_HEADER_SIZE..].copy_from_slice(&self.payload);
        buf
    }

    /// Parse from wire format.
    pub fn unmarshal(data: &[u8]) -> Result<Self, crate::MoshError> {
        if data.len() < FRAGMENT_HEADER_SIZE {
            return Err(crate::MoshError::FragmentTooShort);
        }
        let id = u64::from_be_bytes(data[0..8].try_into().unwrap());
        let num_and_final = u16::from_be_bytes(data[8..10].try_into().unwrap());
        Ok(Fragment {
            id,
            fragment_num: num_and_final & 0x7fff,
            is_final: (num_and_final & 0x8000) != 0,
            payload: data[FRAGMENT_HEADER_SIZE..].to_vec(),
        })
    }
}

/// Split data into fragments.
pub fn fragmentize(id: u64, data: &[u8]) -> Vec<Fragment> {
    if data.is_empty() {
        return vec![Fragment {
            id,
            fragment_num: 0,
            is_final: true,
            payload: Vec::new(),
        }];
    }

    let n = data.len().div_ceil(MAX_FRAGMENT_PAYLOAD);
    let mut frags = Vec::with_capacity(n);
    for i in 0..n {
        let start = i * MAX_FRAGMENT_PAYLOAD;
        let end = (start + MAX_FRAGMENT_PAYLOAD).min(data.len());
        frags.push(Fragment {
            id,
            fragment_num: i as u16,
            is_final: i == n - 1,
            payload: data[start..end].to_vec(),
        });
    }
    frags
}

/// Reassembles fragments into complete messages.
pub struct FragmentAssembler {
    current_id: u64,
    fragments: Vec<Option<Vec<u8>>>,
    total_num: Option<usize>,
}

impl FragmentAssembler {
    pub fn new() -> Self {
        Self {
            current_id: 0,
            fragments: Vec::new(),
            total_num: None,
        }
    }

    /// Add a fragment. Returns the reassembled message when all fragments are received.
    pub fn add(&mut self, f: &Fragment) -> Option<Vec<u8>> {
        // Stale fragment
        if f.id < self.current_id {
            return None;
        }

        // New message — reset state
        if f.id != self.current_id {
            self.current_id = f.id;
            self.fragments.clear();
            self.total_num = None;
        }

        let idx = f.fragment_num as usize;
        while self.fragments.len() <= idx {
            self.fragments.push(None);
        }
        self.fragments[idx] = Some(f.payload.clone());

        if f.is_final {
            self.total_num = Some(idx + 1);
        }

        let total = self.total_num?;
        if self.fragments.len() < total {
            return None;
        }
        for i in 0..total {
            self.fragments[i].as_ref()?;
        }

        // Reassemble
        let mut result = Vec::new();
        for i in 0..total {
            result.extend_from_slice(self.fragments[i].as_ref().unwrap());
        }

        self.fragments.clear();
        self.total_num = None;
        Some(result)
    }
}

impl Default for FragmentAssembler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_fragment_round_trip() {
        let data = b"hello world";
        let frags = fragmentize(1, data);
        assert_eq!(frags.len(), 1);
        assert!(frags[0].is_final);

        let wire = frags[0].marshal();
        let f = Fragment::unmarshal(&wire).unwrap();
        assert_eq!(f.id, 1);
        assert_eq!(f.fragment_num, 0);
        assert!(f.is_final);
        assert_eq!(f.payload, data);
    }

    #[test]
    fn multi_fragment_reassembly() {
        let data = vec![0xABu8; MAX_FRAGMENT_PAYLOAD * 3 + 100];
        let frags = fragmentize(42, &data);
        assert_eq!(frags.len(), 4);
        assert!(!frags[0].is_final);
        assert!(!frags[1].is_final);
        assert!(!frags[2].is_final);
        assert!(frags[3].is_final);

        let mut assembler = FragmentAssembler::new();
        assert!(assembler.add(&frags[0]).is_none());
        assert!(assembler.add(&frags[1]).is_none());
        assert!(assembler.add(&frags[2]).is_none());
        let result = assembler.add(&frags[3]).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn empty_data_produces_single_fragment() {
        let frags = fragmentize(1, &[]);
        assert_eq!(frags.len(), 1);
        assert!(frags[0].is_final);
        assert!(frags[0].payload.is_empty());
    }

    #[test]
    fn stale_fragment_ignored() {
        let mut assembler = FragmentAssembler::new();
        let f1 = Fragment {
            id: 5,
            fragment_num: 0,
            is_final: true,
            payload: b"new".to_vec(),
        };
        assembler.add(&f1);

        let stale = Fragment {
            id: 3,
            fragment_num: 0,
            is_final: true,
            payload: b"old".to_vec(),
        };
        assert!(assembler.add(&stale).is_none());
    }
}
