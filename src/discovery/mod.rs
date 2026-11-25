use futures_util::future::{join, join_all};
use std::net::SocketAddr;
use tokio::sync::mpsc;

use crate::dht::Dht;
use crate::tracker::{Announce, Event, TrackerClient};

#[derive(Debug)]
pub struct Discovery {
    trackers: TrackerClient,
    dht: Option<Dht>,
}

impl Discovery {
    #[must_use]
    pub fn new(trackers: TrackerClient, dht: Option<Dht>) -> Self {
        Self { trackers, dht }
    }

    #[must_use]
    pub fn has_dht(&self) -> bool {
        self.dht.is_some()
    }

    pub async fn discover(
        &self,
        trackers: &[String],
        request: impl Fn(Option<Event>) -> Announce + Copy,
        peers: &mpsc::Sender<Vec<SocketAddr>>,
    ) {
        let trackers = join_all(
            trackers
                .iter()
                .map(|tracker| self.trackers.announce_periodically(tracker, request, peers)),
        );
        let dht = async {
            if let Some(dht) = &self.dht {
                let Announce {
                    info_hash, port, ..
                } = request(None);
                dht.announce_periodically(info_hash, port, peers).await;
            }
        };

        join(trackers, dht).await;
    }

    pub async fn announce_all(&self, trackers: &[String], request: Announce) {
        self.trackers.announce_all(trackers, request).await;
    }
}
