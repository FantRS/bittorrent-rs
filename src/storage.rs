use anyhow::{Context, Result};
use std::io::SeekFrom;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::torrent::TorrentInfo;

#[derive(Debug)]
pub struct Storage {
    file: File,
    piece_length: u64,
}

impl Storage {
    pub async fn create(dir: impl AsRef<Path>, info: &TorrentInfo) -> Result<Self> {
        let name = Path::new(&info.name)
            .file_name()
            .with_context(|| format!("Invalid file name: {:?}", info.name))?;

        tokio::fs::create_dir_all(&dir).await?;
        let file = File::create(dir.as_ref().join(name)).await?;
        file.set_len(info.length).await?;

        Ok(Self {
            file,
            piece_length: info.piece_length,
        })
    }

    pub async fn write_piece(&mut self, index: u32, data: &[u8]) -> Result<()> {
        let offset = u64::from(index)
            .checked_mul(self.piece_length)
            .context("Piece offset overflow")?;

        self.file.seek(SeekFrom::Start(offset)).await?;
        self.file.write_all(data).await?;
        self.file.flush().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> TorrentInfo {
        TorrentInfo {
            name: name.into(),
            piece_length: 4,
            pieces: vec![[0; 20]; 2],
            length: 6,
        }
    }

    #[tokio::test]
    async fn storage_cases() {
        let dir = tempfile::tempdir().unwrap();

        let mut storage = Storage::create(dir.path(), &info("file")).await.unwrap();
        storage.write_piece(1, b"ef").await.unwrap();
        storage.write_piece(0, b"abcd").await.unwrap();
        assert_eq!(std::fs::read(dir.path().join("file")).unwrap(), b"abcdef");

        Storage::create(dir.path(), &info("../escape"))
            .await
            .unwrap();
        assert!(dir.path().join("escape").exists());

        assert!(Storage::create(dir.path(), &info("..")).await.is_err());

        let nested = dir.path().join("a/b");
        Storage::create(&nested, &info("file")).await.unwrap();
        assert!(nested.join("file").exists());
    }
}
