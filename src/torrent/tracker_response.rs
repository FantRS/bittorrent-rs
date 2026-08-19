use anyhow::{Result, anyhow};
use std::net::SocketAddrV4;

use crate::bencode::BencodeValue;

#[derive(Debug)]
pub struct TrackerResponse {
    pub interval: u64,
    pub peers: Vec<SocketAddrV4>,

    pub tracker_id: Option<Vec<u8>>,
    pub complete: Option<u64>,
    pub incomplete: Option<u64>,
}

impl TryFrom<BencodeValue> for TrackerResponse {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Dict(mut dict) = value else {
            return Err(anyhow!("Expected a dictionary for TrackerResponse"));
        };

        if let Some(e) = dict.remove(&b"failure reason".to_vec()) {
            let reason: String = e.try_into()?;
            return Err(anyhow!(reason));
        }

        let interval = dict
            .remove(&b"interval".to_vec())
            .ok_or_else(|| anyhow!("Missing 'interval' key in TrackerResponse"))?
            .try_into()?;

        let peers_bytes: Vec<u8> = dict
            .remove(&b"peers".to_vec())
            .ok_or_else(|| anyhow!("Missing 'peers' key in TrackerResponse"))?
            .try_into()?;

        let tracker_id = dict
            .remove(&b"tracker id".to_vec())
            .and_then(|b| b.try_into().ok());

        let complete = dict
            .remove(&b"complete".to_vec())
            .and_then(|b| b.try_into().ok());

        let incomplete = dict
            .remove(&b"incomplete".to_vec())
            .and_then(|b| b.try_into().ok());

        Ok(TrackerResponse {
            interval,
            peers: parse_peers(peers_bytes)?,
            tracker_id,
            complete,
            incomplete,
        })
    }
}

fn parse_peers(bytes: Vec<u8>) -> Result<Vec<SocketAddrV4>> {
    let chunks = bytes.chunks_exact(6);

    if !chunks.remainder().is_empty() {
        return Err(anyhow!("Peers array is not valid!"));
    }

    chunks
        .map(|chunk| {
            let ip: [u8; 4] = chunk[0..4].try_into().unwrap();
            let port = u16::from_be_bytes(chunk[4..6].try_into().unwrap());

            Ok(SocketAddrV4::new(ip.into(), port))
        })
        .collect()
}
