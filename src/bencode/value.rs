use anyhow::{Result, anyhow};
use std::collections::BTreeMap;

pub type Dict = BTreeMap<Vec<u8>, BencodeValue>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BencodeValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(Dict),
}

pub trait DictExt {
    fn take<T>(&mut self, key: &str) -> Result<T>
    where
        T: TryFrom<BencodeValue, Error = anyhow::Error>;

    fn take_opt<T>(&mut self, key: &str) -> Option<T>
    where
        T: TryFrom<BencodeValue, Error = anyhow::Error>;
}

impl DictExt for Dict {
    fn take<T>(&mut self, key: &str) -> Result<T>
    where
        T: TryFrom<BencodeValue, Error = anyhow::Error>,
    {
        self.remove(key.as_bytes())
            .ok_or_else(|| anyhow!("Missing '{key}' key"))?
            .try_into()
    }

    fn take_opt<T>(&mut self, key: &str) -> Option<T>
    where
        T: TryFrom<BencodeValue, Error = anyhow::Error>,
    {
        self.remove(key.as_bytes())?.try_into().ok()
    }
}

impl TryFrom<BencodeValue> for Dict {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Dict(dict) = value else {
            return Err(anyhow!("Expected a dictionary, got {value:?}"));
        };

        Ok(dict)
    }
}

impl TryFrom<BencodeValue> for u64 {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Int(i) = value else {
            return Err(anyhow!("Expected an integer, got {value:?}"));
        };

        Ok(i.try_into()?)
    }
}

impl TryFrom<BencodeValue> for Vec<u8> {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::Bytes(bytes) = value else {
            return Err(anyhow!("Expected a byte string, got {value:?}"));
        };

        Ok(bytes)
    }
}

impl TryFrom<BencodeValue> for Vec<BencodeValue> {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let BencodeValue::List(list) = value else {
            return Err(anyhow!("Expected a list, got {value:?}"));
        };

        Ok(list)
    }
}

impl TryFrom<BencodeValue> for String {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        match value {
            BencodeValue::Bytes(bytes) => {
                String::from_utf8(bytes).map_err(|e| anyhow!("Invalid UTF-8 string: {e}"))
            }
            _ => Err(anyhow!("Expected a byte string, got {value:?}")),
        }
    }
}
