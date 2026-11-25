use anyhow::{Context, Result, anyhow, bail, ensure};
use data_encoding::{BASE32, HEXLOWER_PERMISSIVE};
use std::str::FromStr;
use url::Url;

use crate::torrent::InfoHash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magnet {
    pub info_hash: InfoHash,
    pub name: Option<String>,
    pub trackers: Vec<String>,
    pub peers: Vec<String>,
}

impl FromStr for Magnet {
    type Err = anyhow::Error;

    fn from_str(link: &str) -> Result<Self> {
        let url = Url::parse(link)?;
        ensure!(url.scheme() == "magnet", "Not a magnet link: {link}");

        let mut info_hash = None;
        let mut name = None;
        let mut trackers = Vec::new();
        let mut peers = Vec::new();

        for (key, value) in url.query_pairs() {
            match &*key {
                "xt" => {
                    if let Some(hash) = value.strip_prefix("urn:btih:") {
                        info_hash = Some(parse_info_hash(hash)?);
                    }
                }
                "dn" => name = Some(value.into_owned()),
                "tr" => trackers.push(value.into_owned()),
                "x.pe"
                    if value
                        .rsplit_once(':')
                        .is_some_and(|(_, port)| port.parse::<u16>().is_ok()) =>
                {
                    peers.push(value.into_owned());
                }
                _ => {}
            }
        }

        Ok(Self {
            info_hash: info_hash.context("Magnet link has no BitTorrent info hash")?,
            name,
            trackers,
            peers,
        })
    }
}

fn parse_info_hash(hash: &str) -> Result<InfoHash> {
    let bytes = match hash.len() {
        40 => HEXLOWER_PERMISSIVE.decode(hash.as_bytes())?,
        32 => BASE32.decode(hash.to_ascii_uppercase().as_bytes())?,
        len => bail!("Invalid info hash length: {len}"),
    };

    <[u8; 20]>::try_from(bytes)
        .map(InfoHash::from)
        .map_err(|_| anyhow!("Invalid info hash: {hash}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: [u8; 20] = [
        0xda, 0xfc, 0x8c, 0x07, 0x6c, 0xa2, 0xf3, 0xed, 0x37, 0x6e, 0xea, 0xe7, 0xc7, 0x6a, 0x0d,
        0x6b, 0xe2, 0x41, 0x5c, 0x45,
    ];

    #[test]
    fn magnet_cases() {
        let magnet: Magnet = "magnet:?xt=urn:btih:DAFC8C076CA2F3ED376EEAE7C76A0D6BE2415C45\
             &dn=ubuntu+26.04&tr=udp%3A%2F%2Ftracker.example%3A1337%2Fannounce\
             &tr=https%3A%2F%2Ft.example%2Fannounce&x.pe=10.0.0.1:6881&x.pe=peer.example:51413&x.pe=bad&x.pe=host:port"
            .parse()
            .unwrap();
        assert_eq!(
            magnet,
            Magnet {
                info_hash: HASH.into(),
                name: Some("ubuntu 26.04".into()),
                trackers: vec![
                    "udp://tracker.example:1337/announce".into(),
                    "https://t.example/announce".into()
                ],
                peers: vec!["10.0.0.1:6881".into(), "peer.example:51413".into()],
            }
        );

        let base32: Magnet =
            "magnet:?xt=urn:btmh:1220aa&xt=urn:btih:3l6iyb3mulz62n3o5lt4o2qnnprecxcf"
                .parse()
                .unwrap();
        assert_eq!(base32.info_hash, HASH.into());
        assert!(base32.name.is_none() && base32.trackers.is_empty());

        assert!("magnet:?dn=x".parse::<Magnet>().is_err());
        assert!("magnet:?xt=urn:btih:abc".parse::<Magnet>().is_err());
        assert!(
            "magnet:?xt=urn:btih:zzfc8c076ca2f3ed376eeae7c76a0d6be2415c45"
                .parse::<Magnet>()
                .is_err()
        );
        assert!(
            "https://example.com/?xt=urn:btih:dafc8c076ca2f3ed376eeae7c76a0d6be2415c45"
                .parse::<Magnet>()
                .is_err()
        );
    }
}
