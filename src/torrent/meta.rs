use anyhow::{Result, ensure};
use sha1::{Digest, Sha1};

use crate::bencode::{BencodeValue, Dict, DictExt};

#[derive(Debug)]
pub struct TorrentMeta {
    pub trackers: Vec<String>,
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

        let mut trackers: Vec<String> = dict
            .take_opt::<Vec<BencodeValue>>("announce-list")
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tier| Vec::<BencodeValue>::try_from(tier).ok())
            .flatten()
            .filter_map(|tracker| String::try_from(tracker).ok())
            .collect();
        if trackers.is_empty() {
            trackers.extend(dict.take_opt::<String>("announce"));
        }
        ensure!(!trackers.is_empty(), "Torrent has no trackers");

        Ok(Self {
            trackers,
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
    use crate::bencode::BencodeParser;

    fn trackers(bencode: &str) -> Result<Vec<String>> {
        let info = "d6:lengthi1e4:name1:f12:piece lengthi1e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
        let value = BencodeParser::new(format!("d{bencode}4:info{info}e").into_bytes()).parse()?;
        Ok(TorrentMeta::try_from(value)?.trackers)
    }

    #[test]
    fn tracker_cases() {
        assert_eq!(trackers("8:announce5:http1").unwrap(), ["http1"]);
        assert_eq!(
            trackers("8:announce5:http113:announce-listll5:http1el4:udp14:udp2ee").unwrap(),
            ["http1", "udp1", "udp2"]
        );
        assert_eq!(trackers("13:announce-listll4:udp1ee").unwrap(), ["udp1"]);
        assert_eq!(
            trackers("8:announce5:http113:announce-listle").unwrap(),
            ["http1"]
        );
        assert!(trackers("").is_err());
    }

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
