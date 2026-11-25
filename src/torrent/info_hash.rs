use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    FromBytes,
    IntoBytes,
    KnownLayout,
    Immutable,
)]
#[repr(transparent)]
pub struct InfoHash([u8; 20]);

impl InfoHash {
    #[must_use]
    pub fn of(data: &[u8]) -> Self {
        Self(Sha1::digest(data).into())
    }
}

impl From<[u8; 20]> for InfoHash {
    fn from(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }
}

impl From<InfoHash> for [u8; 20] {
    fn from(hash: InfoHash) -> Self {
        hash.0
    }
}

impl AsRef<[u8]> for InfoHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
