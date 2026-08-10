use std::collections::BTreeMap;

use anyhow::{Result, anyhow};

/// СТРОКА БАЙТ: *`<len>:<value>`* (*`4:spam`* - spam, *`3:gay`* - gay)
///
/// ЧИСЛО: *`i<value>e`* (*`i42e`* - 42, *`i0e`* - 0, *`i-21e`* - -21)
///
/// СПИСОК: *`l<value>e`* (*`l4:spami42ee`* - spam42)
///
/// СЛОВНИК: *`d<value>e`* (*`d3:bar4:spam3:fooi42ee`* - { bar:spam, foo:42 })
#[derive(Debug, PartialEq, Eq)]
pub enum BencodeValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

pub struct BencodeParser {
    bytes: Vec<u8>,
    cursor: usize,
}

impl BencodeParser {
    pub fn new(bytes: Vec<u8>) -> Self {
        BencodeParser {
            bytes,
            cursor: 0,
        }
    }

    pub fn parse(&mut self) -> Result<BencodeValue> {
        // Self::parse_dict();

        Err(anyhow!("Parse error!"))
    }

    pub fn parse_int(&mut self) -> Result<BencodeValue> {
        let mut value: Vec<u8> = Vec::new();

        while let Some(byte) = self.bytes.get(self.cursor) {
            self.cursor += 1;

            match byte {
                b'i' => continue,
                b'e' => break,
                _ => value.push(*byte),
            }
        } 

        let res = String::from_utf8(value)?.parse::<i64>()?;

        Ok(BencodeValue::Int(res))
    }

    fn parse_bytes() -> Result<BencodeValue> {
        Ok(BencodeValue::Bytes(Vec::new()))
    }

    fn parse_list() -> Result<BencodeValue> {
        Ok(BencodeValue::List(Vec::new()))
    }

    fn parse_dict() -> Result<BencodeValue> {
        Ok(BencodeValue::Dict(BTreeMap::new()))
    }

    fn define_type(&mut self, byte: u8) {
        _ = match byte {
            b'i' => Self::parse_int(self),
            b'l' => Self::parse_list(),
            b'd' => Self::parse_dict(),
            _ => Self::parse_bytes(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: Vec<u8>) -> Result<BencodeValue> {
        let mut parser = BencodeParser::new(bytes.to_vec());
        parser.parse_int()
    }

    #[test]
    fn int_parse_test() -> std::result::Result<(), anyhow::Error> {
        let bytes1 = b"i42e";
        let bytes2 = b"i-420e";
        let bytes3 = b"i0e";

        let res1 = parse(bytes1.to_vec())?;
        let res2 = parse(bytes2.to_vec())?;
        let res3 = parse(bytes3.to_vec())?;

        assert_eq!(res1, BencodeValue::Int(42));
        assert_eq!(res2, BencodeValue::Int(-420));
        assert_eq!(res3, BencodeValue::Int(0));

        Ok(())
    }
}
