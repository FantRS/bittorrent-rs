use anyhow::{Context, Result, anyhow, bail, ensure};
use bitvec::prelude::{BitVec, Msb0};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until, timeout};
use tokio_util::codec::Framed;
use tracing::debug;

use crate::peer::extension::{self, MetadataMessage, RemoteExtensions};
use crate::peer::pipeline::Pipeline;
use crate::peer::{Handshake, Message, MessageCodec, PeerId, Request};
use crate::torrent::{InfoHash, PieceIndex};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(90);

#[derive(Debug)]
pub struct PeerConnection {
    pub addr: SocketAddr,
    handshake: Handshake,
    frames: Framed<TcpStream, MessageCodec>,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
    pieces: Option<BitVec<u8, Msb0>>,
    remote: Option<RemoteExtensions>,
    exchanged: Option<mpsc::Sender<Vec<SocketAddr>>>,
    pipeline: Pipeline,
    last_sent: Instant,
}

impl PeerConnection {
    pub async fn connect(
        addr: impl Into<SocketAddr>,
        info_hash: InfoHash,
        peer_id: PeerId,
        piece_count: Option<usize>,
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

            Ok(Self::new(addr, reply, stream, piece_count))
        })
        .await
    }

    pub async fn accept(
        mut stream: TcpStream,
        info_hash: InfoHash,
        peer_id: PeerId,
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

            Ok(Self::new(addr, theirs, stream, Some(piece_count)))
        })
        .await
    }

    fn new(
        addr: SocketAddr,
        handshake: Handshake,
        stream: TcpStream,
        piece_count: Option<usize>,
    ) -> Self {
        Self {
            addr,
            handshake,
            frames: Framed::new(stream, MessageCodec),
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
            pieces: piece_count.map(|count| BitVec::repeat(false, count)),
            remote: None,
            exchanged: None,
            pipeline: Pipeline::new(),
            last_sent: Instant::now(),
        }
    }

    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        self.handshake.peer_id
    }

    pub async fn enable_extensions(
        &mut self,
        exchanged: mpsc::Sender<Vec<SocketAddr>>,
    ) -> Result<()> {
        if self.supports_extensions() {
            self.send(Message::Extended {
                id: extension::HANDSHAKE,
                payload: extension::handshake(),
            })
            .await?;
            self.exchanged = Some(exchanged);
        }

        Ok(())
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
    pub fn has_piece(&self, index: PieceIndex) -> bool {
        self.pieces
            .as_ref()
            .and_then(|pieces| pieces.get(index.as_usize()))
            .is_some_and(|bit| *bit)
    }

    #[must_use]
    pub fn supports_extensions(&self) -> bool {
        self.handshake.supports_extensions()
    }

    pub async fn metadata_size(&mut self) -> Result<usize> {
        loop {
            match self.remote {
                Some(RemoteExtensions {
                    ut_metadata: None, ..
                }) => bail!("Peer does not support ut_metadata"),
                Some(RemoteExtensions {
                    metadata_size: Some(size),
                    ..
                }) => return Ok(size),
                _ => {}
            }
            self.recv().await?;
        }
    }

    pub async fn request_metadata(&mut self, piece: u32) -> Result<()> {
        let id = self
            .remote
            .and_then(|remote| remote.ut_metadata)
            .context("Peer does not support ut_metadata")?;

        self.send(Message::Extended {
            id,
            payload: MetadataMessage::Request(piece).encode(),
        })
        .await
    }

    pub async fn recv_metadata(&mut self) -> Result<(u32, Bytes)> {
        loop {
            if let Message::Extended {
                id: extension::UT_METADATA,
                payload,
            } = self.recv().await?
            {
                match MetadataMessage::decode(&payload)? {
                    MetadataMessage::Data { piece, block } => return Ok((piece, block)),
                    MetadataMessage::Reject(piece) => bail!("Peer rejected metadata piece {piece}"),
                    MetadataMessage::Request(_) => {}
                }
            }
        }
    }

    pub async fn interested(&mut self) -> Result<()> {
        if !self.am_interested {
            self.send(Message::Interested).await?;
            self.am_interested = true;
        }

        Ok(())
    }

    pub async fn send(&mut self, message: Message) -> Result<()> {
        self.frames.send(message).await?;
        self.last_sent = Instant::now();

        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Message> {
        let message = loop {
            tokio::select! {
                frame = self.frames.next() => {
                    break frame.ok_or_else(|| anyhow!("{}: connection closed", self.addr))??;
                }
                () = sleep_until(self.last_sent + KEEP_ALIVE_INTERVAL) => {
                    self.send(Message::KeepAlive).await?;
                }
            }
        };

        match &message {
            Message::Choke => self.peer_choking = true,
            Message::Unchoke => self.peer_choking = false,
            Message::Interested => self.peer_interested = true,
            Message::NotInterested => self.peer_interested = false,
            Message::Have(index) => {
                if let Some(pieces) = &mut self.pieces {
                    let mut bit = pieces
                        .get_mut(index.as_usize())
                        .ok_or_else(|| anyhow!("Invalid piece index: {index}"))?;
                    *bit = true;
                }
            }
            Message::Piece(piece) => self.pipeline.record_block(piece.block.len()),
            Message::Bitfield(bits) => {
                if let Some(pieces) = &mut self.pieces {
                    let piece_count = pieces.len();
                    ensure!(
                        bits.len() == piece_count.div_ceil(8),
                        "Invalid bitfield length: {}",
                        bits.len()
                    );
                    *pieces = BitVec::from_slice(bits);
                    pieces.truncate(piece_count);
                }
            }
            Message::Extended {
                id: extension::HANDSHAKE,
                payload,
            } => match extension::parse_handshake(payload) {
                Ok(remote) => {
                    self.pipeline.limit_to(remote.reqq);
                    self.remote = Some(remote);
                }
                Err(e) => debug!("{}: invalid extension handshake: {e:#}", self.addr),
            },
            Message::Extended {
                id: extension::UT_METADATA,
                payload,
            } => {
                if let Ok(MetadataMessage::Request(piece)) = MetadataMessage::decode(payload)
                    && let Some(id) = self.remote.and_then(|remote| remote.ut_metadata)
                {
                    self.send(Message::Extended {
                        id,
                        payload: MetadataMessage::Reject(piece).encode(),
                    })
                    .await?;
                }
            }
            Message::Extended {
                id: extension::UT_PEX,
                payload,
            } => {
                if let Some(exchanged) = &self.exchanged {
                    match extension::parse_pex(payload) {
                        Ok(peers) => {
                            debug!("{}: {} peers via PEX", self.addr, peers.len());
                            if !peers.is_empty() {
                                let _ = exchanged.send(peers).await;
                            }
                        }
                        Err(e) => debug!("{}: invalid PEX message: {e:#}", self.addr),
                    }
                }
            }
            _ => {}
        }

        Ok(message)
    }

    #[must_use]
    pub fn queue_depth(&self) -> usize {
        self.pipeline.depth()
    }

    pub async fn request(&mut self, requests: &[Request]) -> Result<()> {
        for &request in requests {
            self.frames.feed(Message::Request(request)).await?;
        }
        self.frames.flush().await?;
        self.last_sent = Instant::now();

        Ok(())
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
    use crate::peer::BLOCK_LEN;
    use crate::peer::testing::spawn_seeder;
    use std::collections::BTreeMap;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn connect_cases() {
        let info_hash = InfoHash::from([7; 20]);
        let our_id = PeerId::from(*b"-RS0001-abcdefghijkl");
        let their_id = PeerId::from(*b"-XX0001-000000000000");
        let data: Bytes = (0..=u8::MAX).cycle().take(80_000).collect();
        let connect = |addr| PeerConnection::connect(addr, info_hash, our_id, Some(2));
        let seeder = |hash| Some(Handshake::new(hash, their_id));

        let addr = spawn_seeder(seeder(info_hash), &[0b0100_0000], data.clone(), 40_000).await;
        let mut conn = connect(addr).await.unwrap();
        assert_eq!(conn.addr, addr);
        assert_eq!(conn.peer_id(), their_id);
        assert!(conn.is_choked());

        while conn.is_choked() {
            conn.recv().await.unwrap();
        }
        assert!(conn.has_piece(1.into()));
        assert!(!conn.has_piece(0.into()));
        assert!(!conn.has_piece(2.into()));
        let requests: Vec<Request> = (0..40_000)
            .step_by(BLOCK_LEN as usize)
            .map(|begin| Request {
                index: 1.into(),
                begin,
                length: BLOCK_LEN.min(40_000 - begin),
            })
            .collect();

        conn.request(&requests).await.unwrap();
        let mut blocks = BTreeMap::new();

        while blocks.len() < requests.len() {
            match conn.recv().await.unwrap() {
                Message::Piece(piece) => {
                    blocks.insert(piece.begin, piece.block);
                }
                Message::Unchoke => {
                    let missing: Vec<Request> = requests
                        .iter()
                        .filter(|request| !blocks.contains_key(&request.begin))
                        .copied()
                        .collect();
                    conn.request(&missing).await.unwrap();
                }
                _ => {}
            }
        }
        assert_eq!(
            blocks.values().flatten().copied().collect::<Bytes>(),
            data.slice(40_000..)
        );
        assert!(conn.queue_depth() > 5);

        let addr = spawn_seeder(seeder([9; 20].into()), &[0], Bytes::new(), 0).await;
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

    #[tokio::test]
    async fn exchange_cases() {
        let info_hash = InfoHash::from([7; 20]);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let remote = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let ours = Handshake::read_from(&mut stream).await.unwrap();
            Handshake::new(info_hash, [1; 20].into())
                .write_to(&mut stream)
                .await
                .unwrap();

            let mut frames = Framed::new(stream, MessageCodec);
            let handshake = frames.next().await.unwrap().unwrap();
            frames
                .send(Message::Extended {
                    id: extension::UT_PEX,
                    payload: Bytes::from_static(b"d5:added6:\x01\x02\x03\x04\x00\x05e"),
                })
                .await
                .unwrap();
            frames.send(Message::Unchoke).await.unwrap();

            (ours.supports_extensions(), handshake)
        });

        let mut conn = PeerConnection::connect(addr, info_hash, [2; 20].into(), Some(1))
            .await
            .unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        conn.enable_extensions(sender).await.unwrap();
        while conn.is_choked() {
            conn.recv().await.unwrap();
        }

        assert_eq!(receiver.try_recv().unwrap(), ["1.2.3.4:5".parse().unwrap()]);
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            remote.await.unwrap(),
            (
                true,
                Message::Extended {
                    id: extension::HANDSHAKE,
                    payload: extension::handshake()
                }
            )
        );
    }
}
