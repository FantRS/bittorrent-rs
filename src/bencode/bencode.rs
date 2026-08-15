use anyhow::{Result, anyhow};
use std::collections::BTreeMap;

use crate::torrent::TorrentMeta;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BencodeValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

impl BencodeValue {
    pub fn torrent_meta(self) -> Result<TorrentMeta> {
        TorrentMeta::try_from(self.clone())
    }
}

impl TryFrom<BencodeValue> for i64 {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        match value {
            BencodeValue::Int(i) => Ok(i),
            _ => Err(anyhow!("Expected an integer, got {:?}", value)),
        }
    }
}

impl TryFrom<BencodeValue> for Vec<u8> {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        match value {
            BencodeValue::Bytes(bytes) => Ok(bytes),
            _ => Err(anyhow!("Expected a byte string, got {:?}", value)),
        }
    }
}

impl TryFrom<BencodeValue> for String {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        match value {
            BencodeValue::Bytes(bytes) => {
                String::from_utf8(bytes).map_err(|e| anyhow!("Invalid UTF-8 string: {}", e))
            }
            _ => Err(anyhow!("Expected a byte string, got {:?}", value)),
        }
    }
}
