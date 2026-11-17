use anyhow::Result;

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
    pub pieces: Vec<u8>,
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

        Ok(Self {
            name: dict.take("name")?,
            piece_length: dict.take("piece length")?,
            pieces: dict.take("pieces")?,
            length: dict.take("length")?,
        })
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
