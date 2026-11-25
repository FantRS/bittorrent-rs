use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;

use crate::peer::{Handshake, Message, MessageCodec, Piece};

pub async fn spawn_seeder(
    reply: Option<Handshake>,
    bitfield: &'static [u8],
    data: Bytes,
    piece_length: usize,
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let Some(reply) = reply else { return };
        if Handshake::read_from(&mut socket).await.is_err() {
            return;
        }
        reply.write_to(&mut socket).await.unwrap();

        let mut frames = Framed::new(socket, MessageCodec);
        frames
            .send(Message::Bitfield(Bytes::from_static(bitfield)))
            .await
            .unwrap();
        frames.send(Message::Unchoke).await.unwrap();

        let mut choked_once = false;
        while let Some(Ok(message)) = frames.next().await {
            let Message::Request(request) = message else {
                continue;
            };

            if !choked_once {
                choked_once = true;
                frames.send(Message::Choke).await.unwrap();
                frames.send(Message::Unchoke).await.unwrap();
                continue;
            }

            let index = request.index.as_usize();
            let start = index * piece_length + usize::try_from(request.begin).unwrap();
            let end = start + usize::try_from(request.length).unwrap();
            frames
                .send(Message::Piece(Piece {
                    index: request.index,
                    begin: request.begin,
                    block: data.slice(start..end),
                }))
                .await
                .unwrap();
        }
    });

    addr
}
