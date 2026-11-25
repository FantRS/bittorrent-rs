mod info_hash;
mod magnet;
mod meta;
mod piece_index;

pub use info_hash::InfoHash;
pub use magnet::Magnet;
pub use meta::{FileEntry, TorrentInfo, TorrentMeta};
pub use piece_index::PieceIndex;

use anyhow::{Context, Result};
use std::path::Path;

use crate::bencode::BencodeParser;

#[derive(Debug)]
pub struct Torrent {
    pub meta: TorrentMeta,
    pub info_hash: InfoHash,
}

impl Torrent {
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = tokio::fs::read(path).await?;
        let mut parser = BencodeParser::new(bytes);
        let bencode_tree = parser.parse()?;

        let meta = bencode_tree.try_into()?;
        let info_hash = InfoHash::of(
            parser
                .info_bytes()
                .context("Torrent has no 'info' dictionary")?,
        );

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
