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
        if let BencodeValue::Dict(mut dict) = value {
            if let Some(e) = dict.remove(&b"failure reason".to_vec()) {
                return Err(anyhow!("{:?}", e));
            }

            let interval = dict
                .remove(&b"interval".to_vec())
                .ok_or_else(|| anyhow!("Missing 'interval' key in TrackerResponse"))?
                .try_into()?;

            let peers = dict
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
                peers,
                tracker_id,
                complete,
                incomplete,
            })
        } else {
            Err(anyhow!("Expected a dictionary for TrackerResponse"))
        }
    }
}
