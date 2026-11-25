pub(crate) mod compact;
pub(crate) mod extension;

mod connection;
mod handshake;
mod message;
mod peer_id;
mod pipeline;

#[cfg(test)]
pub(crate) mod testing;

pub use connection::PeerConnection;
pub use extension::METADATA_PIECE_LEN;
pub use handshake::Handshake;
pub use message::{Message, MessageCodec, Piece, Request};
pub use peer_id::PeerId;
pub use pipeline::BLOCK_LEN;
