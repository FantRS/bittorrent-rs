use anyhow::{Context, Result, bail};
use std::net::SocketAddr;
use std::time::Duration;
use tracing::warn;

use crate::bencode::{BencodeValue, Dict, DictExt};
use crate::peer::compact::parse_compact_peers;

#[derive(Debug)]
pub struct TrackerResponse {
    pub interval: Duration,
    pub peers: Vec<SocketAddr>,
}

impl TryFrom<BencodeValue> for TrackerResponse {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let mut dict = Dict::try_from(value)?;

        if let Some(reason) = dict.take_opt::<String>("failure reason") {
            bail!(reason);
        }

        let mut peers = match dict
            .remove(b"peers".as_slice())
            .context("Missing 'peers' key")?
        {
            BencodeValue::Bytes(bytes) => parse_compact_peers::<[u8; 4]>(&bytes)?,
            BencodeValue::List(peers) => peers.into_iter().filter_map(parse_peer).collect(),
            other => bail!("Invalid 'peers' value: {other:?}"),
        };
        if let Some(bytes) = dict.take_opt::<Vec<u8>>("peers6") {
            match parse_compact_peers::<[u8; 16]>(&bytes) {
                Ok(peers6) => peers.extend(peers6),
                Err(e) => warn!("Ignoring 'peers6': {e:#}"),
            }
        }

        Ok(Self {
            interval: Duration::from_secs(dict.take("interval")?),
            peers,
        })
    }
}

fn parse_peer(value: BencodeValue) -> Option<SocketAddr> {
    let mut dict = Dict::try_from(value).ok()?;
    let ip: String = dict.take_opt("ip")?;
    let port: u64 = dict.take_opt("port")?;

    Some(SocketAddr::new(ip.parse().ok()?, u16::try_from(port).ok()?))
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
        assert_eq!(compact.interval, Duration::from_mins(30));
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
        assert_eq!(
            list.peers,
            [
                "127.0.0.1:6881".parse().unwrap(),
                "[::1]:1".parse().unwrap()
            ]
        );

        let dual = parse(
            b"d8:intervali60e5:peers6:\x7f\0\0\x01\x1a\xe16:peers618:\x20\x01\x0d\xb8\0\0\0\0\0\0\0\0\0\0\0\x01\x1a\xe1e",
        )
        .unwrap();
        assert_eq!(
            dual.peers,
            [
                "127.0.0.1:6881".parse().unwrap(),
                "[2001:db8::1]:6881".parse().unwrap()
            ]
        );
        let broken6 =
            parse(b"d8:intervali60e5:peers6:\x7f\0\0\x01\x1a\xe16:peers65:\0\0\0\0\0e").unwrap();
        assert_eq!(broken6.peers, ["127.0.0.1:6881".parse().unwrap()]);

        let empty = parse(b"d8:intervali60e5:peerslee").unwrap();
        assert!(empty.peers.is_empty());

        assert!(parse(b"d8:intervali60e5:peers5:\0\0\0\0\0e").is_err());
        assert!(parse(b"d8:intervali60e5:peersi1ee").is_err());

        let failure = parse(b"d14:failure reason9:not founde").unwrap_err();
        assert_eq!(failure.to_string(), "not found");
    }
}
