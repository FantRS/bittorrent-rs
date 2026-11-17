use anyhow::{Result, ensure};
use sha1::{Digest, Sha1};

use crate::bencode::{BencodeValue, Dict, DictExt};

#[derive(Debug)]
pub struct TorrentMeta {
    pub announce: String,
    pub info: TorrentInfo,
}

#[derive(Clone)]
pub struct TorrentInfo {
    pub name: String,
    pub piece_length: u64,
    pub pieces: Vec<[u8; 20]>,
    pub length: u64,
}

impl TryFrom<BencodeValue> for TorrentMeta {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let mut dict = Dict::try_from(value)?;

        Ok(Self {
            announce: dict.take("announce")?,
            info: dict.take("info")?,
        })
    }
}

impl TryFrom<BencodeValue> for TorrentInfo {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let mut dict = Dict::try_from(value)?;
        let pieces: Vec<u8> = dict.take("pieces")?;
        let (hashes, rest) = pieces.as_chunks::<20>();
        ensure!(rest.is_empty(), "'pieces' length is not a multiple of 20");

        Ok(Self {
            name: dict.take("name")?,
            piece_length: dict.take("piece length")?,
            pieces: hashes.to_vec(),
            length: dict.take("length")?,
        })
    }
}

impl TorrentInfo {
    #[must_use]
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    #[must_use]
    pub fn piece_len(&self, index: u32) -> Option<u32> {
        let start = u64::from(index).checked_mul(self.piece_length)?;
        let len = self
            .length
            .checked_sub(start)
            .filter(|&rest| rest > 0)?
            .min(self.piece_length);

        u32::try_from(len).ok()
    }

    #[must_use]
    pub fn verify(&self, index: u32, data: &[u8]) -> bool {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.pieces.get(i))
            .is_some_and(|hash| Sha1::digest(data).as_slice() == hash)
    }
}

impl std::fmt::Debug for TorrentInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorrentInfo")
            .field("name", &self.name)
            .field("piece_length", &self.piece_length)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_cases() {
        let info = TorrentInfo {
            name: "file".into(),
            piece_length: 4,
            pieces: vec![Sha1::digest(b"abcd").into(), Sha1::digest(b"ef").into()],
            length: 6,
        };

        assert_eq!(info.piece_count(), 2);
        assert_eq!(info.piece_len(0), Some(4));
        assert_eq!(info.piece_len(1), Some(2));
        assert_eq!(info.piece_len(2), None);
        assert_eq!(info.piece_len(u32::MAX), None);

        assert!(info.verify(0, b"abcd"));
        assert!(info.verify(1, b"ef"));
        assert!(!info.verify(1, b"eg"));
        assert!(!info.verify(2, b""));
    }
}
