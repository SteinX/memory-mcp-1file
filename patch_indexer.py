import re

with open("src/codebase/indexer.rs", "r") as f:
    content = f.read()

# 1. Видаляємо sleep після db.store_chunks
chunk_store_old = """if let Err(e) = db.store_chunks(batch).await {
                                tracing::error!("Failed to store {} chunks: {}", batch_len, e);
                            }
                            // Throttle: Даємо час SurrealDB на HNSW index compaction (OCC retry mitigation)
                            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                            chunk_buffer.clear();"""
chunk_store_new = """if let Err(e) = db.store_chunks(batch).await {
                                tracing::error!("Failed to store {} chunks: {}", batch_len, e);
                            }
                            chunk_buffer.clear();"""
content = content.replace(chunk_store_old, chunk_store_new)

# 2. Видаляємо sleep після db.store_symbols
symbol_store_old = """if let Err(e) = db.store_symbols(batch).await {
                                tracing::error!("Failed to store {} symbols: {}", batch_len, e);
                            }
                            // Throttle: Зменшуємо навантаження на write-locks
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            symbol_buffer.clear();"""
symbol_store_new = """if let Err(e) = db.store_symbols(batch).await {
                                tracing::error!("Failed to store {} symbols: {}", batch_len, e);
                            }
                            symbol_buffer.clear();"""
content = content.replace(symbol_store_old, symbol_store_new)

# 3. Видаляємо sleep після фінальних залишків (flushing)
final_chunk_old = """if let Err(e) = db.store_chunks(batch).await {
                    tracing::error!("Failed to store final {} chunks: {}", batch_len, e);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;"""
final_chunk_new = """if let Err(e) = db.store_chunks(batch).await {
                    tracing::error!("Failed to store final {} chunks: {}", batch_len, e);
                }"""
content = content.replace(final_chunk_old, final_chunk_new)

final_symbol_old = """if let Err(e) = db.store_symbols(batch).await {
                    tracing::error!("Failed to store final {} symbols: {}", batch_len, e);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;"""
final_symbol_new = """if let Err(e) = db.store_symbols(batch).await {
                    tracing::error!("Failed to store final {} symbols: {}", batch_len, e);
                }"""
content = content.replace(final_symbol_old, final_symbol_new)

# 4. Збільшуємо розмір батчу
content = content.replace("let batch_size = 12;", "let batch_size = 100;")

# 5. Видаляємо обрізання truncate(50) та truncate(100)
# Нам треба знайти ці рядки і видалити їх
content = re.sub(r'const MAX_CHUNKS_PER_FILE.*?;\s*', '', content)
content = re.sub(r'const MAX_SYMBOLS_PER_FILE.*?;\s*', '', content)
content = re.sub(r'chunks\.truncate\(MAX_CHUNKS_PER_FILE\);\s*', '', content)
content = re.sub(r'symbols\.truncate\(MAX_SYMBOLS_PER_FILE\);\s*', '', content)

# 6. Збільшуємо MAX_CONCURRENT_PARSES з 4 до num_cpus::get() / 2
content = content.replace(
    "const MAX_CONCURRENT_PARSES: usize = 4;",
    "let max_concurrent_parses = std::cmp::max(4, num_cpus::get() / 2);"
)
# Fix JoinSet loop logic since MAX_CONCURRENT_PARSES is now a variable
content = content.replace(
    "while parse_set.len() >= MAX_CONCURRENT_PARSES",
    "while parse_set.len() >= max_concurrent_parses"
)

with open("src/codebase/indexer.rs", "w") as f:
    f.write(content)
