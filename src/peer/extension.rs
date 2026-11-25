use anyhow::Result;
use bytes::Bytes;
use std::net::SocketAddr;

use crate::bencode::{BencodeParser, BencodeValue, Dict, DictExt};
use crate::peer::compact::parse_compact_peers;

pub const HANDSHAKE: u8 = 0;
pub const UT_PEX: u8 = 1;

pub fn handshake() -> Bytes {
    let extensions = Dict::from([(b"ut_pex".to_vec(), BencodeValue::Int(UT_PEX.into()))]);
    let handshake = Dict::from([(b"m".to_vec(), BencodeValue::Dict(extensions))]);

    BencodeValue::Dict(handshake).encode().into()
}

pub fn parse_pex(payload: &[u8]) -> Result<Vec<SocketAddr>> {
    let mut dict = Dict::try_from(BencodeParser::new(payload.to_vec()).parse()?)?;
    let added = dict
        .take_opt::<Vec<u8>>("added")
        .and_then(|bytes| parse_compact_peers::<[u8; 4]>(&bytes).ok());
    let added6 = dict
        .take_opt::<Vec<u8>>("added6")
        .and_then(|bytes| parse_compact_peers::<[u8; 16]>(&bytes).ok());

    Ok(added.into_iter().chain(added6).flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_cases() {
        assert_eq!(&handshake()[..], b"d1:md6:ut_pexi1eee");

        let pex = parse_pex(
            b"d5:added12:\x01\x02\x03\x04\x00\x05\x0a\0\0\x01\x1a\xe17:added.f2:\0\x106:added618:\
              \x20\x01\x0d\xb8\0\0\0\0\0\0\0\0\0\0\0\x01\0\x507:dropped6:\x01\x01\x01\x01\0\x01e",
        )
        .unwrap();
        assert_eq!(
            pex,
            [
                "1.2.3.4:5".parse().unwrap(),
                "10.0.0.1:6881".parse().unwrap(),
                "[2001:db8::1]:80".parse().unwrap()
            ]
        );

        assert!(parse_pex(b"de").unwrap().is_empty());
        assert_eq!(
            parse_pex(b"d5:added6:\x01\x02\x03\x04\x00\x056:added65:\0\0\0\0\0e").unwrap(),
            ["1.2.3.4:5".parse().unwrap()]
        );
        assert!(parse_pex(b"garbage").is_err());
    }
}
