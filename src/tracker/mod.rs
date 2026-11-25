mod http;
mod response;
mod udp;

pub use response::TrackerResponse;

use anyhow::{Context, Result, bail};
use futures_util::future::join_all;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};
use url::Url;

use crate::peer::PeerId;
use crate::torrent::InfoHash;

const TRACKER_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_INTERVAL: Duration = Duration::from_mins(1);
const RETRY_INTERVAL: Duration = Duration::from_mins(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Started,
    Completed,
    Stopped,
}

impl Event {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Announce {
    pub info_hash: InfoHash,
    pub peer_id: PeerId,
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub event: Option<Event>,
}

#[derive(Debug, Clone)]
pub struct TrackerClient {
    http: reqwest::Client,
}

impl TrackerClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder().build()?,
        })
    }

    pub async fn announce(&self, tracker: &str, request: &Announce) -> Result<TrackerResponse> {
        let url = Url::parse(tracker)?;

        timeout(TRACKER_TIMEOUT, async {
            match url.scheme() {
                "http" | "https" => http::announce(&self.http, url.clone(), request).await,
                "udp" => udp::announce(&url, request).await,
                scheme => bail!("Unsupported tracker scheme: {scheme}"),
            }
        })
        .await
        .context("Tracker timed out")?
    }

    pub async fn announce_all(&self, trackers: &[String], request: Announce) {
        join_all(trackers.iter().map(|tracker| async move {
            if let Err(e) = self.announce(tracker, &request).await {
                warn!("{tracker}: {e:#}");
            }
        }))
        .await;
    }

    pub async fn announce_periodically(
        &self,
        tracker: &str,
        request: impl Fn(Option<Event>) -> Announce,
        peers: &mpsc::Sender<Vec<SocketAddr>>,
    ) {
        let mut event = Some(Event::Started);

        loop {
            let delay = match self.announce(tracker, &request(event)).await {
                Ok(response) => {
                    let interval = response.interval.max(MIN_INTERVAL);
                    info!(
                        "{tracker}: {} peers, next announce in {}s",
                        response.peers.len(),
                        interval.as_secs()
                    );
                    let _ = peers.send(response.peers).await;
                    event = None;
                    interval
                }
                Err(e) => {
                    warn!("{tracker}: {e:#}");
                    RETRY_INTERVAL
                }
            };

            sleep(delay).await;
        }
    }
}
