use std::collections::BTreeMap;

/// СТРОКА БАЙТ: *`<len>:<value>`* (*`4:spam`* - spam, *`3:gay`* - gay)
///
/// ЧИСЛО: *`i<value>e`* (*`i42e`* - 42, *`i0e`* - 0, *`i-21e`* - -21)
///
/// СПИСОК: *`l<value>e`* (*`l4:spami42ee`* - spam42)
///
/// СЛОВНИК: *`d<value>e`* (*`d3:bar4:spam3:fooi42ee`* - { bar:spam, foo:42 })
pub enum BencodeValue {
    Int(i32),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

// pub struct ParseResult {
//     bencode: BencodeValue,
//     cursor: usize,
// }

pub struct ParseResult;

pub fn parse(bytes: &Vec<u8>, cursor: usize) {
    if let Some(b) = bytes.get(cursor) {
        let _parse_result = start_checker(*b);
    }

    if cursor <= bytes.len() {
        parse(bytes, cursor);
    }
}

fn start_checker(byte: u8) -> ParseResult {
    // match char::from(byte) {
    //     'i' => int_parse(),
    //     'l' => list_parse(),
    //     'd' => dict_parse(),
    //     _ => bytes_parse(),
    // }

    ParseResult
}

pub fn int_parse(bytes: &Vec<u8>, cursor: usize) -> (BencodeValue, usize) {
    
}

fn list_parse() -> ParseResult {
    ParseResult
}

fn dict_parse() -> ParseResult {
    ParseResult
}

fn bytes_parse() -> ParseResult {
    ParseResult
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_parse_test() {}
}
