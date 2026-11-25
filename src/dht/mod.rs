use anyhow::Result;
use futures_util::StreamExt;
use mainline::Id;
use mainline::async_dht::AsyncDht;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, info};

use crate::torrent::InfoHash;

const ANNOUNCE_INTERVAL: Duration = Duration::from_mins(15);
const RETRY_INTERVAL: Duration = Duration::from_mins(1);

#[derive(Debug, Clone)]
pub struct Dht(AsyncDht);

impl Dht {
    pub fn start() -> Result<Self> {
        Ok(Self(mainline::Dht::client()?.as_async()))
    }

    pub async fn announce_periodically(
        &self,
        info_hash: InfoHash,
        port: u16,
        peers: &mpsc::Sender<Vec<SocketAddr>>,
    ) {
        let id = Id::from(<[u8; 20]>::from(info_hash));

        loop {
            let found = self.lookup(id, peers).await;
            let delay = if found == 0 {
                RETRY_INTERVAL
            } else {
                ANNOUNCE_INTERVAL
            };
            info!("DHT: {found} peers, next lookup in {}s", delay.as_secs());
            self.announce(id, port).await;
            sleep(delay).await;
        }
    }

    async fn lookup(&self, id: Id, peers: &mpsc::Sender<Vec<SocketAddr>>) -> usize {
        let mut found = 0;
        let mut responses = self.0.get_peers(id);

        while let Some(batch) = responses.next().await {
            found += batch.len();
            let _ = peers
                .send(batch.into_iter().map(SocketAddr::V4).collect())
                .await;
        }

        found
    }

    async fn announce(&self, id: Id, port: u16) {
        if let Err(e) = self.0.announce_peer(id, Some(port)).await {
            debug!("DHT announce failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mainline::Testnet;
    use std::net::Ipv4Addr;

    #[tokio::test(flavor = "multi_thread")]
    async fn dht_cases() {
        let testnet = Testnet::builder(10).build().unwrap();
        let node = || {
            let dht = mainline::Dht::builder()
                .bootstrap(&testnet.bootstrap)
                .bind_address(Ipv4Addr::LOCALHOST)
                .build()
                .unwrap();
            Dht(dht.as_async())
        };
        let id = Id::from([7; 20]);

        node().announce(id, 6881).await;

        let (sender, mut receiver) = mpsc::channel(16);
        let found = node().lookup(id, &sender).await;
        drop(sender);

        let mut peers = Vec::new();
        while let Some(batch) = receiver.recv().await {
            peers.extend(batch);
        }
        assert_eq!(found, peers.len());
        assert!(peers.contains(&"127.0.0.1:6881".parse().unwrap()));
    }
}
