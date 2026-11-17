use anyhow::{Result, bail, ensure};
use std::net::{Ipv4Addr, SocketAddrV4};

use crate::bencode::{BencodeValue, Dict, DictExt};

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
        let mut dict = Dict::try_from(value)?;

        if let Some(reason) = dict.take_opt::<String>("failure reason") {
            bail!(reason);
        }

        let peers: Vec<u8> = dict.take("peers")?;

        Ok(Self {
            interval: dict.take("interval")?,
            peers: parse_peers(&peers)?,
            tracker_id: dict.take_opt("tracker id"),
            complete: dict.take_opt("complete"),
            incomplete: dict.take_opt("incomplete"),
        })
    }
}

fn parse_peers(bytes: &[u8]) -> Result<Vec<SocketAddrV4>> {
    let (chunks, rest) = bytes.as_chunks::<6>();
    ensure!(
        rest.is_empty(),
        "Compact peers length is not a multiple of 6"
    );

    Ok(chunks
        .iter()
        .map(|&[a, b, c, d, hi, lo]| {
            SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), u16::from_be_bytes([hi, lo]))
        })
        .collect())
}
