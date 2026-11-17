use anyhow::{Context, Result, ensure};
use bitvec::vec::BitVec;
use std::time::Duration;
use tokio::time::{Instant, timeout_at};

use crate::peer::PeerConnection;
use crate::storage::Storage;
use crate::torrent::TorrentInfo;

const IDLE_TIMEOUT: Duration = Duration::from_mins(2);

pub async fn download(
    conn: &mut PeerConnection,
    info: &TorrentInfo,
    storage: &mut Storage,
    mut on_piece: impl FnMut(u64),
) -> Result<()> {
    let mut missing: BitVec = BitVec::repeat(true, info.piece_count());
    let mut deadline = Instant::now() + IDLE_TIMEOUT;
    conn.interested().await?;

    while missing.any() {
        let next = missing.iter_ones().find(|&index| conn.has_piece(index));
        let Some(slot) = next.filter(|_| !conn.is_choked()) else {
            timeout_at(deadline, conn.recv())
                .await
                .with_context(|| format!("{}: no pieces to download", conn.addr))??;
            continue;
        };
        let index = u32::try_from(slot)?;

        let length = info
            .piece_len(index)
            .with_context(|| format!("Invalid piece index: {index}"))?;
        let piece = conn.download_piece(index, length).await?;
        ensure!(
            info.verify(index, &piece),
            "Piece {index} failed hash check"
        );

        storage.write_piece(index, &piece).await?;
        missing.set(slot, false);
        deadline = Instant::now() + IDLE_TIMEOUT;
        on_piece(u64::from(length));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::Handshake;
    use crate::peer::testing::spawn_seeder;
    use bytes::Bytes;
    use sha1::{Digest, Sha1};

    #[tokio::test]
    async fn downloads_whole_file() {
        let info_hash = [7; 20];
        let data: Bytes = (0..=u8::MAX).cycle().take(100_000).collect();
        let info = TorrentInfo {
            name: "file".into(),
            piece_length: 40_000,
            pieces: data
                .chunks(40_000)
                .map(|chunk| Sha1::digest(chunk).into())
                .collect(),
            length: 100_000,
        };

        let reply = Handshake::new(info_hash, [1; 20]);
        let addr = spawn_seeder(Some(reply), &[0b1110_0000], data.clone(), 40_000).await;
        let mut conn = PeerConnection::connect(addr, info_hash, [2; 20], 3)
            .await
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::create(dir.path(), &info).await.unwrap();
        let mut received = 0;
        download(&mut conn, &info, &mut storage, |len| received += len)
            .await
            .unwrap();

        assert_eq!(received, 100_000);
        assert_eq!(std::fs::read(dir.path().join("file")).unwrap(), data);
    }
}
