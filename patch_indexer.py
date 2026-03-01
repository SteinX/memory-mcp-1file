import re

with open("src/codebase/indexer.rs", "r") as f:
    content = f.read()

# Зменшуємо batch size
content = content.replace(
    "let batch_size = 20;",
    "let batch_size = 12;"
)

# Додаємо Throttle після збереження чанків (блок усередині файлу)
chunk_store = """if let Err(e) = db.store_chunks(batch).await {
                                tracing::error!("Failed to store {} chunks: {}", batch_len, e);
                            }
                            chunk_buffer.clear();"""
chunk_store_new = """if let Err(e) = db.store_chunks(batch).await {
                                tracing::error!("Failed to store {} chunks: {}", batch_len, e);
                            }
                            // Throttle: Даємо час SurrealDB на HNSW index compaction (OCC retry mitigation)
                            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                            chunk_buffer.clear();"""
content = content.replace(chunk_store, chunk_store_new)

# Додаємо Throttle після збереження символів
symbol_store = """if let Err(e) = db.store_symbols(batch).await {
                                tracing::error!("Failed to store {} symbols: {}", batch_len, e);
                            }
                            symbol_buffer.clear();"""
symbol_store_new = """if let Err(e) = db.store_symbols(batch).await {
                                tracing::error!("Failed to store {} symbols: {}", batch_len, e);
                            }
                            // Throttle: Зменшуємо навантаження на write-locks
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            symbol_buffer.clear();"""
content = content.replace(symbol_store, symbol_store_new)

# Додаємо Throttle після фінальних залишків чанків (flushing)
final_chunk_store = """if let Err(e) = db.store_chunks(batch).await {
                    tracing::error!("Failed to store final {} chunks: {}", batch_len, e);
                }"""
final_chunk_store_new = """if let Err(e) = db.store_chunks(batch).await {
                    tracing::error!("Failed to store final {} chunks: {}", batch_len, e);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;"""
content = content.replace(final_chunk_store, final_chunk_store_new)

# Додаємо Throttle після фінальних залишків символів
final_symbol_store = """if let Err(e) = db.store_symbols(batch).await {
                    tracing::error!("Failed to store final {} symbols: {}", batch_len, e);
                }"""
final_symbol_store_new = """if let Err(e) = db.store_symbols(batch).await {
                    tracing::error!("Failed to store final {} symbols: {}", batch_len, e);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;"""
content = content.replace(final_symbol_store, final_symbol_store_new)

with open("src/codebase/indexer.rs", "w") as f:
    f.write(content)
