use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use std::fs::File;
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::task::spawn_blocking;

use crate::resume::FileStamp;
use crate::torrent::TorrentInfo;

#[derive(Debug)]
pub struct Storage {
    files: Arc<Files>,
    piece_length: u64,
    existing: bool,
}

#[derive(Debug)]
struct Files(Vec<StoredFile>);

#[derive(Debug)]
struct StoredFile {
    file: Option<File>,
    offset: u64,
    length: u64,
}

impl Storage {
    pub async fn create(dir: impl AsRef<Path>, info: &TorrentInfo) -> Result<Self> {
        let mut files = Vec::with_capacity(info.files.len());
        let mut offset = 0;
        let mut existing = false;

        for entry in &info.files {
            if entry.padding {
                files.push(StoredFile {
                    file: None,
                    offset,
                    length: entry.length,
                });
                offset += entry.length;
                continue;
            }

            let path = dir.as_ref().join(&entry.path);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .await
                .with_context(|| format!("Failed to open {}", path.display()))?;
            let current = file.metadata().await?.len();
            existing |= current > 0;
            if current != entry.length {
                file.set_len(entry.length).await?;
            }

            files.push(StoredFile {
                file: Some(file.into_std().await),
                offset,
                length: entry.length,
            });
            offset += entry.length;
        }

        Ok(Self {
            files: Arc::new(Files(files)),
            piece_length: info.piece_length,
            existing,
        })
    }

    #[must_use]
    pub fn has_existing_data(&self) -> bool {
        self.existing
    }

    pub async fn stamps(&self) -> Result<Vec<FileStamp>> {
        let files = Arc::clone(&self.files);

        spawn_blocking(move || files.stamps()).await?
    }

    pub async fn sync(&self) -> Result<()> {
        let files = Arc::clone(&self.files);

        spawn_blocking(move || files.sync()).await?
    }

    pub async fn write_piece(&self, index: u32, data: Bytes) -> Result<()> {
        let offset = self.offset(index, 0)?;
        let files = Arc::clone(&self.files);

        spawn_blocking(move || files.write_at(offset, &data)).await?
    }

    pub async fn read_block(&self, index: u32, begin: u32, length: u32) -> Result<Bytes> {
        let offset = self.offset(index, begin)?;
        let files = Arc::clone(&self.files);

        let buf = spawn_blocking(move || files.read_at(offset, length)).await??;

        Ok(Bytes::from(buf))
    }

    pub fn read_piece_blocking(&self, index: u32, length: u32) -> Result<Vec<u8>> {
        self.files.read_at(self.offset(index, 0)?, length)
    }

    fn offset(&self, index: u32, begin: u32) -> Result<u64> {
        u64::from(index)
            .checked_mul(self.piece_length)
            .and_then(|start| start.checked_add(u64::from(begin)))
            .context("Block offset overflow")
    }
}

impl Files {
    fn opened(&self) -> impl Iterator<Item = &File> {
        self.0.iter().filter_map(|stored| stored.file.as_ref())
    }

    fn stamps(&self) -> Result<Vec<FileStamp>> {
        self.opened()
            .map(|file| {
                let metadata = file.metadata()?;
                Ok(FileStamp {
                    length: metadata.len(),
                    modified: metadata.modified()?,
                })
            })
            .collect()
    }

    fn sync(&self) -> Result<()> {
        self.opened().try_for_each(File::sync_all)?;

        Ok(())
    }

    fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        for (file, file_offset, range) in self.spans(offset, data.len())? {
            let chunk = data.get(range).context("Span out of buffer bounds")?;
            if let Some(file) = file {
                file.write_all_at(chunk, file_offset)?;
            }
        }

        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>> {
        let mut buf = vec![0; usize::try_from(length)?];

        for (file, file_offset, range) in self.spans(offset, buf.len())? {
            let chunk = buf.get_mut(range).context("Span out of buffer bounds")?;
            if let Some(file) = file {
                file.read_exact_at(chunk, file_offset)?;
            }
        }

        Ok(buf)
    }

    fn spans(
        &self,
        offset: u64,
        len: usize,
    ) -> Result<impl Iterator<Item = (Option<&File>, u64, Range<usize>)>> {
        let end = offset
            .checked_add(u64::try_from(len)?)
            .context("Range overflow")?;
        let total = self.0.last().map_or(0, |file| file.offset + file.length);
        ensure!(end <= total, "Range is out of storage bounds");

        let first = self
            .0
            .partition_point(|file| file.offset + file.length <= offset);
        let overlapping = self.0.get(first..).unwrap_or_default();

        Ok(overlapping
            .iter()
            .take_while(move |file| file.offset < end)
            .filter_map(move |file| {
                let start = offset.max(file.offset);
                let stop = end.min(file.offset + file.length);
                let range =
                    usize::try_from(start - offset).ok()?..usize::try_from(stop - offset).ok()?;

                (start < stop).then_some((file.file.as_ref(), start - file.offset, range))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::torrent::FileEntry;

    #[tokio::test]
    async fn single_file_cases() {
        let dir = tempfile::tempdir().unwrap();
        let info = TorrentInfo::from_data("file", 4, b"abcdef");

        let storage = Storage::create(dir.path(), &info).await.unwrap();
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

        let reopened = Storage::create(dir.path(), &info).await.unwrap();
        assert!(reopened.has_existing_data());
        assert_eq!(reopened.read_piece_blocking(1, 2).unwrap(), b"ef");

        let nested = dir.path().join("a/b");
        Storage::create(&nested, &info).await.unwrap();
        assert!(nested.join("file").exists());
    }

    #[tokio::test]
    async fn multi_file_cases() {
        let dir = tempfile::tempdir().unwrap();
        let file = |path: &str, length| FileEntry {
            path: path.into(),
            length,
            padding: false,
        };
        let info = TorrentInfo {
            files: vec![
                file("dir/a", 2),
                file("dir/sub/b", 3),
                file("dir/empty", 0),
                FileEntry {
                    padding: true,
                    ..file("dir/.pad/2", 2)
                },
                file("dir/c", 1),
            ],
            ..TorrentInfo::from_data("dir", 4, b"abcde\0\0f")
        };

        let storage = Storage::create(dir.path(), &info).await.unwrap();
        storage
            .write_piece(0, Bytes::from_static(b"abcd"))
            .await
            .unwrap();
        storage
            .write_piece(1, Bytes::from_static(b"e\0\0f"))
            .await
            .unwrap();
        storage.sync().await.unwrap();
        assert_eq!(storage.stamps().await.unwrap().len(), 4);
        assert!(!dir.path().join("dir/.pad").exists());

        let read = |path: &str| std::fs::read(dir.path().join(path)).unwrap();
        assert_eq!(read("dir/a"), b"ab");
        assert_eq!(read("dir/sub/b"), b"cde");
        assert_eq!(read("dir/empty"), b"");
        assert_eq!(read("dir/c"), b"f");

        assert_eq!(&storage.read_block(0, 1, 3).await.unwrap()[..], b"bcd");
        assert_eq!(&storage.read_block(1, 0, 4).await.unwrap()[..], b"e\0\0f");
        assert_eq!(&storage.read_block(1, 2, 2).await.unwrap()[..], b"\0f");
        assert!(storage.read_block(1, 3, 2).await.is_err());
    }
}
