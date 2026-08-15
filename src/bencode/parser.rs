use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use sha1::{Digest, Sha1};

use crate::bencode::BencodeValue;

pub struct BencodeParser {
    bytes: Vec<u8>,
    cursor: usize,

    info_span: Option<(usize, usize)>,
}

impl BencodeParser {
    pub fn new(bytes: Vec<u8>) -> Self {
        BencodeParser {
            bytes,
            cursor: 0,
            info_span: None,
        }
    }

    pub fn parse(&mut self) -> Result<BencodeValue> {
        self.parse_dict()
    }

    pub fn info_hash(&self) -> Option<[u8; 20]> {
        let (start, end) = self.info_span?;

        let info_bytes = self.bytes.get(start..end)?;
        let digest = Sha1::digest(info_bytes);

        Some(digest.into())
    }

    /// ЧИСЛО: `i<var>e`
    fn parse_int(&mut self) -> Result<BencodeValue> {
        self.cursor += 1;
        let raw = self.read_until(b'e')?;
        let var: i64 = String::from_utf8(raw)?.parse()?;

        Ok(BencodeValue::Int(var))
    }

    /// СТРОКА БАЙТ: `<len>:<var>`
    fn parse_bytes(&mut self) -> Result<BencodeValue> {
        let raw = self.read_until(b':')?;
        let len: usize = String::from_utf8(raw)?.parse()?;
        let var = self
            .bytes
            .get(self.cursor..self.cursor + len)
            .ok_or_else(|| anyhow!("Invalid byte string"))?
            .to_vec();
        self.cursor += len;

        Ok(BencodeValue::Bytes(var))
    }

    /// СПИСОК: `l<var>e`
    fn parse_list(&mut self) -> Result<BencodeValue> {
        let mut var = Vec::new();

        self.parse_delimited(b'l', |p, byte| {
            var.push(p.parse_by_type(byte)?);

            Ok(())
        })?;

        Ok(BencodeValue::List(var))
    }

    /// СЛОВНИК: `d<var>e`
    fn parse_dict(&mut self) -> Result<BencodeValue> {
        let mut var = BTreeMap::new();

        self.parse_delimited(b'd', |p, _byte| {
            let key = match p.parse_bytes()? {
                BencodeValue::Bytes(k) => k,
                _ => {
                    return Err(anyhow!("Dictionary key must be a byte string"));
                }
            };

            let inner_byte = *p
                .bytes
                .get(p.cursor)
                .ok_or_else(|| anyhow!("Unexpected end of input while reading dictionary value"))?;

            let is_info_key = key == b"info";
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
        mut on_item: impl FnMut(&mut Self, u8) -> Result<()>,
    ) -> Result<()> {
        let mut is_start = true;
        let mut is_end = false;

        while let Some(&byte) = self.bytes.get(self.cursor) {
            match byte {
                b if b == open && is_start => {
                    self.cursor += 1;
                    is_start = false;
                }
                b'e' => {
                    self.cursor += 1;
                    is_end = true;
                    break;
                }
                _ => on_item(self, byte)?,
            }
        }

        if !is_end {
            return Err(anyhow!("Value not terminated with 'e'"));
        }

        Ok(())
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
        assert_eq!(
            p.parse_bytes().unwrap(),
            BencodeValue::Bytes(b"spam".to_vec())
        );
        assert_eq!(p.cursor, 6);

        // порожній рядок
        let mut p = BencodeParser::new(b"0:".to_vec());
        assert_eq!(p.parse_bytes().unwrap(), BencodeValue::Bytes(Vec::new()));
        assert_eq!(p.cursor, 2);

        // довжина з кількох цифр
        let mut p = BencodeParser::new(b"10:0123456789".to_vec());
        assert_eq!(
            p.parse_bytes().unwrap(),
            BencodeValue::Bytes(b"0123456789".to_vec())
        );

        // бінарні (не-UTF8) байти
        let mut p = BencodeParser::new(vec![b'4', b':', 0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            p.parse_bytes().unwrap(),
            BencodeValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );

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

    #[test]
    fn parse_list_cases() {
        // порожній список
        let mut p = BencodeParser::new(b"le".to_vec());
        assert_eq!(p.parse_list().unwrap(), BencodeValue::List(vec![]));

        // список зі змішаними типами
        let mut p = BencodeParser::new(b"l4:spami42ee".to_vec());
        assert_eq!(
            p.parse_list().unwrap(),
            BencodeValue::List(vec![
                BencodeValue::Bytes(b"spam".to_vec()),
                BencodeValue::Int(42)
            ])
        );

        // вкладений список
        let mut p = BencodeParser::new(b"ll4:spamee".to_vec());
        assert_eq!(
            p.parse_list().unwrap(),
            BencodeValue::List(vec![BencodeValue::List(vec![BencodeValue::Bytes(
                b"spam".to_vec()
            )])])
        );

        // список списків з числами
        let mut p = BencodeParser::new(b"lli1ei2eeli3ei4eee".to_vec());
        assert_eq!(
            p.parse_list().unwrap(),
            BencodeValue::List(vec![
                BencodeValue::List(vec![BencodeValue::Int(1), BencodeValue::Int(2)]),
                BencodeValue::List(vec![BencodeValue::Int(3), BencodeValue::Int(4)])
            ])
        );

        // курсор просунувся рівно до кінця вхідних байтів
        let mut p = BencodeParser::new(b"l4:spame".to_vec());
        p.parse_list().unwrap();
        assert_eq!(p.cursor, 8);

        // помилка: список без завершального 'e'
        let mut p = BencodeParser::new(b"l4:spam".to_vec());
        assert!(p.parse_list().is_err());

        // помилка: обірваний елемент всередині списку
        let mut p = BencodeParser::new(b"l4:sp".to_vec());
        assert!(p.parse_list().is_err());

        // помилка: невідомий тип елемента
        let mut p = BencodeParser::new(b"lxe".to_vec());
        assert!(p.parse_list().is_err());
    }

    #[test]
    fn parse_dict_cases() {
        // порожній словник
        let mut p = BencodeParser::new(b"de".to_vec());
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(BTreeMap::new()));

        // простий словник з двома різнотипними значеннями
        let mut p = BencodeParser::new(b"d3:bar4:spam3:fooi42ee".to_vec());
        let mut expected = BTreeMap::new();
        expected.insert(b"bar".to_vec(), BencodeValue::Bytes(b"spam".to_vec()));
        expected.insert(b"foo".to_vec(), BencodeValue::Int(42));
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(expected));

        // вкладений словник як значення
        let mut p = BencodeParser::new(b"d3:food3:bar3:bazee".to_vec());
        let mut inner = BTreeMap::new();
        inner.insert(b"bar".to_vec(), BencodeValue::Bytes(b"baz".to_vec()));
        let mut expected = BTreeMap::new();
        expected.insert(b"foo".to_vec(), BencodeValue::Dict(inner));
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(expected));

        // список як значення (наближено до реального .torrent, поле "files")
        let mut p = BencodeParser::new(b"d5:filesli1ei2eee".to_vec());
        let mut expected = BTreeMap::new();
        expected.insert(
            b"files".to_vec(),
            BencodeValue::List(vec![BencodeValue::Int(1), BencodeValue::Int(2)]),
        );
        assert_eq!(p.parse_dict().unwrap(), BencodeValue::Dict(expected));

        // помилка: ключ не byte string
        let mut p = BencodeParser::new(b"di42e4:spame".to_vec());
        assert!(p.parse_dict().is_err());

        // помилка: значення відсутнє (обірвано одразу після ключа)
        let mut p = BencodeParser::new(b"d3:fooe".to_vec());
        assert!(p.parse_dict().is_err());

        // помилка: словник без завершального 'e'
        let mut p = BencodeParser::new(b"d3:foo3:bar".to_vec());
        assert!(p.parse_dict().is_err());
    }
}
