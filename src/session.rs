use anyhow::{Context, Result, ensure};
use bitvec::prelude::{BitVec, Msb0};
use bytes::Bytes;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

use crate::client::{Announce, Event};
use crate::peer::Request;
use crate::storage::Storage;
use crate::torrent::Torrent;

const MAX_BLOCK_LEN: u32 = 1 << 17;
const HAVE_BUFFER: usize = 64;

#[derive(Debug)]
pub struct Session {
    pub torrent: Torrent,
    pub peer_id: [u8; 20],
    storage: Storage,
    have: RwLock<BitVec<u8, Msb0>>,
    completed: broadcast::Sender<u32>,
    uploaded: AtomicU64,
    downloaded: AtomicU64,
}

impl Session {
    #[must_use]
    pub fn new(torrent: Torrent, peer_id: [u8; 20], storage: Storage) -> Self {
        let piece_count = torrent.meta.info.piece_count();

        Self {
            torrent,
            peer_id,
            storage,
            have: RwLock::new(BitVec::repeat(false, piece_count)),
            completed: broadcast::Sender::new(HAVE_BUFFER),
            uploaded: AtomicU64::new(0),
            downloaded: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn bitfield(&self) -> Bytes {
        Bytes::copy_from_slice(self.have.read().as_raw_slice())
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<u32> {
        self.completed.subscribe()
    }

    #[must_use]
    pub fn stats(&self, event: Option<Event>) -> Announce {
        let downloaded = self.downloaded.load(Ordering::Relaxed);

        Announce {
            uploaded: self.uploaded.load(Ordering::Relaxed),
            downloaded,
            left: self.torrent.meta.info.length.saturating_sub(downloaded),
            event,
        }
    }

    pub async fn store_piece(&self, index: u32, piece: Bytes) -> Result<()> {
        let len = u64::try_from(piece.len())?;
        self.storage.write_piece(index, piece).await?;

        let slot = usize::try_from(index)?;
        *self
            .have
            .write()
            .get_mut(slot)
            .with_context(|| format!("Invalid piece index: {index}"))? = true;
        self.downloaded.fetch_add(len, Ordering::Relaxed);
        let _ = self.completed.send(index);

        Ok(())
    }

    pub async fn read_block(&self, request: Request) -> Result<Bytes> {
        let Request {
            index,
            begin,
            length,
        } = request;

        ensure!(
            length <= MAX_BLOCK_LEN,
            "Requested block is too large: {length}"
        );
        ensure!(
            self.have
                .read()
                .get(usize::try_from(index)?)
                .is_some_and(|bit| *bit),
            "Requested missing piece {index}"
        );
        let piece_len = self
            .torrent
            .meta
            .info
            .piece_len(index)
            .with_context(|| format!("Invalid piece index: {index}"))?;
        ensure!(
            begin
                .checked_add(length)
                .is_some_and(|end| end <= piece_len),
            "Requested block is out of piece bounds"
        );

        let block = self.storage.read_block(index, begin, length).await?;
        self.uploaded
            .fetch_add(u64::from(length), Ordering::Relaxed);

        Ok(block)
    }
}
