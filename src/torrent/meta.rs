use anyhow::{Result, ensure};
use sha1::{Digest, Sha1};
use std::path::{Component, Path, PathBuf};

use crate::bencode::{BencodeValue, Dict, DictExt};
use crate::torrent::PieceIndex;

#[derive(Debug)]
pub struct TorrentMeta {
    pub trackers: Vec<String>,
    pub info: TorrentInfo,
}

#[derive(Clone)]
pub struct TorrentInfo {
    pub name: String,
    pub piece_length: u64,
    pub pieces: Vec<[u8; 20]>,
    pub length: u64,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub length: u64,
    pub padding: bool,
}

impl TryFrom<BencodeValue> for TorrentMeta {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let mut dict = Dict::try_from(value)?;

        let mut trackers: Vec<String> = dict
            .take_opt::<Vec<BencodeValue>>("announce-list")
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tier| Vec::<BencodeValue>::try_from(tier).ok())
            .flatten()
            .filter_map(|tracker| String::try_from(tracker).ok())
            .collect();
        if trackers.is_empty() {
            trackers.extend(dict.take_opt::<String>("announce"));
        }

        Ok(Self {
            trackers,
            info: dict.take("info")?,
        })
    }
}

impl TryFrom<BencodeValue> for TorrentInfo {
    type Error = anyhow::Error;

    fn try_from(value: BencodeValue) -> Result<Self> {
        let mut dict = Dict::try_from(value)?;
        let name: String = dict.take("name")?;
        let piece_length: u64 = dict.take("piece length")?;
        ensure!(piece_length > 0, "'piece length' must be positive");

        let pieces: Vec<u8> = dict.take("pieces")?;
        let (hashes, rest) = pieces.as_chunks::<20>();
        ensure!(rest.is_empty(), "'pieces' length is not a multiple of 20");
        ensure!(
            u32::try_from(hashes.len()).is_ok(),
            "Torrent has too many pieces"
        );

        let root = PathBuf::from(safe_component(&name)?);
        let files = match dict.take_opt::<Vec<BencodeValue>>("files") {
            Some(entries) => {
                ensure!(!entries.is_empty(), "'files' list is empty");
                entries
                    .into_iter()
                    .map(|entry| parse_file(entry, &root))
                    .collect::<Result<Vec<_>>>()?
            }
            None => vec![FileEntry {
                path: root,
                length: dict.take("length")?,
                padding: false,
            }],
        };

        let length: u64 = files.iter().map(|file| file.length).sum();
        ensure!(
            usize::try_from(length.div_ceil(piece_length)).is_ok_and(|count| count == hashes.len()),
            "Piece count does not match total length"
        );

        Ok(Self {
            name,
            piece_length,
            pieces: hashes.to_vec(),
            length,
            files,
        })
    }
}

impl TorrentInfo {
    #[must_use]
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    #[must_use]
    pub fn piece_len(&self, index: PieceIndex) -> Option<u32> {
        let start = u64::from(index.get()).checked_mul(self.piece_length)?;
        let len = self
            .length
            .checked_sub(start)
            .filter(|&rest| rest > 0)?
            .min(self.piece_length);

        u32::try_from(len).ok()
    }

    #[must_use]
    pub fn verify(&self, index: PieceIndex, data: &[u8]) -> bool {
        self.pieces
            .get(index.as_usize())
            .is_some_and(|hash| Sha1::digest(data).as_slice() == hash)
    }
}

impl std::fmt::Debug for TorrentInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorrentInfo")
            .field("name", &self.name)
            .field("piece_length", &self.piece_length)
            .field("length", &self.length)
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

fn parse_file(value: BencodeValue, root: &Path) -> Result<FileEntry> {
    let mut dict = Dict::try_from(value)?;
    let parts: Vec<BencodeValue> = dict.take("path")?;
    ensure!(!parts.is_empty(), "File path is empty");

    let mut path = root.to_path_buf();
    for part in parts {
        path.push(safe_component(&String::try_from(part)?)?);
    }

    Ok(FileEntry {
        path,
        length: dict.take("length")?,
        padding: dict
            .take_opt::<Vec<u8>>("attr")
            .is_some_and(|attr| attr.contains(&b'p')),
    })
}

fn safe_component(component: &str) -> Result<&str> {
    let mut parts = Path::new(component).components();
    ensure!(
        matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(part)), None) if part == component
        ) && !component.contains('\0'),
        "Unsafe path component: {component:?}"
    );

    Ok(component)
}

#[cfg(test)]
impl TorrentInfo {
    pub(crate) fn from_data(name: &str, piece_length: u64, data: &[u8]) -> Self {
        let length = u64::try_from(data.len()).unwrap();
        let chunk = usize::try_from(piece_length).unwrap();

        Self {
            name: name.into(),
            piece_length,
            pieces: data
                .chunks(chunk)
                .map(|piece| Sha1::digest(piece).into())
                .collect(),
            length,
            files: vec![FileEntry {
                path: name.into(),
                length,
                padding: false,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bencode::BencodeParser;

    fn info(bencode: &str) -> Result<TorrentInfo> {
        BencodeParser::new(bencode.as_bytes().to_vec())
            .parse()?
            .try_into()
    }

    #[test]
    fn file_cases() {
        let pieces = "6:pieces20:aaaaaaaaaaaaaaaaaaaa";
        let single = info(&format!("d6:lengthi3e4:name1:f12:piece lengthi4e{pieces}e")).unwrap();
        assert_eq!(single.length, 3);
        assert_eq!(
            single.files,
            [FileEntry {
                path: "f".into(),
                length: 3,
                padding: false
            }]
        );

        let multi = info(&format!(
            "d5:filesld6:lengthi2e4:pathl1:aeed4:attr1:p6:lengthi1e4:pathl4:.pad1:1eed6:lengthi1e4:pathl3:sub6:b: c.dee\
             e4:name3:dir12:piece lengthi4e{pieces}e"
        ))
        .unwrap();
        assert_eq!(multi.length, 4);
        assert_eq!(
            multi.files,
            [
                FileEntry {
                    path: "dir/a".into(),
                    length: 2,
                    padding: false
                },
                FileEntry {
                    path: "dir/.pad/1".into(),
                    length: 1,
                    padding: true
                },
                FileEntry {
                    path: "dir/sub/b: c.d".into(),
                    length: 1,
                    padding: false
                },
            ]
        );

        assert!(
            info(&format!(
                "d6:lengthi3e4:name2:..12:piece lengthi4e{pieces}e"
            ))
            .is_err()
        );
        assert!(
            info(&format!(
                "d5:filesld6:lengthi1e4:pathl2:..1:xeee4:name3:dir12:piece lengthi4e{pieces}e"
            ))
            .is_err()
        );
        for name in ["1:/", "3:a/b", "2:..", "1:.", "0:", "2:a\0"] {
            let torrent = format!("d6:lengthi3e4:name{name}12:piece lengthi4e{pieces}e");
            assert!(info(&torrent).is_err(), "{name}");
        }
        assert!(info(&format!("d5:filesle4:name3:dir12:piece lengthi4e{pieces}e")).is_err());
        assert!(info(&format!("d6:lengthi9e4:name1:f12:piece lengthi4e{pieces}e")).is_err());
        assert!(info(&format!("d6:lengthi3e4:name1:f12:piece lengthi0e{pieces}e")).is_err());
    }

    fn trackers(bencode: &str) -> Result<Vec<String>> {
        let info = "d6:lengthi1e4:name1:f12:piece lengthi1e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
        let value = BencodeParser::new(format!("d{bencode}4:info{info}e").into_bytes()).parse()?;
        Ok(TorrentMeta::try_from(value)?.trackers)
    }

    #[test]
    fn tracker_cases() {
        assert_eq!(trackers("8:announce5:http1").unwrap(), ["http1"]);
        assert_eq!(
            trackers("8:announce5:http113:announce-listll5:http1el4:udp14:udp2ee").unwrap(),
            ["http1", "udp1", "udp2"]
        );
        assert_eq!(trackers("13:announce-listll4:udp1ee").unwrap(), ["udp1"]);
        assert_eq!(
            trackers("8:announce5:http113:announce-listle").unwrap(),
            ["http1"]
        );
        assert!(trackers("").unwrap().is_empty());
    }

    #[test]
    fn piece_cases() {
        let info = TorrentInfo::from_data("file", 4, b"abcdef");

        assert_eq!(info.piece_count(), 2);
        assert_eq!(info.piece_len(0.into()), Some(4));
        assert_eq!(info.piece_len(1.into()), Some(2));
        assert_eq!(info.piece_len(2.into()), None);
        assert_eq!(info.piece_len(u32::MAX.into()), None);

        assert!(info.verify(0.into(), b"abcd"));
        assert!(info.verify(1.into(), b"ef"));
        assert!(!info.verify(1.into(), b"eg"));
        assert!(!info.verify(2.into(), b""));
    }
}
