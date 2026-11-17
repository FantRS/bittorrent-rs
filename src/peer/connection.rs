use anyhow::{Context, Result, anyhow, ensure};
use bitvec::prelude::{BitVec, Msb0};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::codec::Framed;

use crate::peer::{Handshake, Message, MessageCodec, Request};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const BLOCK_LEN: u32 = 1 << 14;
const MAX_PENDING_REQUESTS: usize = 5;
const STALL_TIMEOUT: Duration = Duration::from_mins(1);

#[derive(Debug)]
pub struct PeerConnection {
    pub addr: SocketAddr,
    pub peer_id: [u8; 20],
    frames: Framed<TcpStream, MessageCodec>,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
    pieces: BitVec<u8, Msb0>,
}

impl PeerConnection {
    pub async fn connect(
        addr: impl Into<SocketAddr>,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
        piece_count: usize,
    ) -> Result<Self> {
        let addr = addr.into();

        with_timeout(addr, async {
            let mut stream = TcpStream::connect(addr).await?;
            Handshake::new(info_hash, peer_id)
                .write_to(&mut stream)
                .await?;
            let reply = Handshake::read_from(&mut stream).await?;
            ensure!(reply.info_hash == info_hash, "info_hash mismatch");
            ensure!(reply.peer_id != peer_id, "Connected to ourselves");

            Ok(Self::new(addr, reply.peer_id, stream, piece_count))
        })
        .await
    }

    pub async fn accept(
        mut stream: TcpStream,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
        piece_count: usize,
    ) -> Result<Self> {
        let addr = stream.peer_addr()?;

        with_timeout(addr, async {
            let theirs = Handshake::read_from(&mut stream).await?;
            ensure!(theirs.info_hash == info_hash, "info_hash mismatch");
            ensure!(theirs.peer_id != peer_id, "Connected to ourselves");
            Handshake::new(info_hash, peer_id)
                .write_to(&mut stream)
                .await?;

            Ok(Self::new(addr, theirs.peer_id, stream, piece_count))
        })
        .await
    }

    fn new(addr: SocketAddr, peer_id: [u8; 20], stream: TcpStream, piece_count: usize) -> Self {
        Self {
            addr,
            peer_id,
            frames: Framed::new(stream, MessageCodec),
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
            pieces: BitVec::repeat(false, piece_count),
        }
    }

    #[must_use]
    pub fn is_choked(&self) -> bool {
        self.peer_choking
    }

    #[must_use]
    pub fn is_peer_interested(&self) -> bool {
        self.peer_interested
    }

    #[must_use]
    pub fn has_piece(&self, index: usize) -> bool {
        self.pieces.get(index).is_some_and(|bit| *bit)
    }

    #[must_use]
    pub fn has_any_piece(&self) -> bool {
        self.pieces.any()
    }

    pub async fn interested(&mut self) -> Result<()> {
        if !self.am_interested {
            self.send(Message::Interested).await?;
            self.am_interested = true;
        }

        Ok(())
    }

    pub async fn send(&mut self, message: Message) -> Result<()> {
        self.frames.send(message).await
    }

    pub async fn recv(&mut self) -> Result<Message> {
        let message = self
            .frames
            .next()
            .await
            .ok_or_else(|| anyhow!("{}: connection closed", self.addr))??;

        match &message {
            Message::Choke => self.peer_choking = true,
            Message::Unchoke => self.peer_choking = false,
            Message::Interested => self.peer_interested = true,
            Message::NotInterested => self.peer_interested = false,
            Message::Have(index) => {
                let mut bit = usize::try_from(*index)
                    .ok()
                    .and_then(|index| self.pieces.get_mut(index))
                    .ok_or_else(|| anyhow!("Invalid piece index: {index}"))?;
                *bit = true;
            }
            Message::Bitfield(bits) => {
                let piece_count = self.pieces.len();
                ensure!(
                    bits.len() == piece_count.div_ceil(8),
                    "Invalid bitfield length: {}",
                    bits.len()
                );
                self.pieces = BitVec::from_slice(bits);
                self.pieces.truncate(piece_count);
            }
            _ => {}
        }

        Ok(message)
    }

    pub async fn download_piece(&mut self, index: u32, length: u32) -> Result<Bytes> {
        let mut queue: VecDeque<Request> = (0..length.div_ceil(BLOCK_LEN))
            .map(|block| {
                let begin = block * BLOCK_LEN;
                Request {
                    index,
                    begin,
                    length: BLOCK_LEN.min(length - begin),
                }
            })
            .collect();
        let mut pending = Vec::with_capacity(MAX_PENDING_REQUESTS);
        let mut buf = BytesMut::zeroed(usize::try_from(length)?);
        let mut deadline = Instant::now() + STALL_TIMEOUT;

        while !queue.is_empty() || !pending.is_empty() {
            while !self.peer_choking
                && pending.len() < MAX_PENDING_REQUESTS
                && let Some(request) = queue.pop_front()
            {
                self.send(Message::Request(request)).await?;
                pending.push(request);
            }

            let was_choked = self.peer_choking;
            let message = timeout_at(deadline, self.recv())
                .await
                .with_context(|| format!("{}: piece {index} stalled", self.addr))??;

            if was_choked && !self.peer_choking {
                queue.extend(pending.drain(..));
                deadline = Instant::now() + STALL_TIMEOUT;
            }

            match message {
                Message::Piece(piece) if piece.index == index => {
                    let Some(position) = pending.iter().position(|request| {
                        request.begin == piece.begin
                            && usize::try_from(request.length)
                                .is_ok_and(|len| len == piece.block.len())
                    }) else {
                        continue;
                    };
                    pending.swap_remove(position);

                    let start = usize::try_from(piece.begin)?;
                    buf.get_mut(start..start + piece.block.len())
                        .context("Block out of piece bounds")?
                        .copy_from_slice(&piece.block);
                    deadline = Instant::now() + STALL_TIMEOUT;
                }
                _ => {}
            }
        }

        Ok(buf.freeze())
    }
}

async fn with_timeout(
    addr: SocketAddr,
    handshake: impl Future<Output = Result<PeerConnection>>,
) -> Result<PeerConnection> {
    timeout(HANDSHAKE_TIMEOUT, handshake)
        .await
        .with_context(|| format!("{addr}: handshake timed out"))?
        .with_context(|| format!("{addr}: handshake failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::testing::spawn_seeder;

    #[tokio::test]
    async fn connect_cases() {
        let info_hash = [7; 20];
        let our_id = *b"-RS0001-abcdefghijkl";
        let their_id = *b"-XX0001-000000000000";
        let data: Bytes = (0..=u8::MAX).cycle().take(80_000).collect();
        let connect = |addr| PeerConnection::connect(addr, info_hash, our_id, 2);
        let seeder = |hash| Some(Handshake::new(hash, their_id));

        let addr = spawn_seeder(seeder(info_hash), &[0b0100_0000], data.clone(), 40_000).await;
        let mut conn = connect(addr).await.unwrap();
        assert_eq!(conn.addr, addr);
        assert_eq!(conn.peer_id, their_id);
        assert!(conn.is_choked());

        while conn.is_choked() {
            conn.recv().await.unwrap();
        }
        assert!(conn.has_any_piece());
        assert!(conn.has_piece(1));
        assert!(!conn.has_piece(0));
        assert!(!conn.has_piece(2));
        assert_eq!(
            conn.download_piece(1, 40_000).await.unwrap(),
            data.slice(40_000..)
        );

        let addr = spawn_seeder(seeder([9; 20]), &[0], Bytes::new(), 0).await;
        assert!(connect(addr).await.is_err());

        let addr = spawn_seeder(
            Some(Handshake::new(info_hash, our_id)),
            &[0],
            Bytes::new(),
            0,
        )
        .await;
        assert!(connect(addr).await.is_err());

        let addr = spawn_seeder(None, &[0], Bytes::new(), 0).await;
        assert!(connect(addr).await.is_err());
    }
}
