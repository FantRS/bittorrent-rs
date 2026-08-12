use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub enum BencodeValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

impl BencodeValue {
    pub fn get<K>(&self, _key: K) -> String
    where
        K: ToString,
    {
        "".into()
    }
}