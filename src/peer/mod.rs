pub(crate) mod compact;
pub mod connection;
pub(crate) mod extension;
pub mod handshake;
pub mod message;
#[cfg(test)]
pub(crate) mod testing;

pub use connection::PeerConnection;
pub use extension::METADATA_PIECE_LEN;
pub use handshake::Handshake;
pub use message::{Message, MessageCodec, Piece, Request};
