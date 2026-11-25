use std::fmt;
use std::num::TryFromIntError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PieceIndex(u32);

impl PieceIndex {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for PieceIndex {
    fn from(index: u32) -> Self {
        Self(index)
    }
}

impl TryFrom<usize> for PieceIndex {
    type Error = TryFromIntError;

    fn try_from(index: usize) -> Result<Self, Self::Error> {
        u32::try_from(index).map(Self)
    }
}

impl fmt::Display for PieceIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
