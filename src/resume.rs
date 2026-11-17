use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeData {
    pub info_hash: [u8; 20],
    pub have: Vec<u8>,
    pub files: Vec<FileStamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub length: u64,
    pub modified: SystemTime,
}

impl ResumeData {
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
    }

    pub async fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let temp = path.with_added_extension("tmp");
        let mut file = File::create(&temp).await?;
        file.write_all(&serde_json::to_vec(self)?).await?;
        file.sync_all().await?;
        tokio::fs::rename(temp, path).await?;

        Ok(())
    }
}
