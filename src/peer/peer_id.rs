use std::fmt;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

const CLIENT_PREFIX: &[u8; 8] = b"-RS0001-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(transparent)]
pub struct PeerId([u8; 20]);

impl PeerId {
    #[must_use]
    pub fn generate() -> Self {
        let mut id: [u8; 20] = rand::random();
        if let Some(prefix) = id.first_chunk_mut() {
            *prefix = *CLIENT_PREFIX;
        }
        Self(id)
    }
}

impl From<[u8; 20]> for PeerId {
    fn from(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for PeerId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.escape_ascii().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_cases() {
        let generated = PeerId::generate();
        assert!(generated.as_ref().starts_with(CLIENT_PREFIX));
        assert_ne!(generated, PeerId::generate());

        let peer = PeerId::from(*b"-qB5210-abc\n\x1b[2Jxyzw");
        assert_eq!(peer.to_string(), "-qB5210-abc\\n\\x1b[2Jxyzw");
    }
}
