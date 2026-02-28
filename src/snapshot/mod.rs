use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::RwLock;

/// Snapshot of indexed files with their hashes
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub files: HashMap<PathBuf, FileEntry>,
    pub collection_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub hash: String,
    pub chunk_count: usize,
    pub indexed_at: u64,
}

/// Manages snapshots for incremental indexing
pub struct SnapshotManager {
    snapshot_path: PathBuf,
    snapshot: RwLock<Snapshot>,
}

impl SnapshotManager {
    pub fn new(snapshot_path: PathBuf) -> Result<Self> {
        Ok(Self {
            snapshot_path,
            snapshot: RwLock::new(Snapshot::default()),
        })
    }

    pub async fn load(&self) -> Result<()> {
        if self.snapshot_path.exists() {
            let data = fs::read_to_string(&self.snapshot_path).await?;
            let snapshot = serde_json::from_str(&data)?;
            *self.snapshot.write().await = snapshot;
        }
        Ok(())
    }

    pub async fn save(&self) -> Result<()> {
        let snapshot = self.snapshot.read().await;
        let data = serde_json::to_string_pretty(&*snapshot)?;
        
        if let Some(parent) = self.snapshot_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        fs::write(&self.snapshot_path, data).await?;
        Ok(())
    }

    pub async fn get_file_hash(&self, path: &Path) -> Option<String> {
        self.snapshot.read().await.files.get(path).map(|e| e.hash.clone())
    }

    pub async fn update_file(&self, path: PathBuf, hash: String, chunk_count: usize) {
        let mut snapshot = self.snapshot.write().await;
        snapshot.files.insert(path, FileEntry {
            hash,
            chunk_count,
            indexed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        });
    }

    pub async fn set_collection_name(&self, name: String) {
        let mut snapshot = self.snapshot.write().await;
        snapshot.collection_name = name;
    }

    pub async fn get_collection_name(&self) -> Option<String> {
        Some(self.snapshot.read().await.collection_name.clone())
    }

    pub async fn clear(&self) {
        let mut snapshot = self.snapshot.write().await;
        snapshot.files.clear();
        snapshot.collection_name = String::new();
    }
}
