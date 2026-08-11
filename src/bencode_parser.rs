use std::collections::BTreeMap;

use anyhow::{Result, anyhow};

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

    /// ЧИСЛО: *`i<value>e`* (*`i42e`* - 42, *`i0e`* - 0, *`i-21e`* - -21)
    pub fn parse_int(&mut self) -> Result<BencodeValue> {
        let mut value: Vec<u8> = Vec::new();
        let mut is_end = false;

        while let Some(byte) = self.bytes.get(self.cursor) {
            self.cursor += 1;

            match byte {
                b'i' => continue,
                b'e' => {
                    is_end = true;
                    break;
                },
                _ => value.push(*byte)
            }
        }

        if !is_end {
            return Err(anyhow!("Integer not terminated with 'e'"));
        }

        let res = String::from_utf8(value)?.parse::<i64>()?;
        Ok(BencodeValue::Int(res))

    }

    /// СТРОКА БАЙТ: *`<len>:<value>`* (*`4:spam`* - spam, *`3:gay`* - gay)
    pub fn parse_bytes(&mut self) -> Result<BencodeValue> {
        let mut len: Vec<u8> = Vec::new();
        let mut is_terminated = false;

        while let Some(byte) = self.bytes.get(self.cursor) {
            self.cursor += 1;

            match byte {
                b'0'..=b'9' => len.push(*byte),
                b':' => {
                    is_terminated = true;
                    break;
                },
                _ => return Err(anyhow!("Invalid byte string length"))
            }
        }

        if !is_terminated {
            return Err(anyhow!("Byte string length not terminated with ':'"));
        }

        let len = String::from_utf8(len)?.parse::<usize>()?;
        let value = self.bytes.get(self.cursor..self.cursor + len).ok_or_else(|| anyhow!("Invalid byte string"))?.to_vec();
        self.cursor += len;

        Ok(BencodeValue::Bytes(value))
    }

    /// СПИСОК: *`l<value>e`* (*`l4:spami42ee`* - spam42)
    fn _parse_list() -> Result<BencodeValue> {
        Ok(BencodeValue::List(Vec::new()))
    }

    /// СЛОВНИК: *`d<value>e`* (*`d3:bar4:spam3:fooi42ee`* - { bar:spam, foo:42 })
    fn _parse_dict() -> Result<BencodeValue> {
        Ok(BencodeValue::Dict(BTreeMap::new()))
    }

    fn _define_type(&mut self, byte: u8) {
        _ = match byte {
            b'i' => Self::parse_int(self),
            b'l' => Self::_parse_list(),
            b'd' => Self::_parse_dict(),
            b'0'..=b'9' => Self::parse_bytes(self),
            _ => Err(anyhow!("Unknown type"))
        };
    }

    fn _is_start_type(byte: u8) -> bool {
        matches!(byte, b'i' | b'l' | b'd' | b'0'..=b'9')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_cases() {
        // число 42
        let mut p = BencodeParser::new(b"i42e".to_vec());
        assert_eq!(p.parse_int().unwrap(), BencodeValue::Int(42));
        assert_eq!(p.cursor, 4);

        // число 0
        let mut p = BencodeParser::new(b"i0e".to_vec());
        assert_eq!(p.parse_int().unwrap(), BencodeValue::Int(0));

        // число -21
        let mut p = BencodeParser::new(b"i-21e".to_vec());
        assert_eq!(p.parse_int().unwrap(), BencodeValue::Int(-21));

        // помилка: немає закриваючого 'e'
        let mut p = BencodeParser::new(b"i42".to_vec());
        assert!(p.parse_int().is_err());
    }

    #[test]
    fn parse_bytes_cases() {
        // звичайний рядок
        let mut p = BencodeParser::new(b"4:spam".to_vec());
        assert_eq!(p.parse_bytes().unwrap(), BencodeValue::Bytes(b"spam".to_vec()));
        assert_eq!(p.cursor, 6);

        // порожній рядок
        let mut p = BencodeParser::new(b"0:".to_vec());
        assert_eq!(p.parse_bytes().unwrap(), BencodeValue::Bytes(Vec::new()));
        assert_eq!(p.cursor, 2);

        // довжина з кількох цифр
        let mut p = BencodeParser::new(b"10:0123456789".to_vec());
        assert_eq!(p.parse_bytes().unwrap(), BencodeValue::Bytes(b"0123456789".to_vec()));

        // бінарні (не-UTF8) байти
        let mut p = BencodeParser::new(vec![b'4', b':', 0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(p.parse_bytes().unwrap(), BencodeValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));

        // помилка: немає роздільника ':'
        let mut p = BencodeParser::new(b"4spam".to_vec());
        assert!(p.parse_bytes().is_err());

        // помилка: заявлена довжина більша, ніж лишилось байтів
        let mut p = BencodeParser::new(b"10:spam".to_vec());
        assert!(p.parse_bytes().is_err());

        // помилка: довжина не число
        let mut p = BencodeParser::new(b"a:spam".to_vec());
        assert!(p.parse_bytes().is_err());
    }
}