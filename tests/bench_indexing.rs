use std::path::PathBuf;
use std::time::Instant;

use memory_mcp::codebase::indexer::index_project_with_metrics;
use memory_mcp::test_utils::TestContext;
use serde_json::json;

fn count_corpus_recursive(dir: &PathBuf) -> (u64, u64) {
    let mut file_count = 0u64;
    let mut byte_count = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let (fc, bc) = count_corpus_recursive(&path);
            file_count += fc;
            byte_count += bc;
        } else if path.is_file() {
            file_count += 1;
            byte_count += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (file_count, byte_count)
}

#[tokio::test(flavor = "multi_thread")]
async fn bench_indexing_src_directory() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");
    assert!(src_dir.exists(), "src/ not found at {:?}", src_dir);

    let (corpus_file_count, corpus_byte_count) = count_corpus_recursive(&src_dir);
    let cpu_count = num_cpus::get();
    let rayon_num_threads =
        std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "auto".to_string());

    let ctx = TestContext::new().await;
    let state = ctx.state.clone();

    let cold_or_warm = "cold";

    let wall_start = Instant::now();
    let (index_status, metrics) = index_project_with_metrics(state, &src_dir)
        .await
        .expect("index_project_with_metrics should succeed");
    let total_wall_time_ms = wall_start.elapsed().as_millis();

    let evidence = json!({
        "benchmark": "task-1-indexing-benchmark",
        "corpus": {
            "path": src_dir.to_string_lossy(),
            "corpus_file_count": corpus_file_count,
            "corpus_byte_count": corpus_byte_count
        },
        "environment": {
            "cpu_count": cpu_count,
            "rayon_num_threads": rayon_num_threads,
            "cold_or_warm": cold_or_warm
        },
        "timings": {
            "total_wall_time_ms": total_wall_time_ms,
            "file_read_hash_elapsed_ms": metrics.file_read_hash_elapsed_ms,
            "parse_chunk_elapsed_ms": metrics.parse_chunk_elapsed_ms,
            "chunk_db_write_elapsed_ms": metrics.chunk_db_write_elapsed_ms,
            "symbol_db_write_elapsed_ms": metrics.symbol_db_write_elapsed_ms,
            "embedding_enqueue_elapsed_ms": metrics.embedding_enqueue_elapsed_ms,
            "relation_create_elapsed_ms": metrics.relation_create_elapsed_ms,
            "status_update_elapsed_ms": metrics.status_update_elapsed_ms
        },
        "counts": {
            "files_read": metrics.files_read,
            "chunks_written": metrics.chunks_written,
            "symbols_written": metrics.symbols_written,
            "embeddings_enqueued": metrics.embeddings_enqueued
        },
        "index_status": {
            "state": format!("{:?}", index_status.status),
            "total_files": index_status.total_files,
            "indexed_files": index_status.indexed_files,
            "total_chunks": index_status.total_chunks,
            "total_symbols": index_status.total_symbols,
            "failed_files_count": index_status.failed_files.len()
        }
    });

    let evidence_path = manifest_dir.join(".sisyphus/evidence/task-1-indexing-benchmark.json");
    std::fs::create_dir_all(evidence_path.parent().unwrap())
        .expect("create evidence dir");
    std::fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&evidence).unwrap(),
    )
    .expect("write evidence file");

    println!("\n=== Benchmark Evidence ===");
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
    println!("Evidence written to: {}", evidence_path.display());

    assert!(
        metrics.file_read_hash_elapsed_ms > 0,
        "file_read_hash_elapsed_ms should be non-zero"
    );
    assert!(
        metrics.parse_chunk_elapsed_ms > 0,
        "parse_chunk_elapsed_ms should be non-zero"
    );
    assert!(
        metrics.chunk_db_write_elapsed_ms > 0,
        "chunk_db_write_elapsed_ms should be non-zero"
    );
    assert!(
        metrics.symbol_db_write_elapsed_ms > 0,
        "symbol_db_write_elapsed_ms should be non-zero"
    );
}
