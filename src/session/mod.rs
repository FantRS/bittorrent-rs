mod client;

pub use client::Client;

use anyhow::{Context, Result, ensure};
use bitvec::prelude::{BitVec, Msb0};
use bytes::Bytes;
use parking_lot::RwLock;
use rayon::prelude::*;
use std::convert::Infallible;
use std::future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{broadcast, mpsc};

use crate::cpu;
use crate::discovery::Discovery;
use crate::peer::{PeerId, Request};
use crate::storage::{ResumeData, Storage};
use crate::torrent::{PieceIndex, Torrent};
use crate::tracker::{Announce, Event};

const MAX_BLOCK_LEN: u32 = 1 << 17;
const HAVE_BUFFER: usize = 64;

#[derive(Debug)]
pub struct Session {
    pub torrent: Torrent,
    pub peer_id: PeerId,
    storage: Storage,
    have: RwLock<BitVec<u8, Msb0>>,
    completed: broadcast::Sender<PieceIndex>,
    uploaded: AtomicU64,
    downloaded: AtomicU64,
}

impl Session {
    #[must_use]
    pub fn new(torrent: Torrent, peer_id: PeerId, storage: Storage) -> Self {
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

    pub async fn resume(self: &Arc<Self>, saved: Option<ResumeData>) -> Result<usize> {
        if let Some(saved) = saved
            && saved.info_hash == self.torrent.info_hash
            && saved.files == self.storage.stamps().await?
        {
            return Ok(self.restore(saved.have));
        }

        if !self.storage.has_existing_data() {
            return Ok(0);
        }

        let session = Arc::clone(self);
        cpu::spawn(move || session.verify_existing()).await
    }

    pub async fn resume_data(&self) -> Result<ResumeData> {
        let have = self.bitfield().to_vec();
        self.storage.sync().await?;

        Ok(ResumeData {
            info_hash: self.torrent.info_hash,
            have,
            files: self.storage.stamps().await?,
        })
    }

    fn restore(&self, raw: Vec<u8>) -> usize {
        let mut have = BitVec::from_vec(raw);
        have.resize(self.torrent.meta.info.piece_count(), false);
        let count = have.count_ones();
        *self.have.write() = have;

        count
    }

    fn verify_existing(&self) -> usize {
        let info = &self.torrent.meta.info;
        let valid: Vec<usize> = (0..info.piece_count())
            .into_par_iter()
            .filter_map(|slot| {
                let index = PieceIndex::try_from(slot).ok()?;
                let length = info.piece_len(index)?;
                let piece = self.storage.read_piece_blocking(index, length).ok()?;
                info.verify(index, &piece).then_some(slot)
            })
            .collect();

        let mut have = self.have.write();
        for &slot in &valid {
            have.set(slot, true);
        }

        valid.len()
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.have.read().all()
    }

    #[must_use]
    pub fn missing(&self) -> BitVec {
        self.have.read().iter().map(|bit| !*bit).collect()
    }

    #[must_use]
    pub fn verified_bytes(&self) -> u64 {
        self.torrent.meta.info.length.saturating_sub(self.left())
    }

    #[must_use]
    pub fn bitfield(&self) -> Bytes {
        Bytes::copy_from_slice(self.have.read().as_raw_slice())
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<PieceIndex> {
        self.completed.subscribe()
    }

    #[must_use]
    pub fn announce(&self, port: u16, event: Option<Event>) -> Announce {
        Announce {
            info_hash: self.torrent.info_hash,
            peer_id: self.peer_id,
            port,
            uploaded: self.uploaded.load(Ordering::Relaxed),
            downloaded: self.downloaded.load(Ordering::Relaxed),
            left: self.left(),
            event,
        }
    }

    pub async fn run_announcer(
        &self,
        port: u16,
        peers: mpsc::Sender<Vec<SocketAddr>>,
        discovery: &Discovery,
    ) -> Infallible {
        let request = |event| self.announce(port, event);
        discovery
            .discover(&self.torrent.meta.trackers, request, &peers)
            .await;
        future::pending().await
    }

    fn left(&self) -> u64 {
        let info = &self.torrent.meta.info;
        self.have
            .read()
            .iter_zeros()
            .filter_map(|slot| {
                PieceIndex::try_from(slot)
                    .ok()
                    .and_then(|index| info.piece_len(index))
            })
            .map(u64::from)
            .sum()
    }

    pub async fn store_piece(&self, index: PieceIndex, piece: Bytes) -> Result<()> {
        let len = u64::try_from(piece.len())?;
        self.storage.write_piece(index, piece).await?;

        *self
            .have
            .write()
            .get_mut(index.as_usize())
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
                .get(index.as_usize())
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
