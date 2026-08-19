use anyhow::{Result, anyhow};
use std::net::TcpListener;

use crate::torrent::Torrent;

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

    pub fn format_url(&self, torrent: &Torrent) -> Result<String> {
        let base_url = torrent.meta.announce.as_str();
        let info_hash = urlencoding::encode_binary(&torrent.info_hash);
        let peer_id = urlencoding::encode_binary(&self.id);
        let port = self.lst.local_addr()?.port();

        Ok(format!(
            "{}?info_hash={}&peer_id={}&port={}",
            base_url, info_hash, peer_id, port
        ))
    }

    fn generate_id() -> [u8; 20] {
        let mut id = [0; 20];
        id[..8].copy_from_slice(b"-RS0001-");

        let rands: [u8; 12] = rand::random();
        id[8..].copy_from_slice(&rands);

        id
    }

    fn generate_lst() -> Result<TcpListener> {
        for port in 6881..=6889 {
            let addr = format!("0.0.0.0:{}", port);
            match TcpListener::bind(addr) {
                Ok(lst) => return Ok(lst),
                Err(_) => continue,
            }
        }

        Err(anyhow!("Could not bind to any port in range 6881-6889"))
    }
}
