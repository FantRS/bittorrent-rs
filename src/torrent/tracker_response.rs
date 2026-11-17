use anyhow::{Context, Result, bail, ensure};
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

        let peers = match dict
            .remove(b"peers".as_slice())
            .context("Missing 'peers' key")?
        {
            BencodeValue::Bytes(bytes) => parse_compact_peers(&bytes)?,
            BencodeValue::List(peers) => peers.into_iter().filter_map(parse_peer).collect(),
            other => bail!("Invalid 'peers' value: {other:?}"),
        };

        Ok(Self {
            interval: dict.take("interval")?,
            peers,
            tracker_id: dict.take_opt("tracker id"),
            complete: dict.take_opt("complete"),
            incomplete: dict.take_opt("incomplete"),
        })
    }
}

fn parse_compact_peers(bytes: &[u8]) -> Result<Vec<SocketAddrV4>> {
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

fn parse_peer(value: BencodeValue) -> Option<SocketAddrV4> {
    let mut dict = Dict::try_from(value).ok()?;
    let ip: String = dict.take_opt("ip")?;
    let port: u64 = dict.take_opt("port")?;

    Some(SocketAddrV4::new(
        ip.parse().ok()?,
        u16::try_from(port).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bencode::BencodeParser;

    fn parse(bytes: &[u8]) -> Result<TrackerResponse> {
        BencodeParser::new(bytes.to_vec()).parse()?.try_into()
    }

    #[test]
    fn response_cases() {
        let compact =
            parse(b"d8:intervali1800e5:peers12:\x7f\0\0\x01\x1a\xe1\x0a\0\0\x02\0\x50e").unwrap();
        assert_eq!(compact.interval, 1800);
        assert_eq!(
            compact.peers,
            [
                "127.0.0.1:6881".parse().unwrap(),
                "10.0.0.2:80".parse().unwrap()
            ]
        );

        let list =
            parse(b"d8:intervali60e5:peersld2:ip9:127.0.0.14:porti6881eed2:ip3:::14:porti1eeee")
                .unwrap();
        assert_eq!(list.peers, ["127.0.0.1:6881".parse().unwrap()]);

        let empty = parse(b"d8:intervali60e5:peerslee").unwrap();
        assert!(empty.peers.is_empty());

        assert!(parse(b"d8:intervali60e5:peers5:\0\0\0\0\0e").is_err());
        assert!(parse(b"d8:intervali60e5:peersi1ee").is_err());

        let failure = parse(b"d14:failure reason9:not founde").unwrap_err();
        assert_eq!(failure.to_string(), "not found");
    }
}
