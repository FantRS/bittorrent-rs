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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Started,
    Completed,
    Stopped,
}

const TRACKER_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_INTERVAL: Duration = Duration::from_mins(1);
const RETRY_INTERVAL: Duration = Duration::from_mins(5);

#[derive(Debug, Clone, Copy)]
pub struct Announce {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub event: Option<Event>,
}

pub async fn announce(tracker: &str, request: &Announce) -> Result<TrackerResponse> {
    let url = Url::parse(tracker)?;

    timeout(TRACKER_TIMEOUT, async {
        match url.scheme() {
            "http" | "https" => http::announce(url.clone(), request).await,
            "udp" => udp::announce(&url, request).await,
            scheme => bail!("Unsupported tracker scheme: {scheme}"),
        }
    })
    .await
    .context("Tracker timed out")?
}

pub async fn announce_all(trackers: &[String], request: Announce) {
    join_all(trackers.iter().map(|tracker| async move {
        if let Err(e) = announce(tracker, &request).await {
            warn!("{tracker}: {e:#}");
        }
    }))
    .await;
}

pub async fn announce_periodically(
    tracker: &str,
    request: impl Fn(Option<Event>) -> Announce,
    peers: &mpsc::Sender<SocketAddr>,
) {
    let mut event = Some(Event::Started);

    loop {
        let delay = match announce(tracker, &request(event)).await {
            Ok(response) => {
                let interval = Duration::from_secs(response.interval).max(MIN_INTERVAL);
                info!(
                    "{tracker}: {} peers, next announce in {}s",
                    response.peers.len(),
                    interval.as_secs()
                );
                for peer in response.peers {
                    let _ = peers.try_send(peer);
                }
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
