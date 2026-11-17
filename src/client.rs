use anyhow::{Result, anyhow};
use std::net::{self, Ipv4Addr};
use tokio::net::TcpListener;

use crate::{
    bencode::BencodeParser,
    torrent::{Torrent, TrackerResponse},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Started,
    Completed,
    Stopped,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Announce {
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub event: Option<Event>,
}

#[derive(Debug)]
pub struct Client {
    pub id: [u8; 20],
    pub lst: TcpListener,
}

impl Client {
    pub fn build() -> Result<Self> {
        let id = Self::generate_id();
        let lst = Self::generate_lst()?;

        Ok(Self { id, lst })
    }

    pub async fn send_tracker_req(
        &self,
        torrent: &Torrent,
        announce: Announce,
    ) -> Result<TrackerResponse> {
        let tracker_url = self.build_url(torrent, announce)?;

        let body = reqwest::get(tracker_url).await?.bytes().await?;

        BencodeParser::new(body.to_vec()).parse()?.try_into()
    }

    fn generate_id() -> [u8; 20] {
        let mut id: [u8; 20] = rand::random();
        id[..8].copy_from_slice(b"-RS0001-");
        id
    }

    fn generate_lst() -> Result<TcpListener> {
        let listener = (6881..=6889)
            .find_map(|port| net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).ok())
            .ok_or_else(|| anyhow!("Could not bind to any port in range 6881-6889"))?;
        listener.set_nonblocking(true)?;

        Ok(TcpListener::from_std(listener)?)
    }

    fn build_url(&self, torrent: &Torrent, announce: Announce) -> Result<String> {
        let base_url = torrent.meta.announce.as_str();
        let info_hash = urlencoding::encode_binary(&torrent.info_hash);
        let peer_id = urlencoding::encode_binary(&self.id);
        let port = self.lst.local_addr()?.port();
        let Announce {
            uploaded,
            downloaded,
            left,
            event,
        } = announce;
        let compact = 1;
        let numwant = 100;

        let mut url = format!(
            "{base_url}?info_hash={info_hash}&peer_id={peer_id}&port={port}&left={left}&uploaded={uploaded}&downloaded={downloaded}&compact={compact}&numwant={numwant}"
        );
        if let Some(event) = event {
            url.push_str("&event=");
            url.push_str(event.as_str());
        }

        Ok(url)
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
