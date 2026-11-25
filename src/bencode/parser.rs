use anyhow::{Result, anyhow, ensure};
use std::collections::BTreeMap;

use crate::bencode::BencodeValue;

const MAX_DEPTH: usize = 64;

pub struct BencodeParser {
    bytes: Vec<u8>,
    cursor: usize,
    depth: usize,

    info_span: Option<(usize, usize)>,
}

impl BencodeParser {
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        BencodeParser {
            bytes,
            cursor: 0,
            depth: 0,
            info_span: None,
        }
    }

    pub fn parse(&mut self) -> Result<BencodeValue> {
        self.parse_dict()
    }

    #[must_use]
    pub fn consumed(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn info_bytes(&self) -> Option<&[u8]> {
        let (start, end) = self.info_span?;
        self.bytes.get(start..end)
    }

    fn parse_int(&mut self) -> Result<BencodeValue> {
        self.cursor += 1;
        let raw = self.read_until(b'e')?;
        let var: i64 = String::from_utf8(raw)?.parse()?;

        Ok(BencodeValue::Int(var))
    }

    fn parse_bytes(&mut self) -> Result<BencodeValue> {
        let raw = self.read_until(b':')?;
        let len: usize = String::from_utf8(raw)?.parse()?;
        let var = self
            .cursor
            .checked_add(len)
            .and_then(|end| self.bytes.get(self.cursor..end))
            .ok_or_else(|| anyhow!("Invalid byte string"))?
            .to_vec();
        self.cursor += len;

        Ok(BencodeValue::Bytes(var))
    }

    fn parse_list(&mut self) -> Result<BencodeValue> {
        let mut var = Vec::new();

        self.parse_delimited(b'l', |p, byte| {
            var.push(p.parse_by_type(byte)?);

            Ok(())
        })?;

        Ok(BencodeValue::List(var))
    }

    fn parse_dict(&mut self) -> Result<BencodeValue> {
        let mut var = BTreeMap::new();

        self.parse_delimited(b'd', |p, _byte| {
            let BencodeValue::Bytes(key) = p.parse_bytes()? else {
                return Err(anyhow!("Dictionary key must be a byte string"));
            };

            let inner_byte = *p
                .bytes
                .get(p.cursor)
                .ok_or_else(|| anyhow!("Unexpected end of input while reading dictionary value"))?;

            let is_info_key = p.depth == 1 && key == b"info";
            let value_start = p.cursor;

            let value = p.parse_by_type(inner_byte)?;

            if is_info_key {
                p.info_span = Some((value_start, p.cursor));
            }

            var.insert(key, value);

            Ok(())
        })?;

        Ok(BencodeValue::Dict(var))
    }

    fn parse_by_type(&mut self, byte: u8) -> Result<BencodeValue> {
        match byte {
            b'0'..=b'9' => self.parse_bytes(),
            b'i' => self.parse_int(),
            b'l' => self.parse_list(),
            b'd' => self.parse_dict(),
            _ => Err(anyhow!("Unknown type")),
        }
    }

    fn read_until(&mut self, terminator: u8) -> Result<Vec<u8>> {
        let mut buf = Vec::new();

        while let Some(&byte) = self.bytes.get(self.cursor) {
            self.cursor += 1;
            if byte == terminator {
                return Ok(buf);
            }
            buf.push(byte);
        }

        Err(anyhow!(
            "Unexpected end of input, expected '{}'",
            terminator as char
        ))
    }

    fn parse_delimited(
        &mut self,
        open: u8,
        on_item: impl FnMut(&mut Self, u8) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.bytes.get(self.cursor) == Some(&open),
            "Expected '{}'",
            open as char
        );
        ensure!(
            self.depth < MAX_DEPTH,
            "Nesting deeper than {MAX_DEPTH} levels"
        );

        self.cursor += 1;
        self.depth += 1;
        let items = self.parse_items(on_item);
        self.depth -= 1;

        items
    }

    fn parse_items(&mut self, mut on_item: impl FnMut(&mut Self, u8) -> Result<()>) -> Result<()> {
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if byte == b'e' {
                self.cursor += 1;
                return Ok(());
            }
            on_item(self, byte)?;
        }

        Err(anyhow!("Value not terminated with 'e'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_cases() {
        let mut p = BencodeParser::new(b"i42e".to_vec());
        assert_eq!(p.parse_int().unwrap(), BencodeValue::Int(42));
        assert_eq!(p.cursor, 4);

        let mut p = BencodeParser::new(b"i0e".to_vec());
        assert_eq!(p.parse_int().unwrap(), BencodeValue::Int(0));

        let mut p = BencodeParser::new(b"i-21e".to_vec());
        assert_eq!(p.parse_int().unwrap(), BencodeValue::Int(-21));

        let mut p = BencodeParser::new(b"i42".to_vec());
        assert!(p.parse_int().is_err());
    }

    #[test]
    fn parse_bytes_cases() {
        let mut p = BencodeParser::new(b"4:spam".to_vec());
        assert_eq!(
            p.parse_bytes().unwrap(),
            BencodeValue::Bytes(b"spam".to_vec())
        );
        assert_eq!(p.cursor, 6);

        let mut p = BencodeParser::new(b"0:".to_vec());
        assert_eq!(p.parse_bytes().unwrap(), BencodeValue::Bytes(Vec::new()));
        assert_eq!(p.cursor, 2);

        let mut p = BencodeParser::new(b"10:0123456789".to_vec());
        assert_eq!(
            p.parse_bytes().unwrap(),
            BencodeValue::Bytes(b"0123456789".to_vec())
        );

        let mut p = BencodeParser::new(vec![b'4', b':', 0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            p.parse_bytes().unwrap(),
            BencodeValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );

        let mut p = BencodeParser::new(b"4spam".to_vec());
        assert!(p.parse_bytes().is_err());

        let mut p = BencodeParser::new(b"10:spam".to_vec());
        assert!(p.parse_bytes().is_err());

        let mut p = BencodeParser::new(b"a:spam".to_vec());
        assert!(p.parse_bytes().is_err());
    }

    #[test]
    fn parse_list_cases() {
        let mut p = BencodeParser::new(b"le".to_vec());
        assert_eq!(p.parse_list().unwrap(), BencodeValue::List(vec![]));

        let nested = |depth| [b"l".repeat(depth), b"e".repeat(depth)].concat();
        assert!(BencodeParser::new(nested(MAX_DEPTH)).parse_list().is_ok());
        assert!(
            BencodeParser::new(nested(MAX_DEPTH + 1))
                .parse_list()
                .is_err()
        );
        assert!(BencodeParser::new(nested(1 << 20)).parse_list().is_err());

        let mut p = BencodeParser::new(b"ll1:ae".to_vec());
        assert!(p.parse_list().is_err());
        assert_eq!(p.depth, 0);

        let mut p = BencodeParser::new(b"l4:spami42ee".to_vec());
        assert_eq!(
            p.parse_list().unwrap(),
            BencodeValue::List(vec![
                BencodeValue::Bytes(b"spam".to_vec()),
                BencodeValue::Int(42)
            ])
        );

        let mut p = BencodeParser::new(b"ll4:spamee".to_vec());
        assert_eq!(
            p.parse_list().unwrap(),
            BencodeValue::List(vec![BencodeValue::List(vec![BencodeValue::Bytes(
                b"spam".to_vec()
            )])])
        );

        let mut p = BencodeParser::new(b"lli1ei2eeli3ei4eee".to_vec());
        assert_eq!(
            p.parse_list().unwrap(),
            BencodeValue::List(vec![
                BencodeValue::List(vec![BencodeValue::Int(1), BencodeValue::Int(2)]),
                BencodeValue::List(vec![BencodeValue::Int(3), BencodeValue::Int(4)])
            ])
        );

        let mut p = BencodeParser::new(b"l4:spame".to_vec());
        p.parse_list().unwrap();
        assert_eq!(p.cursor, 8);

        let mut p = BencodeParser::new(b"l4:spam".to_vec());
        assert!(p.parse_list().is_err());

        let mut p = BencodeParser::new(b"l4:sp".to_vec());
        assert!(p.parse_list().is_err());

        let mut p = BencodeParser::new(b"lxe".to_vec());
        assert!(p.parse_list().is_err());
    }

    #[test]
    fn parse_dict_cases() {
        let mut p = BencodeParser::new(b"de".to_vec());
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(BTreeMap::new()));

        let mut p = BencodeParser::new(b"d3:bar4:spam3:fooi42ee".to_vec());
        let mut expected = BTreeMap::new();
        expected.insert(b"bar".to_vec(), BencodeValue::Bytes(b"spam".to_vec()));
        expected.insert(b"foo".to_vec(), BencodeValue::Int(42));
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(expected));

        let mut p = BencodeParser::new(b"d3:food3:bar3:bazee".to_vec());
        let mut inner = BTreeMap::new();
        inner.insert(b"bar".to_vec(), BencodeValue::Bytes(b"baz".to_vec()));
        let mut expected = BTreeMap::new();
        expected.insert(b"foo".to_vec(), BencodeValue::Dict(inner));
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(expected));

        let mut p = BencodeParser::new(b"d5:filesli1ei2eee".to_vec());
        let mut expected = BTreeMap::new();
        expected.insert(
            b"files".to_vec(),
            BencodeValue::List(vec![BencodeValue::Int(1), BencodeValue::Int(2)]),
        );
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(expected));

        let mut p = BencodeParser::new(b"di42e4:spame".to_vec());
        assert!(p.parse_dict().is_err());

        let mut p = BencodeParser::new(b"d3:fooe".to_vec());
        assert!(p.parse_dict().is_err());

        let mut p = BencodeParser::new(b"d3:foo3:bar".to_vec());
        assert!(p.parse_dict().is_err());

        let mut p = BencodeParser::new(b"d18446744073709551615:xe".to_vec());
        assert!(p.parse_dict().is_err());
    }

    #[test]
    fn info_bytes_cases() {
        let mut p = BencodeParser::new(b"d4:infod4:infoi1e1:ai2ee7:x-extrad4:infoi3eee".to_vec());
        p.parse().unwrap();
        assert_eq!(p.info_bytes(), Some(&b"d4:infoi1e1:ai2ee"[..]));

        let mut p = BencodeParser::new(b"d3:fooi1ee".to_vec());
        p.parse().unwrap();
        assert_eq!(p.info_bytes(), None);
    }
}
