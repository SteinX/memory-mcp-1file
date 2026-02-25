use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use moka::future::Cache;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const CACHE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("embeddings");
const META_TABLE: TableDefinition<&str, &str> = TableDefinition::new("meta");
const META_KEY_MODEL: &str = "model_name";

#[derive(Clone)]
pub struct EmbeddingStore {
    // L1 Cache: RAM (Fastest)
    ram_cache: Cache<String, Vec<f32>>,
    // L2 Cache: Disk (Persistent)
    disk_cache: Arc<Database>,
    model_name: String,
}

impl EmbeddingStore {
    pub fn new(data_dir: &Path, model_name: &str) -> Result<Self> {
        let db_path = data_dir.join("cache.redb");
        let disk_cache = Database::create(db_path)?;

        let write_txn = disk_cache.begin_write()?;
        {
            let mut meta = write_txn.open_table(META_TABLE)?;

            let stored_model = meta.get(META_KEY_MODEL)?.map(|v| v.value().to_string());
            if stored_model.as_deref() != Some(model_name) {
                if stored_model.is_some() {
                    tracing::warn!(
                        "Embedding model changed from {:?} to {}, purging stale cache",
                        stored_model,
                        model_name
                    );
                    // Drop and recreate cache table to purge stale entries
                    drop(meta);
                    write_txn.delete_table(CACHE_TABLE)?;
                    let _table = write_txn.open_table(CACHE_TABLE)?;
                    let mut meta = write_txn.open_table(META_TABLE)?;
                    meta.insert(META_KEY_MODEL, model_name)?;
                } else {
                    let _table = write_txn.open_table(CACHE_TABLE)?;
                    meta.insert(META_KEY_MODEL, model_name)?;
                }
            } else {
                let _table = write_txn.open_table(CACHE_TABLE)?;
            }
        }
        write_txn.commit()?;

        Ok(Self {
            ram_cache: Cache::builder().max_capacity(2_000).build(),
            disk_cache: Arc::new(disk_cache),
            model_name: model_name.to_string(),
        })
    }

    fn cache_key(&self, hash: &str) -> String {
        format!("{}:{}", self.model_name, hash)
    }

    pub async fn get(&self, hash: &str) -> Option<Vec<f32>> {
        let key = self.cache_key(hash);

        if let Some(vec) = self.ram_cache.get(&key).await {
            return Some(vec);
        }

        let db = self.disk_cache.clone();
        let key_owned = key.clone();

        let vec_opt = tokio::task::spawn_blocking(move || -> Result<Option<Vec<f32>>> {
            let read_txn = db.begin_read()?;
            let table = read_txn.open_table(CACHE_TABLE)?;

            if let Some(value) = table.get(key_owned.as_str())? {
                let vec: Vec<f32> =
                    bincode::serde::decode_from_slice(value.value(), bincode::config::standard())?
                        .0;
                Ok(Some(vec))
            } else {
                Ok(None)
            }
        })
        .await
        .ok()?
        .ok()?;

        if let Some(ref vec) = vec_opt {
            self.ram_cache.insert(key, vec.clone()).await;
        }

        vec_opt
    }

    pub async fn put(&self, hash: String, embedding: Vec<f32>) -> Result<()> {
        let key = self.cache_key(&hash);

        self.ram_cache.insert(key.clone(), embedding.clone()).await;

        let db = self.disk_cache.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(CACHE_TABLE)?;
                let bytes = bincode::serde::encode_to_vec(&embedding, bincode::config::standard())?;
                table.insert(key.as_str(), bytes.as_slice())?;
            }
            write_txn.commit()?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Batch insert embeddings into both RAM and disk cache with a single
    /// redb write transaction (one fsync instead of N).
    pub async fn put_batch(&self, items: Vec<(String, Vec<f32>)>) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }

        // L1: update RAM cache for all items
        for (hash, embedding) in &items {
            let key = self.cache_key(hash);
            self.ram_cache.insert(key, embedding.clone()).await;
        }

        // L2: single disk transaction for all items
        let db = self.disk_cache.clone();
        let cache_keys: Vec<(String, Vec<f32>)> = items
            .into_iter()
            .map(|(hash, emb)| (self.cache_key(&hash), emb))
            .collect();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(CACHE_TABLE)?;
                for (key, embedding) in &cache_keys {
                    let bytes =
                        bincode::serde::encode_to_vec(embedding, bincode::config::standard())?;
                    table.insert(key.as_str(), bytes.as_slice())?;
                }
            }
            write_txn.commit()?;
            Ok(())
        })
        .await??;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_embedding_store_persistence() {
        let dir = tempdir().unwrap();
        let store = EmbeddingStore::new(dir.path(), "test-model").unwrap();

        let hash = "abc123hash".to_string();
        let embedding = vec![0.1, 0.2, 0.3];

        store.put(hash.clone(), embedding.clone()).await.unwrap();

        let retrieved = store.get(&hash).await.unwrap();
        assert_eq!(retrieved, embedding);

        drop(store);
        let store2 = EmbeddingStore::new(dir.path(), "test-model").unwrap();

        let retrieved2 = store2.get(&hash).await.unwrap();
        assert_eq!(retrieved2, embedding);
    }

    #[tokio::test]
    async fn test_model_change_warns() {
        let dir = tempdir().unwrap();
        let store = EmbeddingStore::new(dir.path(), "model-v1").unwrap();

        let hash = "test-hash".to_string();
        let embedding = vec![1.0, 2.0, 3.0];
        store.put(hash.clone(), embedding.clone()).await.unwrap();

        drop(store);

        let store2 = EmbeddingStore::new(dir.path(), "model-v2").unwrap();

        let result = store2.get(&hash).await;
        assert!(result.is_none());
    }
}
