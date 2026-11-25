use anyhow::{Result, bail};
use bytes::Bytes;
use std::net::SocketAddr;

use crate::bencode::{BencodeParser, BencodeValue, Dict, DictExt};
use crate::peer::compact::parse_compact_peers;

pub const HANDSHAKE: u8 = 0;
pub const UT_PEX: u8 = 1;
pub const UT_METADATA: u8 = 2;
pub const METADATA_PIECE_LEN: usize = 1 << 14;
pub const REQQ: u8 = 250;

const METADATA_REQUEST: u8 = 0;
const METADATA_DATA: u8 = 1;
const METADATA_REJECT: u8 = 2;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RemoteExtensions {
    pub ut_metadata: Option<u8>,
    pub metadata_size: Option<usize>,
    pub reqq: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataMessage {
    Request(u32),
    Data { piece: u32, block: Bytes },
    Reject(u32),
}

pub fn handshake() -> Bytes {
    let extensions = Dict::from([
        (b"ut_pex".to_vec(), BencodeValue::Int(UT_PEX.into())),
        (
            b"ut_metadata".to_vec(),
            BencodeValue::Int(UT_METADATA.into()),
        ),
    ]);
    let handshake = Dict::from([
        (b"m".to_vec(), BencodeValue::Dict(extensions)),
        (b"reqq".to_vec(), BencodeValue::Int(REQQ.into())),
    ]);

    BencodeValue::Dict(handshake).encode().into()
}

pub fn parse_handshake(payload: &[u8]) -> Result<RemoteExtensions> {
    let mut dict = parse_dict(payload)?;
    let mut extensions: Dict = dict.take_opt("m").unwrap_or_default();

    Ok(RemoteExtensions {
        ut_metadata: extensions
            .take_opt::<u64>("ut_metadata")
            .and_then(|id| u8::try_from(id).ok())
            .filter(|&id| id != 0),
        metadata_size: dict
            .take_opt::<u64>("metadata_size")
            .and_then(|size| usize::try_from(size).ok()),
        reqq: dict
            .take_opt::<u64>("reqq")
            .and_then(|reqq| usize::try_from(reqq).ok()),
    })
}

pub fn parse_pex(payload: &[u8]) -> Result<Vec<SocketAddr>> {
    let mut dict = parse_dict(payload)?;
    let added = dict
        .take_opt::<Vec<u8>>("added")
        .and_then(|bytes| parse_compact_peers::<[u8; 4]>(&bytes).ok());
    let added6 = dict
        .take_opt::<Vec<u8>>("added6")
        .and_then(|bytes| parse_compact_peers::<[u8; 16]>(&bytes).ok());

    Ok(added.into_iter().chain(added6).flatten().collect())
}

impl MetadataMessage {
    pub fn encode(&self) -> Bytes {
        let (kind, piece) = match self {
            Self::Request(piece) => (METADATA_REQUEST, piece),
            Self::Data { piece, .. } => (METADATA_DATA, piece),
            Self::Reject(piece) => (METADATA_REJECT, piece),
        };
        let header = BencodeValue::Dict(Dict::from([
            (b"msg_type".to_vec(), BencodeValue::Int(kind.into())),
            (b"piece".to_vec(), BencodeValue::Int((*piece).into())),
        ]));

        let mut payload = header.encode();
        if let Self::Data { block, .. } = self {
            payload.extend_from_slice(block);
        }

        payload.into()
    }

    pub fn decode(payload: &Bytes) -> Result<Self> {
        let mut parser = BencodeParser::new(payload.to_vec());
        let mut dict = Dict::try_from(parser.parse()?)?;
        let piece = u32::try_from(dict.take::<u64>("piece")?)?;

        Ok(match u8::try_from(dict.take::<u64>("msg_type")?)? {
            METADATA_REQUEST => Self::Request(piece),
            METADATA_DATA => Self::Data {
                piece,
                block: payload.slice(parser.consumed()..),
            },
            METADATA_REJECT => Self::Reject(piece),
            kind => bail!("Unknown metadata message type: {kind}"),
        })
    }
}

fn parse_dict(payload: &[u8]) -> Result<Dict> {
    Dict::try_from(BencodeParser::new(payload.to_vec()).parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_cases() {
        assert_eq!(
            &handshake()[..],
            b"d1:md11:ut_metadatai2e6:ut_pexi1ee4:reqqi250ee"
        );
        assert_eq!(
            parse_handshake(
                b"d1:md11:ut_metadatai3e6:ut_pexi1ee13:metadata_sizei31235e4:reqqi500ee"
            )
            .unwrap(),
            RemoteExtensions {
                ut_metadata: Some(3),
                metadata_size: Some(31235),
                reqq: Some(500),
            }
        );
        assert_eq!(
            parse_handshake(b"d1:md11:ut_metadatai0eee").unwrap(),
            RemoteExtensions::default()
        );
        assert_eq!(parse_handshake(b"de").unwrap(), RemoteExtensions::default());
        assert!(parse_handshake(b"le").is_err());
    }

    #[test]
    fn metadata_cases() {
        let messages = [
            MetadataMessage::Request(0),
            MetadataMessage::Data {
                piece: 1,
                block: Bytes::from_static(b"d4:infoe"),
            },
            MetadataMessage::Reject(7),
        ];
        for message in messages {
            assert_eq!(MetadataMessage::decode(&message.encode()).unwrap(), message);
        }

        assert_eq!(
            &MetadataMessage::Request(2).encode()[..],
            b"d8:msg_typei0e5:piecei2ee"
        );
        assert!(
            MetadataMessage::decode(&Bytes::from_static(b"d8:msg_typei5e5:piecei0ee")).is_err()
        );
        assert!(MetadataMessage::decode(&Bytes::from_static(b"d8:msg_typei0ee")).is_err());
    }

    #[test]
    fn pex_cases() {
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
