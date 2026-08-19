use anyhow::{Result, anyhow};

use crate::bencode::BencodeValue;

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
        let BencodeValue::Dict(mut dict) = value else {
            return Err(anyhow!("Expected a dictionary for MetaInfo"));
        };

        let announce = dict
            .remove(&b"announce".to_vec())
            .ok_or_else(|| anyhow!("Missing 'announce' key in MetaInfo"))?
            .try_into()?;

        let info = dict
            .remove(&b"info".to_vec())
            .ok_or_else(|| anyhow!("Missing 'info' key in MetaInfo"))?
            .try_into()?;

        Ok(TorrentMeta { announce, info })
    }
}

impl TryFrom<BencodeValue> for TorrentInfo {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Dict(mut dict) = value else {
            return Err(anyhow!("Expected a dictionary for Info"));
        };

        let name = dict
            .remove(&b"name".to_vec())
            .ok_or_else(|| anyhow!("Missing 'name' key in Info"))?
            .try_into()?;

        let piece_length = dict
            .remove(&b"piece length".to_vec())
            .ok_or_else(|| anyhow!("Missing 'piece length' key in Info"))?
            .try_into()?;

        let pieces = dict
            .remove(&b"pieces".to_vec())
            .ok_or_else(|| anyhow!("Missing 'pieces' key in Info"))?
            .try_into()?;

        let length = dict
            .remove(&b"length".to_vec())
            .ok_or_else(|| anyhow!("Missing 'length' key in Info"))?
            .try_into()?;

        Ok(TorrentInfo {
            name,
            piece_length,
            pieces,
            length,
        })
    }
}

impl std::fmt::Debug for TorrentInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorrentInfo")
            .field("name", &self.name)
            .field("piece_length", &self.piece_length)
            .field("length", &self.length)
            .finish()
    }
}
