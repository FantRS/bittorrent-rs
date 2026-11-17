use anyhow::{Context, Result, ensure};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::peer::Handshake;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct PeerConnection {
    pub addr: SocketAddr,
    pub peer_id: [u8; 20],
    pub stream: TcpStream,
}

impl PeerConnection {
    pub async fn connect(
        addr: impl Into<SocketAddr>,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Result<Self> {
        let addr = addr.into();

        timeout(HANDSHAKE_TIMEOUT, Self::establish(addr, info_hash, peer_id))
            .await
            .with_context(|| format!("{addr}: handshake timed out"))?
            .with_context(|| format!("{addr}: handshake failed"))
    }

    async fn establish(addr: SocketAddr, info_hash: [u8; 20], peer_id: [u8; 20]) -> Result<Self> {
        let mut stream = TcpStream::connect(addr).await?;
        let reply = Handshake::new(info_hash, peer_id)
            .exchange(&mut stream)
            .await?;

        ensure!(reply.info_hash == info_hash, "info_hash mismatch");

        Ok(Self {
            addr,
            peer_id: reply.peer_id,
            stream,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn spawn_peer(reply: Option<Handshake>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            if let Some(reply) = reply {
                let _ = reply.exchange(&mut socket).await;
            }
        });

        addr
    }

    #[tokio::test]
    async fn connect_cases() {
        let info_hash = [7; 20];
        let our_id = *b"-RS0001-abcdefghijkl";
        let their_id = *b"-XX0001-000000000000";
        let connect = |addr| PeerConnection::connect(addr, info_hash, our_id);

        let addr = spawn_peer(Some(Handshake::new(info_hash, their_id))).await;
        let conn = connect(addr).await.unwrap();
        assert_eq!(conn.addr, addr);
        assert_eq!(conn.peer_id, their_id);

        let addr = spawn_peer(Some(Handshake::new([9; 20], their_id))).await;
        assert!(connect(addr).await.is_err());

        let addr = spawn_peer(None).await;
        assert!(connect(addr).await.is_err());
    }
}
