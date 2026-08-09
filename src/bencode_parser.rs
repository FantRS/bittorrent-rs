/// СТРОКА БАЙТ: *`<len>:<value>`* (*`4:spam`* - spam, *`3:gay`* - gay)
///
/// ЧИСЛО: *`i<value>e`* (*`i42e`* - 42, *`i0e`* - 0, *`i-21e`* - -21)
///
/// СПИСОК: *`l<value>e`* (*`l4:spami42ee`* - spam42)
///
/// СЛОВНИК: *`d<value>e`* (*`d3:bar4:spam3:fooi42ee`* - { bar:spam, foo:42 })
use std::collections::BTreeMap;

pub enum BencodeValue {
    Int(i32),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

pub fn parse(bytes: Vec<u8>) {
    for byte in bytes {}
}
