pub mod magnet;
pub mod meta;

pub use magnet::Magnet;
pub use meta::{FileEntry, TorrentInfo, TorrentMeta};

use anyhow::{Result, anyhow};
use std::path::Path;

use crate::bencode::BencodeParser;

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

    pub fn from_metadata(magnet: &Magnet, metadata: Vec<u8>) -> Result<Self> {
        let info = BencodeParser::new(metadata).parse()?.try_into()?;

        Ok(Self {
            meta: TorrentMeta {
                trackers: magnet.trackers.clone(),
                info,
            },
            info_hash: magnet.info_hash,
        })
    }
}
