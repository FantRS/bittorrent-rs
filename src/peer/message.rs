use anyhow::{Result, bail, ensure};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::mem;
use tokio_util::codec::{Decoder, Encoder};

use crate::torrent::PieceIndex;

const MAX_FRAME_LEN: usize = 1 << 20;

mod id {
    pub const CHOKE: u8 = 0;
    pub const UNCHOKE: u8 = 1;
    pub const INTERESTED: u8 = 2;
    pub const NOT_INTERESTED: u8 = 3;
    pub const HAVE: u8 = 4;
    pub const BITFIELD: u8 = 5;
    pub const REQUEST: u8 = 6;
    pub const PIECE: u8 = 7;
    pub const CANCEL: u8 = 8;
    pub const PORT: u8 = 9;
    pub const EXTENDED: u8 = 20;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub index: PieceIndex,
    pub begin: u32,
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub index: PieceIndex,
    pub begin: u32,
    pub block: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(PieceIndex),
    Bitfield(Bytes),
    Request(Request),
    Piece(Piece),
    Cancel(Request),
    Port(u16),
    Extended { id: u8, payload: Bytes },
}

#[derive(Debug, Default)]
pub struct MessageCodec;

impl Decoder for MessageCodec {
    type Item = Message;
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Message>> {
        let Some(&prefix) = src.first_chunk::<4>() else {
            return Ok(None);
        };

        let len = usize::try_from(u32::from_be_bytes(prefix))?;
        ensure!(len <= MAX_FRAME_LEN, "Frame too large: {len} bytes");

        if src.len() < 4 + len {
            src.reserve(4 + len - src.len());
            return Ok(None);
        }

        src.advance(4);
        let mut payload = src.split_to(len).freeze();

        if payload.is_empty() {
            return Ok(Some(Message::KeepAlive));
        }

        let kind = payload.try_get_u8()?;
        let message = match kind {
            id::CHOKE => Message::Choke,
            id::UNCHOKE => Message::Unchoke,
            id::INTERESTED => Message::Interested,
            id::NOT_INTERESTED => Message::NotInterested,
            id::HAVE => Message::Have(payload.try_get_u32()?.into()),
            id::BITFIELD => Message::Bitfield(mem::take(&mut payload)),
            id::REQUEST => Message::Request(Request::decode(&mut payload)?),
            id::PIECE => Message::Piece(Piece {
                index: payload.try_get_u32()?.into(),
                begin: payload.try_get_u32()?,
                block: mem::take(&mut payload),
            }),
            id::CANCEL => Message::Cancel(Request::decode(&mut payload)?),
            id::PORT => Message::Port(payload.try_get_u16()?),
            id::EXTENDED => Message::Extended {
                id: payload.try_get_u8()?,
                payload: mem::take(&mut payload),
            },
            _ => bail!("Unknown message id: {kind}"),
        };

        ensure!(payload.is_empty(), "Malformed message with id {kind}");

        Ok(Some(message))
    }
}

impl Encoder<Message> for MessageCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, message: Message, dst: &mut BytesMut) -> Result<()> {
        match message {
            Message::KeepAlive => {
                dst.put_u32(0);
                Ok(())
            }
            Message::Choke => put_frame(dst, id::CHOKE, &[]),
            Message::Unchoke => put_frame(dst, id::UNCHOKE, &[]),
            Message::Interested => put_frame(dst, id::INTERESTED, &[]),
            Message::NotInterested => put_frame(dst, id::NOT_INTERESTED, &[]),
            Message::Have(index) => put_frame(dst, id::HAVE, &[&index.get().to_be_bytes()]),
            Message::Bitfield(bits) => put_frame(dst, id::BITFIELD, &[&bits]),
            Message::Request(request) => {
                put_frame(dst, id::REQUEST, &[request.to_bytes().as_flattened()])
            }
            Message::Piece(piece) => put_frame(
                dst,
                id::PIECE,
                &[
                    &piece.index.get().to_be_bytes(),
                    &piece.begin.to_be_bytes(),
                    &piece.block,
                ],
            ),
            Message::Cancel(request) => {
                put_frame(dst, id::CANCEL, &[request.to_bytes().as_flattened()])
            }
            Message::Port(port) => put_frame(dst, id::PORT, &[&port.to_be_bytes()]),
            Message::Extended {
                id: extension,
                payload,
            } => put_frame(dst, id::EXTENDED, &[&[extension], &payload]),
        }
    }
}

impl Request {
    fn decode(buf: &mut Bytes) -> Result<Self> {
        Ok(Self {
            index: buf.try_get_u32()?.into(),
            begin: buf.try_get_u32()?,
            length: buf.try_get_u32()?,
        })
    }

    fn to_bytes(self) -> [[u8; 4]; 3] {
        [self.index.get(), self.begin, self.length].map(u32::to_be_bytes)
    }
}

fn put_frame(dst: &mut BytesMut, id: u8, parts: &[&[u8]]) -> Result<()> {
    let len = 1 + parts.iter().map(|part| part.len()).sum::<usize>();

    dst.reserve(4 + len);
    dst.put_u32(u32::try_from(len)?);
    dst.put_u8(id);
    for part in parts {
        dst.put_slice(part);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(message: Message) -> BytesMut {
        let mut buf = BytesMut::new();
        MessageCodec.encode(message, &mut buf).unwrap();
        buf
    }

    #[test]
    fn roundtrip() {
        let request = Request {
            index: 1.into(),
            begin: 16384,
            length: 16384,
        };
        let messages = [
            Message::KeepAlive,
            Message::Choke,
            Message::Unchoke,
            Message::Interested,
            Message::NotInterested,
            Message::Have(42.into()),
            Message::Bitfield(Bytes::from_static(&[0b1010_0000, 0xFF])),
            Message::Request(request),
            Message::Piece(Piece {
                index: 3.into(),
                begin: 0,
                block: Bytes::from_static(b"block"),
            }),
            Message::Cancel(request),
            Message::Port(6881),
            Message::Extended {
                id: 1,
                payload: Bytes::from_static(b"d5:addede"),
            },
        ];

        let mut stream = BytesMut::new();
        for message in &messages {
            stream.extend_from_slice(&encode(message.clone()));
        }

        for message in messages {
            assert_eq!(MessageCodec.decode(&mut stream).unwrap(), Some(message));
        }
        assert!(stream.is_empty());
        assert_eq!(MessageCodec.decode(&mut stream).unwrap(), None);
    }

    #[test]
    fn wire_format() {
        assert_eq!(&encode(Message::KeepAlive)[..], &[0, 0, 0, 0]);
        assert_eq!(&encode(Message::Interested)[..], &[0, 0, 0, 1, 2]);
        assert_eq!(
            &encode(Message::Have(258.into()))[..],
            &[0, 0, 0, 5, 4, 0, 0, 1, 2]
        );
        assert_eq!(
            &encode(Message::Request(Request {
                index: 1.into(),
                begin: 2,
                length: 3
            }))[..],
            &[0, 0, 0, 13, 6, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3]
        );
    }

    #[test]
    fn decode_cases() {
        let full = encode(Message::Have(7.into()));
        let mut partial = BytesMut::from(&full[..6]);
        assert_eq!(MessageCodec.decode(&mut partial).unwrap(), None);
        assert_eq!(partial.len(), 6);
        partial.extend_from_slice(&full[6..]);
        assert_eq!(
            MessageCodec.decode(&mut partial).unwrap(),
            Some(Message::Have(7.into()))
        );

        let mut short_prefix = BytesMut::from(&[0, 0][..]);
        assert_eq!(MessageCodec.decode(&mut short_prefix).unwrap(), None);

        let mut too_large = BytesMut::from(&u32::MAX.to_be_bytes()[..]);
        assert!(MessageCodec.decode(&mut too_large).is_err());

        let mut unknown = BytesMut::from(&[0, 0, 0, 1, 42][..]);
        assert!(MessageCodec.decode(&mut unknown).is_err());

        let mut short_have = BytesMut::from(&[0, 0, 0, 3, 4, 0, 0][..]);
        assert!(MessageCodec.decode(&mut short_have).is_err());

        let mut empty_extended = BytesMut::from(&[0, 0, 0, 1, 20][..]);
        assert!(MessageCodec.decode(&mut empty_extended).is_err());

        let mut long_choke = BytesMut::from(&[0, 0, 0, 2, 0, 0][..]);
        assert!(MessageCodec.decode(&mut long_choke).is_err());
    }
}
