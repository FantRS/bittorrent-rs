use anyhow::{Result, anyhow};
use std::path::Path;

use crate::bencode::BencodeParser;
use crate::torrent::TorrentMeta;

#[derive(Debug)]
pub struct Torrent {
    pub meta: TorrentMeta,
    pub info_hash: [u8; 20],
}

impl Torrent {
    pub fn load<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let bytes = std::fs::read(path)?;
        let mut parser = BencodeParser::new(bytes);
        let bencode_tree = parser.parse()?;

        let meta = bencode_tree.try_into()?;
        let info_hash = parser
            .info_hash()
            .ok_or_else(|| anyhow!("Info hash getting error"))?;

        Ok(Self { meta, info_hash })
    }
}
