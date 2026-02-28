use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::RwLock;

/// Default maximum number of indexed projects
pub const DEFAULT_MAX_PROJECTS: usize = 10;

/// Snapshot of indexed files with their hashes
/// Supports multiple projects (roots), each with its own collection
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// Map from project root path to its root info (collection + files)
    pub roots: HashMap<PathBuf, RootInfo>,
}

/// Information about a single project root
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootInfo {
    /// Collection name for this project in Milvus
    pub collection_name: String,
    /// Files indexed in this project
    pub files: HashMap<PathBuf, FileEntry>,
    /// Last index timestamp
    pub indexed_at: u64,
    /// Last access timestamp (for LRU eviction)
    pub last_accessed_at: u64,
}

impl RootInfo {
    pub fn new(collection_name: String) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self {
            collection_name,
            files: HashMap::new(),
            indexed_at: now,
            last_accessed_at: now,
        }
    }

    /// Update last accessed timestamp
    pub fn touch(&mut self) {
        self.last_accessed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub hash: String,
    pub chunk_count: usize,
    pub indexed_at: u64,
}

/// Manages snapshots for incremental indexing with multi-project support
pub struct SnapshotManager {
    snapshot_path: PathBuf,
    snapshot: RwLock<Snapshot>,
    max_projects: usize,
}

impl SnapshotManager {
    #[allow(dead_code)] // Used when max_projects is not configured
    pub fn new(snapshot_path: PathBuf) -> Result<Self> {
        Self::new_with_max_projects(snapshot_path, DEFAULT_MAX_PROJECTS)
    }

    pub fn new_with_max_projects(snapshot_path: PathBuf, max_projects: usize) -> Result<Self> {
        Ok(Self {
            snapshot_path,
            snapshot: RwLock::new(Snapshot::default()),
            max_projects,
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

    /// Get file hash for a specific project
    pub async fn get_file_hash(&self, project_root: &Path, file_path: &Path) -> Option<String> {
        let snapshot = self.snapshot.read().await;
        snapshot
            .roots
            .get(project_root)
            .and_then(|root| root.files.get(file_path).map(|e| e.hash.clone()))
    }

    /// Update file info for a specific project
    pub async fn update_file(&self, project_root: &Path, file_path: PathBuf, hash: String, chunk_count: usize) {
        let mut snapshot = self.snapshot.write().await;
        if let Some(root) = snapshot.roots.get_mut(project_root) {
            root.files.insert(file_path, FileEntry {
                hash,
                chunk_count,
                indexed_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            });
        }
    }

    /// Create or get root info for a project
    /// If max_projects is exceeded, returns the oldest project to evict
    pub async fn get_or_create_root(&self, project_root: &Path, collection_name: &str) -> (RootInfo, Option<PathBuf>) {
        let mut snapshot = self.snapshot.write().await;
        
        // Check if project already exists
        if let Some(root) = snapshot.roots.get_mut(project_root) {
            root.touch();
            return (root.clone(), None);
        }
        
        // Check if we need to evict oldest project
        let mut to_evict = None;
        if snapshot.roots.len() >= self.max_projects {
            // Find the oldest project (by last_accessed_at)
            to_evict = snapshot
                .roots
                .iter()
                .min_by_key(|(_, root)| root.last_accessed_at)
                .map(|(path, _)| path.clone());
        }
        
        let new_root = RootInfo::new(collection_name.to_string());
        snapshot.roots.insert(project_root.to_path_buf(), new_root.clone());
        
        (new_root, to_evict)
    }

    /// Update last accessed time for a project (called on search)
    #[allow(dead_code)] // Reserved for future use - could be called on search
    pub async fn touch_project(&self, project_root: &Path) {
        let mut snapshot = self.snapshot.write().await;
        if let Some(root) = snapshot.roots.get_mut(project_root) {
            root.touch();
        }
    }

    /// Get collection name for a project
    pub async fn get_collection_name(&self, project_root: &Path) -> Option<String> {
        let mut snapshot = self.snapshot.write().await;
        if let Some(root) = snapshot.roots.get_mut(project_root) {
            root.touch(); // Update access time
            Some(root.collection_name.clone())
        } else {
            None
        }
    }

    /// Get all collection names (for cross-project search)
    pub async fn get_all_collection_names(&self) -> Vec<(PathBuf, String)> {
        let snapshot = self.snapshot.read().await;
        snapshot
            .roots
            .iter()
            .map(|(path, root)| (path.clone(), root.collection_name.clone()))
            .collect()
    }

    /// Check if a path is within any indexed project
    pub async fn find_project_root(&self, path: &Path) -> Option<PathBuf> {
        let snapshot = self.snapshot.read().await;
        for root_path in snapshot.roots.keys() {
            if path.starts_with(root_path) {
                return Some(root_path.clone());
            }
        }
        None
    }

    /// Remove a project root and return its collection name
    pub async fn remove_root(&self, project_root: &Path) -> Option<String> {
        let mut snapshot = self.snapshot.write().await;
        snapshot.roots.remove(project_root).map(|r| r.collection_name)
    }

    /// Get all project roots
    pub async fn get_all_roots(&self) -> Vec<PathBuf> {
        let snapshot = self.snapshot.read().await;
        snapshot.roots.keys().cloned().collect()
    }

    /// Get project count
    pub async fn get_project_count(&self) -> usize {
        let snapshot = self.snapshot.read().await;
        snapshot.roots.len()
    }

    /// Get max projects limit
    #[allow(dead_code)] // Reserved for future use - could be exposed via status tool
    pub fn max_projects(&self) -> usize {
        self.max_projects
    }

    /// Clear all data
    pub async fn clear(&self) {
        let mut snapshot = self.snapshot.write().await;
        snapshot.roots.clear();
    }

    /// Clear a specific project
    pub async fn clear_project(&self, project_root: &Path) -> Option<String> {
        let mut snapshot = self.snapshot.write().await;
        snapshot.roots.remove(project_root).map(|r| r.collection_name)
    }

    /// Get all projects sorted by last accessed time (oldest first)
    #[allow(dead_code)] // Reserved for future use - could be used for eviction reporting
    pub async fn get_projects_by_age(&self) -> Vec<(PathBuf, u64)> {
        let snapshot = self.snapshot.read().await;
        let mut projects: Vec<_> = snapshot
            .roots
            .iter()
            .map(|(path, root)| (path.clone(), root.last_accessed_at))
            .collect();
        projects.sort_by_key(|(_, timestamp)| *timestamp);
        projects
    }
}
