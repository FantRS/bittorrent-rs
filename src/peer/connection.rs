use anyhow::{Context, Result, anyhow, bail, ensure};
use bitvec::prelude::{BitVec, Msb0};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::Framed;

use crate::peer::{Handshake, Message, MessageCodec, Piece, Request};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const BLOCK_LEN: u32 = 1 << 14;
const MAX_PENDING_REQUESTS: usize = 5;

#[derive(Debug)]
pub struct PeerConnection {
    pub addr: SocketAddr,
    pub peer_id: [u8; 20],
    frames: Framed<TcpStream, MessageCodec>,
    choked: bool,
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

        timeout(
            HANDSHAKE_TIMEOUT,
            Self::establish(addr, info_hash, peer_id, piece_count),
        )
        .await
        .with_context(|| format!("{addr}: handshake timed out"))?
        .with_context(|| format!("{addr}: handshake failed"))
    }

    async fn establish(
        addr: SocketAddr,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
        piece_count: usize,
    ) -> Result<Self> {
        let mut stream = TcpStream::connect(addr).await?;
        let reply = Handshake::new(info_hash, peer_id)
            .exchange(&mut stream)
            .await?;

        ensure!(reply.info_hash == info_hash, "info_hash mismatch");

        Ok(Self {
            addr,
            peer_id: reply.peer_id,
            frames: Framed::new(stream, MessageCodec),
            choked: true,
            pieces: BitVec::repeat(false, piece_count),
        })
    }

    #[must_use]
    pub fn is_choked(&self) -> bool {
        self.choked
    }

    pub fn pieces(&self) -> impl Iterator<Item = u32> + '_ {
        self.pieces
            .iter_ones()
            .filter_map(|index| u32::try_from(index).ok())
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
            Message::Choke => self.choked = true,
            Message::Unchoke => self.choked = false,
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
        let mut queue = (0..length.div_ceil(BLOCK_LEN)).map(|block| {
            let begin = block * BLOCK_LEN;
            Request {
                index,
                begin,
                length: BLOCK_LEN.min(length - begin),
            }
        });
        let mut pending = Vec::with_capacity(MAX_PENDING_REQUESTS);
        let mut piece = BytesMut::zeroed(usize::try_from(length)?);

        loop {
            while pending.len() < MAX_PENDING_REQUESTS
                && let Some(request) = queue.next()
            {
                self.send(Message::Request(request)).await?;
                pending.push(request);
            }

            if pending.is_empty() {
                return Ok(piece.freeze());
            }

            match self.recv().await? {
                Message::Piece(Piece {
                    index: i,
                    begin,
                    block,
                }) if i == index => {
                    let Some(position) = pending.iter().position(|request| {
                        request.begin == begin
                            && usize::try_from(request.length).is_ok_and(|len| len == block.len())
                    }) else {
                        continue;
                    };
                    pending.swap_remove(position);

                    let start = usize::try_from(begin)?;
                    piece
                        .get_mut(start..start + block.len())
                        .ok_or_else(|| anyhow!("Block out of piece bounds"))?
                        .copy_from_slice(&block);
                }
                Message::Choke => bail!("{}: choked during piece {index}", self.addr),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn spawn_seeder(reply: Option<Handshake>, piece: Bytes) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let Some(reply) = reply else { return };
            if reply.exchange(&mut socket).await.is_err() {
                return;
            }

            let mut frames = Framed::new(socket, MessageCodec);
            frames
                .send(Message::Bitfield(Bytes::from_static(&[0b0100_0000])))
                .await
                .unwrap();
            frames.send(Message::Unchoke).await.unwrap();

            while let Some(Ok(Message::Request(request))) = frames.next().await {
                let start = usize::try_from(request.begin).unwrap();
                let end = start + usize::try_from(request.length).unwrap();
                let block = piece.slice(start..end);
                frames
                    .send(Message::Piece(Piece {
                        index: request.index,
                        begin: request.begin,
                        block,
                    }))
                    .await
                    .unwrap();
            }
        });

        addr
    }

    #[tokio::test]
    async fn connect_cases() {
        let info_hash = [7; 20];
        let our_id = *b"-RS0001-abcdefghijkl";
        let their_id = *b"-XX0001-000000000000";
        let piece: Bytes = (0..=u8::MAX).cycle().take(40_000).collect();
        let connect = |addr| PeerConnection::connect(addr, info_hash, our_id, 2);

        let addr = spawn_seeder(Some(Handshake::new(info_hash, their_id)), piece.clone()).await;
        let mut conn = connect(addr).await.unwrap();
        assert_eq!(conn.addr, addr);
        assert_eq!(conn.peer_id, their_id);
        assert!(conn.is_choked());

        while conn.is_choked() {
            conn.recv().await.unwrap();
        }
        assert_eq!(conn.pieces().collect::<Vec<_>>(), [1]);
        assert_eq!(conn.download_piece(1, 40_000).await.unwrap(), piece);

        let addr = spawn_seeder(Some(Handshake::new([9; 20], their_id)), Bytes::new()).await;
        assert!(connect(addr).await.is_err());

        let addr = spawn_seeder(None, Bytes::new()).await;
        assert!(connect(addr).await.is_err());
    }
}
