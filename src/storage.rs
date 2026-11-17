use anyhow::{Context, Result};
use bytes::Bytes;
use positioned_io::{RandomAccessFile, ReadAt, WriteAt};
use std::path::Path;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::task::spawn_blocking;

use crate::torrent::TorrentInfo;

#[derive(Debug)]
pub struct Storage {
    file: Arc<RandomAccessFile>,
    piece_length: u64,
    existing: bool,
}

impl Storage {
    pub async fn create(dir: impl AsRef<Path>, info: &TorrentInfo) -> Result<Self> {
        let name = Path::new(&info.name)
            .file_name()
            .with_context(|| format!("Invalid file name: {:?}", info.name))?;

        tokio::fs::create_dir_all(&dir).await?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.as_ref().join(name))
            .await?;
        let existing = file.metadata().await?.len() > 0;
        file.set_len(info.length).await?;

        Ok(Self {
            file: Arc::new(RandomAccessFile::try_new(file.into_std().await)?),
            piece_length: info.piece_length,
            existing,
        })
    }

    #[must_use]
    pub fn has_existing_data(&self) -> bool {
        self.existing
    }

    pub async fn write_piece(&self, index: u32, data: Bytes) -> Result<()> {
        let offset = self.offset(index, 0)?;
        let file = Arc::clone(&self.file);

        spawn_blocking(move || (&*file).write_all_at(offset, &data)).await??;

        Ok(())
    }

    pub async fn read_block(&self, index: u32, begin: u32, length: u32) -> Result<Bytes> {
        let offset = self.offset(index, begin)?;
        let file = Arc::clone(&self.file);

        let buf = spawn_blocking(move || read_at(&file, offset, length)).await??;

        Ok(Bytes::from(buf))
    }

    pub fn read_piece_blocking(&self, index: u32, length: u32) -> Result<Vec<u8>> {
        read_at(&self.file, self.offset(index, 0)?, length)
    }

    fn offset(&self, index: u32, begin: u32) -> Result<u64> {
        u64::from(index)
            .checked_mul(self.piece_length)
            .and_then(|start| start.checked_add(u64::from(begin)))
            .context("Block offset overflow")
    }
}

fn read_at(file: &RandomAccessFile, offset: u64, length: u32) -> Result<Vec<u8>> {
    let mut buf = vec![0; usize::try_from(length)?];
    file.read_exact_at(offset, &mut buf)?;

    Ok(buf)
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

        let storage = Storage::create(dir.path(), &info("file")).await.unwrap();
        assert!(!storage.has_existing_data());
        storage
            .write_piece(1, Bytes::from_static(b"ef"))
            .await
            .unwrap();
        storage
            .write_piece(0, Bytes::from_static(b"abcd"))
            .await
            .unwrap();
        assert_eq!(std::fs::read(dir.path().join("file")).unwrap(), b"abcdef");
        assert_eq!(&storage.read_block(1, 1, 1).await.unwrap()[..], b"f");
        assert_eq!(&storage.read_block(0, 1, 4).await.unwrap()[..], b"bcde");
        assert!(storage.read_block(1, 0, 4).await.is_err());

        let reopened = Storage::create(dir.path(), &info("file")).await.unwrap();
        assert!(reopened.has_existing_data());
        assert_eq!(reopened.read_piece_blocking(1, 2).unwrap(), b"ef");
        assert_eq!(&reopened.read_block(0, 0, 4).await.unwrap()[..], b"abcd");

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
