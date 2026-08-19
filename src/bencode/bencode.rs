use anyhow::{Result, anyhow};
use std::collections::BTreeMap;
use std::net::SocketAddrV4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BencodeValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

impl TryFrom<BencodeValue> for u64 {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Int(i) = value else {
            return Err(anyhow!("Expected an integer, got {:?}", value));
        };

        Ok(i.try_into()?)
    }
}

impl TryFrom<BencodeValue> for Vec<u8> {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Bytes(bytes) = value else {
            return Err(anyhow!("Expected a byte string, got {:?}", value));
        };

        Ok(bytes)
    }
}

impl TryFrom<BencodeValue> for String {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        match value {
            BencodeValue::Bytes(bytes) => {
                String::from_utf8(bytes).map_err(|e| anyhow!("Invalid UTF-8 string: {}", e))
            }
            _ => Err(anyhow!("Expected a byte string, got {:?}", value)),
        }
    }
}

impl TryFrom<BencodeValue> for Vec<SocketAddrV4> {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::List(list) = value else {
            return Err(anyhow!("Expected a list of SocketAddrV4, got {:?}", value));
        };

        list.into_iter()
            .map(|item| {
                let addr_str: String = item.try_into()?;
                let socket_addr: SocketAddrV4 = addr_str.parse()?;
                Ok(socket_addr)
            })
            .collect()
    }
}
