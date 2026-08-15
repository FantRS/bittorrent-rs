use anyhow::{Result, anyhow};

use crate::bencode::BencodeValue;

#[derive(Debug, Clone)]
pub struct TorrentMeta {
    pub announce: String,
    pub info: TorrentInfo,
}

#[derive(Debug, Clone)]
pub struct TorrentInfo {
    pub name: String,
    pub piece_length: i64,
    pub pieces: Vec<u8>,
    pub length: i64,
}

impl TryFrom<BencodeValue> for TorrentMeta {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        if let BencodeValue::Dict(mut dict) = value {
            let announce = dict
                .remove(&b"announce".to_vec())
                .ok_or_else(|| anyhow!("Missing 'announce' key in MetaInfo"))?;

            let info_value = dict
                .remove(&b"info".to_vec())
                .ok_or_else(|| anyhow!("Missing 'info' key in MetaInfo"))?;

            let announce: String = announce.try_into()?;
            let info: TorrentInfo = info_value.try_into()?;

            Ok(TorrentMeta { announce, info })
        } else {
            Err(anyhow!("Expected a dictionary for MetaInfo"))
        }
    }
}

impl TryFrom<BencodeValue> for TorrentInfo {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        if let BencodeValue::Dict(mut dict) = value {
            let name = dict
                .remove(&b"name".to_vec())
                .ok_or_else(|| anyhow!("Missing 'name' key in Info"))?;

            let piece_length = dict
                .remove(&b"piece length".to_vec())
                .ok_or_else(|| anyhow!("Missing 'piece length' key in Info"))?;

            let pieces = dict
                .remove(&b"pieces".to_vec())
                .ok_or_else(|| anyhow!("Missing 'pieces' key in Info"))?;

            let length = dict
                .remove(&b"length".to_vec())
                .ok_or_else(|| anyhow!("Missing 'length' key in Info"))?;

            let name: String = name.try_into()?;
            let piece_length: i64 = piece_length.try_into()?;
            let pieces: Vec<u8> = pieces.try_into()?;
            let length: i64 = length.try_into()?;

            Ok(TorrentInfo {
                name,
                piece_length,
                pieces,
                length,
            })
        } else {
            Err(anyhow!("Expected a dictionary for Info"))
        }
    }
}
