use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[cfg(all(not(target_env = "msvc"), not(target_os = "windows")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use memory_mcp::codebase::{CodebaseManager, IndexWorker};
use memory_mcp::config::{AppConfig, AppState};
use memory_mcp::embedding::{
    EmbeddingConfig, EmbeddingService, EmbeddingStore, EmbeddingWorker, ModelType,
};
use memory_mcp::lifecycle::{
    install_panic_hook, record_runtime_event_with_details, spawn_heartbeat,
};
use memory_mcp::search::{CodeSearchEngine, MemorySearchEngine};
use memory_mcp::server::MemoryMcpServer;
use memory_mcp::storage::{StorageBackend, SurrealStorage};
use memory_mcp::transport::{serve_http_sse, HttpServerConfig};
use memory_mcp::types::EmbeddingState;

#[derive(Parser)]
#[command(name = "memory-mcp")]
#[command(about = "MCP memory server for AI agents")]
struct Cli {
    #[arg(long, env, default_value_os_t = default_data_dir())]
    data_dir: PathBuf,

    #[arg(long, env = "EMBEDDING_MODEL", default_value = "gemma")]
    model: String,

    #[arg(long, env, default_value = "1000")]
    cache_size: usize,

    #[arg(long, env, default_value = "8")]
    batch_size: usize,

    #[arg(
        long,
        env = "MRL_DIM",
        help = "MRL output dimension (Qwen3/Gemma only). Defaults to model native dim (1024 for qwen3)"
    )]
    mrl_dim: Option<usize>,

    #[arg(long, env = "TIMEOUT_MS", default_value = "30000")]
    timeout: u64,

    /// Maximum time (seconds) a tool call will block waiting for the model to
    /// finish loading. Applies only on the first call on a fresh machine where
    /// the model must be downloaded. Default: 600 s (10 min).
    #[arg(long, env = "MODEL_LOAD_TIMEOUT_SECS", default_value = "600")]
    model_load_timeout_secs: u64,

    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    log_level: String,

    /// Log file path. If specified, logs will be written to this file in addition to stderr.
    /// The file will be rotated when it reaches the maximum size.
    /// Example: --log-file /path/to/logs/memory-mcp.log
    #[arg(long, env = "LOG_FILE")]
    log_file: Option<PathBuf>,

    /// Maximum size of log file in megabytes before rotation (default: 10 MB).
    /// Only effective when --log-file is specified.
    /// Rotated files will be named with timestamp: memory-mcp.2026-04-09_14-30-00.log.1
    #[arg(long, env = "LOG_FILE_MAX_SIZE_MB", default_value = "10")]
    log_file_max_size_mb: u64,

    /// Idle timeout in minutes. 0 = disabled (default, recommended for MCP stdio).
    /// Per MCP spec, stdio servers should exit only on stdin close or signals.
    #[arg(long, env, default_value = "0")]
    idle_timeout: u64,

    #[arg(long)]
    list_models: bool,

    #[arg(long, help = "Use stdio transport mode (backward compatibility)")]
    stdio: bool,

    #[arg(long, env, default_value = "8080", help = "HTTP server port")]
    port: u16,

    #[arg(
        long,
        env,
        default_value = "127.0.0.1",
        help = "HTTP server bind address"
    )]
    bind: String,

    /// SurrealKV block cache capacity in MB. Default: auto-detect based on available RAM.
    /// Set to 0 to use SurrealDB's default (RAM/2, may cause OOM on constrained systems).
    /// Recommended: 256 MB for 16 GB RAM, 512 MB for 32 GB RAM.
    #[arg(long, env = "SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY_MB")]
    block_cache_mb: Option<u64>,
}

fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("memory-mcp")
}

fn init_logging(
    log_level: &str,
    log_file: Option<&PathBuf>,
    max_size_mb: u64,
) -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::new(log_level);

    if let Some(log_path) = log_file {
        let parent = log_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid log file path: no parent directory"))?;
        std::fs::create_dir_all(parent)?;

        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let file_stem = log_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("memory-mcp");
        let extension = log_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("log");

        let base_log_path = parent.join(format!("{}.{}", file_stem, extension));
        let rolling_writer = SizeRollingWriter::new(
            base_log_path,
            timestamp.to_string(),
            max_size_mb * 1024 * 1024,
        );

        let (file_writer, guard) = tracing_appender::non_blocking(rolling_writer);
        std::mem::forget(guard);

        let stderr_writer = std::io::stderr;

        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(file_writer)
                    .with_ansi(false),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(stderr_writer)
                    .with_ansi(true),
            )
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(std::io::stderr)
            .init();
    }

    Ok(())
}

struct SizeRollingWriter {
    base_path: PathBuf,
    timestamp: String,
    max_bytes: u64,
    current_file: std::fs::File,
    current_size: u64,
    rotation_count: u64,
}

impl SizeRollingWriter {
    fn new(base_path: PathBuf, timestamp: String, max_bytes: u64) -> Self {
        let parent = base_path.parent().unwrap();
        let file_stem = base_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("memory-mcp");
        let extension = base_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("log");

        let path = parent.join(format!("{}.{}.{}", file_stem, timestamp, extension));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("Failed to create log file");

        let current_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        Self {
            base_path: path,
            timestamp,
            max_bytes,
            current_file: file,
            current_size,
            rotation_count: 0,
        }
    }

    fn rotate(&mut self) {
        self.rotation_count += 1;
        let parent = self.base_path.parent().unwrap();
        let file_stem = self
            .base_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("memory-mcp");

        let rotated_path = parent.join(format!(
            "{}.{}.log.{}",
            file_stem, self.timestamp, self.rotation_count
        ));

        if let Err(e) = std::fs::rename(&self.base_path, &rotated_path) {
            eprintln!("Failed to rotate log file: {}", e);
        }

        self.current_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.base_path)
            .expect("Failed to create new log file");
        self.current_size = 0;
    }
}

impl std::io::Write for SizeRollingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.current_size + buf.len() as u64 > self.max_bytes {
            self.rotate();
        }

        let written = self.current_file.write(buf)?;
        self.current_size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.current_file.flush()
    }
}
/// Pin ML compute thread pools to 2 threads to prevent CPU contention with
/// the tokio runtime. Must be called before any thread pool is initialized.
///
/// SAFETY: `std::env::set_var` is unsafe since Rust 1.66 because concurrent
/// reads from other threads are UB. This is safe here because `main()` calls
/// us before `tokio::runtime::Builder::build()`, so no other threads exist yet.
fn pin_compute_threads() {
    assert!(
        std::thread::current().name() == Some("main"),
        "pin_compute_threads must be called from the main thread before spawning any threads"
    );
    for var in ["RAYON_NUM_THREADS", "MKL_NUM_THREADS", "OMP_NUM_THREADS"] {
        if std::env::var(var).is_err() {
            unsafe { std::env::set_var(var, "2") };
        }
    }
}

/// Configure SurrealKV block cache to prevent OOM on constrained systems.
/// SurrealDB defaults to RAM/2 which is too aggressive when other processes consume memory.
///
/// Priority: env var > CLI arg > auto-detect
///
/// Auto-detect logic: min(available_RAM * 0.2, 512 MB)
///
/// SAFETY: Must be called before any SurrealDB initialization (single-threaded).
fn configure_block_cache(cli_cache_mb: Option<u64>) {
    const ENV_VAR: &str = "SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY";
    const ENV_VAR_MB: &str = "SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY_MB";

    // Priority 1: Check if user already set the raw bytes env var
    if std::env::var(ENV_VAR).is_ok() {
        return;
    }

    // Priority 2: Use CLI arg if provided (convert MB to bytes)
    if let Some(mb) = cli_cache_mb {
        if mb > 0 {
            let bytes = mb * 1024 * 1024;
            unsafe {
                std::env::set_var(ENV_VAR, bytes.to_string());
            }
        }
        return;
    }

    // Priority 3: Check MB env var
    if let Ok(mb_str) = std::env::var(ENV_VAR_MB) {
        if let Ok(mb) = mb_str.parse::<u64>() {
            if mb > 0 {
                let bytes = mb * 1024 * 1024;
                unsafe {
                    std::env::set_var(ENV_VAR, bytes.to_string());
                }
            }
        }
        return;
    }

    // Priority 4: Auto-detect based on available memory
    #[cfg(target_os = "macos")]
    {
        if let Some(available_mb) = get_available_memory_mb_macos() {
            // Use 20% of available RAM, max 512 MB
            let cache_mb = std::cmp::min((available_mb as f64 * 0.2) as u64, 512);
            let cache_bytes = cache_mb * 1024 * 1024;
            unsafe {
                std::env::set_var(ENV_VAR, cache_bytes.to_string());
            }
            eprintln!(
                "[memory-mcp] Auto-configured block cache: {} MB (available RAM: {} MB)",
                cache_mb, available_mb
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // On non-macOS, default to conservative 256 MB to avoid OOM
        let cache_bytes = 256u64 * 1024 * 1024;
        unsafe {
            std::env::set_var(ENV_VAR, cache_bytes.to_string());
        }
        eprintln!("[memory-mcp] Auto-configured block cache: 256 MB (default for non-macOS)");
    }
}

#[cfg(target_os = "macos")]
fn get_available_memory_mb_macos() -> Option<u64> {
    let mut size = std::mem::size_of::<u64>();
    let mut free_pages: u64 = 0;

    let result = unsafe {
        libc::sysctlbyname(
            b"vm.page_free_count\0".as_ptr() as *const i8,
            &mut free_pages as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };

    if result != 0 {
        return None;
    }

    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    let free_bytes = free_pages * page_size;
    Some(free_bytes / (1024 * 1024))
}

fn main() -> anyhow::Result<()> {
    pin_compute_threads();
    let cli = Cli::parse();
    configure_block_cache(cli.block_cache_mb);

    // Candle ML models (especially Qwen3) allocate large tensors on the stack
    // during forward passes. 16 MB covers both SurrealKV WAL recovery and
    // any spawn_blocking inference that inherits the worker stack size.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024) // 16 MB
        .build()?;

    runtime.block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> anyhow::Result<()> {
    if cli.list_models {
        println!("Available models:");
        println!("  qwen3     - 1024 dim, ~1.2 GB          [Apache 2.0] Top open-source 2026, MRL, 32K ctx");
        println!(
            "  gemma     -  768 dim, ~195 MB (default) [Gemma license] Lightweight MRL alternative"
        );
        println!(
            "  bge_m3    - 1024 dim, ~420 MB           [MIT] Hybrid dense+sparse+colbert retrieval"
        );
        println!(
            "  nomic     -  768 dim, ~270 MB           [Apache 2.0] Long-context BERT-compatible"
        );
        println!(
            "  e5_multi  -  768 dim, ~180 MB           [MIT] Legacy; kept for backward compat"
        );
        println!("  e5_small  -  384 dim,  ~85 MB           [MIT] Minimal RAM, dev/testing only");
        println!();
        println!(
            "NOTE: gemma uses Gemma license (not Apache 2.0). Review terms before commercial use."
        );
        return Ok(());
    }

    init_logging(
        &cli.log_level,
        cli.log_file.as_ref(),
        cli.log_file_max_size_mb,
    )?;

    let runtime_mode = if cli.stdio { "stdio" } else { "http_sse" };
    install_panic_hook(cli.data_dir.clone(), runtime_mode.to_string());
    record_runtime_event_with_details(
        &cli.data_dir,
        "last_start.json",
        "process_start",
        runtime_mode,
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "model": cli.model,
            "bind": cli.bind,
            "port": cli.port,
            "log_level": cli.log_level,
        }),
    );

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        ppid = unsafe { libc::getppid() },
        mode = runtime_mode,
        model = %cli.model,
        data_dir = %cli.data_dir.display(),
        "memory-mcp starting"
    );

    let model: ModelType = cli.model.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    if model.requires_license_agreement() {
        tracing::warn!(
            "LEGAL NOTICE: Model '{}' operates under a proprietary license (not Apache 2.0). \
             Review terms before commercial use.",
            model
        );
    }

    let embedding_config = EmbeddingConfig {
        model,
        cache_size: cli.cache_size,
        batch_size: cli.batch_size,
        mrl_dim: cli.mrl_dim,
        cache_dir: Some(cli.data_dir.join("models")),
    };

    embedding_config
        .validate()
        .map_err(|e| anyhow::anyhow!("Invalid embedding configuration: {}", e))?;

    let storage =
        Arc::new(SurrealStorage::new(&cli.data_dir, embedding_config.output_dim()).await?);

    if let Err(e) = storage.check_dimension(embedding_config.output_dim()).await {
        tracing::warn!("Dimension check: {}", e);
    }

    tracing::info!(output_dim = embedding_config.output_dim(), model = %embedding_config.model, "Embedding engine configured");

    // Initialize Embedding Store (L1/L2 Cache)
    let embedding_store = Arc::new(EmbeddingStore::new(&cli.data_dir, model.repo_id())?);

    let embedding = Arc::new(EmbeddingService::new(embedding_config));
    embedding.start_loading();

    let metrics = std::sync::Arc::new(memory_mcp::embedding::EmbeddingMetrics::new());
    let (queue_tx, queue_rx) = tokio::sync::mpsc::channel(256);
    let adaptive_queue =
        memory_mcp::embedding::AdaptiveEmbeddingQueue::with_defaults(queue_tx, metrics.clone());

    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);

    let requeue_tx = adaptive_queue.requeue_sender();

    let state = Arc::new(AppState {
        config: AppConfig {
            data_dir: cli.data_dir,
            model: cli.model,
            cache_size: cli.cache_size,
            batch_size: cli.batch_size,
            timeout_ms: cli.timeout,
            log_level: cli.log_level,
            model_load_timeout_ms: cli.model_load_timeout_secs * 1000,
            // New fields: use compile-time defaults (values are documented in AppConfig::default)
            ..AppConfig::default()
        },
        storage: storage.clone(),
        embedding: embedding.clone(),
        embedding_store: embedding_store.clone(),
        embedding_queue: adaptive_queue,
        progress: memory_mcp::config::IndexProgressTracker::new(),
        db_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
        code_search: Arc::new(CodeSearchEngine::new()),
        memory_search: Arc::new(MemorySearchEngine::new()),
        indexing_projects: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        shutdown_tx,
        index_pending: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    });

    spawn_heartbeat(
        state.config.data_dir.clone(),
        runtime_mode.to_string(),
        state.shutdown_rx(),
    );

    let worker = EmbeddingWorker::new(
        queue_rx,
        requeue_tx,
        embedding.get_engine(),
        embedding_store.clone(),
        state.clone(),
    );
    tokio::spawn(async move {
        match tokio::spawn(worker.run()).await {
            Ok(count) => tracing::info!(count, "Embedding worker finished"),
            Err(e) => tracing::error!("Embedding worker panicked: {}", e),
        }
    });

    let monitor_state = state.clone();
    let monitor_shutdown = state.shutdown_rx();
    tokio::spawn(memory_mcp::embedding::run_completion_monitor(
        monitor_state,
        monitor_shutdown,
    ));

    // Warm the in-memory BM25 index from existing DB data (background, non-blocking)
    let bm25_state = state.clone();
    tokio::spawn(async move {
        let count = bm25_state
            .code_search
            .load_all_from_storage(bm25_state.storage.as_ref())
            .await;
        if count > 0 {
            tracing::info!(chunks = count, "BM25 in-memory index warmed from DB");
        }
    });

    let memory_bm25_state = state.clone();
    tokio::spawn(async move {
        let count = memory_bm25_state
            .memory_search
            .load_all_from_storage(memory_bm25_state.storage.as_ref())
            .await;
        if count > 0 {
            tracing::info!(memories = count, "Memory lexical index warmed from DB");
        }
    });

    // Re-embed stale memories (background, non-blocking, throttled)
    let reembed_state = state.clone();
    let reembed_engine = embedding.get_engine();
    tokio::spawn(async move {
        // Wait for embedding engine to be ready
        loop {
            let guard = reembed_engine.read().await;
            if guard.is_some() {
                break;
            }
            drop(guard);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        let stale_memories = match reembed_state.storage.get_stale_memories().await {
            Ok(memories) => memories,
            Err(e) => {
                tracing::warn!("Failed to query stale memories: {}", e);
                return;
            }
        };

        if stale_memories.is_empty() {
            return;
        }

        tracing::info!(count = stale_memories.len(), "Re-embedding stale memories");
        let mut re_embedded = 0u32;
        for memory in &stale_memories {
            let mem_id = match &memory.id {
                Some(thing) => memory_mcp::types::record_key_to_string(&thing.key),
                None => continue,
            };
            let content = memory.content.clone();
            let engine_clone = reembed_engine.clone();
            // Use spawn_blocking to avoid stack overflow — ML model needs large stack
            let emb_result = tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                let guard = rt.block_on(engine_clone.read());
                match guard.as_ref() {
                    Some(engine) => engine.embed(&content).ok(),
                    None => None,
                }
            })
            .await;

            if let Ok(Some(emb)) = emb_result {
                let hash = blake3::hash(memory.content.as_bytes()).to_hex().to_string();
                if let Err(e) = reembed_state
                    .storage
                    .raw_update_embedding(&mem_id, emb, hash, &EmbeddingState::Current.to_string())
                    .await
                {
                    tracing::warn!(id = %mem_id, error = %e, "Failed to update re-embedded memory");
                } else {
                    re_embedded += 1;
                }
            } else {
                tracing::warn!(id = %mem_id, "Failed to re-embed memory");
            }
            // Throttle: 1 memory per second to avoid CPU contention
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        if re_embedded > 0 {
            tracing::info!(
                count = re_embedded,
                "Stale memories re-embedded successfully"
            );
        }
    });

    let server = MemoryMcpServer::new(state.clone());

    // ── Lazy-init architecture ─────────────────────────────────────────────
    // `serve_server` is called immediately after lightweight synchronous setup.
    // The MCP `initialize` handshake is handled by `get_info()` which is a
    // pure, synchronous function — it returns in < 1 ms regardless of model
    // state.  The embedding model continues loading in a background OS thread
    // (`start_loading` above).
    //
    // Tool calls that need embeddings use `ensure_embedding_ready!`, which now
    // *waits* up to `model_load_timeout_ms` for the model instead of failing
    // immediately.  This means:
    //   • Fresh machine (model must download):  tool calls block transparently
    //     until the download completes; the MCP session stays alive.
    //   • Warm machine (model cached):  the model is ready in < 5 s; tool
    //     calls proceed with zero perceptible delay.
    //
    // This is the architecturally correct fix for the SIGTERM-on-initialize
    // bug: the server ALWAYS responds to `initialize` instantly; only the
    // heavier tool calls experience startup latency, and only once.
    // ──────────────────────────────────────────────────────────────────────

    // Auto-start codebase manager if /project exists
    let project_path = PathBuf::from("/project");
    if project_path.exists() {
        let project_id = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        // Create the IndexWorker for this project and start it in the background.
        let (index_worker, index_tx) = IndexWorker::new(state.clone(), project_id.clone());
        // Register the pending-job counter in AppState so the status API can read it.
        state
            .index_pending
            .write()
            .await
            .insert(project_id.clone(), index_tx.pending_arc());
        index_worker.start(state.shutdown_rx());

        // Build the CodebaseManager with the channel sender and start it.
        let mgr = CodebaseManager::new(state.clone(), project_path.clone(), index_tx.clone());
        if let Err(e) = mgr.start().await {
            tracing::error!(error = %e, "Failed to start codebase manager for /project");
        }
        let mgr = Arc::new(mgr);

        // ── Periodic manifest-diff task ─────────────────────────────────
        // Every N minutes, run a lightweight manifest diff and push any
        // discovered changes into the IndexWorker channel.  This catches
        // files that were missed by the file-system watcher (e.g. because
        // the process was restarted or the watcher lost events under heavy load).
        let mgr_for_diff = mgr.clone();
        let mut diff_shutdown = state.shutdown_rx();
        let manifest_diff_interval_mins = state.config.manifest_diff_interval_mins;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(manifest_diff_interval_mins * 60));
            interval.tick().await; // skip immediate first tick

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        tracing::debug!(
                            project_id = %project_id,
                            "Periodic manifest diff starting"
                        );
                        if let Err(e) = mgr_for_diff.validate_index_full().await {
                            tracing::warn!(
                                project_id = %project_id,
                                error = %e,
                                "Periodic manifest diff failed"
                            );
                        }
                    }
                    _ = diff_shutdown.changed() => {
                        if *diff_shutdown.borrow() {
                            tracing::debug!(project_id = %project_id, "Manifest diff task stopping");
                            break;
                        }
                    }
                }
            }
        });
    }

    if cli.stdio {
        run_stdio_mode(server, state, cli.idle_timeout).await?;
    } else {
        run_http_mode(server, cli.bind, cli.port, state).await?;
    }

    Ok(())
}

async fn run_stdio_mode(
    server: MemoryMcpServer,
    state: Arc<AppState>,
    idle_timeout: u64,
) -> anyhow::Result<()> {
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::service::serve_server(server, transport).await?;

    if idle_timeout > 0 {
        tracing::warn!(
            minutes = idle_timeout,
            "Non-zero idle timeout is not recommended for MCP stdio transport. \
             Per MCP spec, stdio servers should exit only when stdin is closed or on signals."
        );
    }

    tracing::info!("Server started in stdio mode, waiting for client disconnect or signals...");

    let stdin_closed = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let idle_future = async {
        if idle_timeout > 0 {
            tokio::time::sleep(Duration::from_secs(idle_timeout * 60)).await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    let stdin_closed_flag = stdin_closed.clone();
    let shutdown_reason = tokio::select! {
        res = service.waiting() => {
            stdin_closed_flag.store(true, Ordering::SeqCst);
            match res {
                Ok(_) => {
                    tracing::info!("Client disconnected (stdin closed)");
                    "client_disconnect"
                }
                Err(e) => {
                    tracing::error!("Server error: {}", e);
                    "server_error"
                }
            }
        },
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutting down gracefully... (SIGINT)");
            "sigint"
        },
        _ = async {
            #[cfg(unix)]
            { terminate.recv().await; }
            #[cfg(not(unix))]
            { std::future::pending::<()>().await; }
        } => {
            if !stdin_closed.load(Ordering::SeqCst) {
                tracing::warn!(
                    "SIGTERM received while stdin still open. Client may have violated MCP spec."
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            "sigterm"
        },
        _ = idle_future => {
            tracing::info!(minutes = idle_timeout, "Idle timeout reached, shutting down");
            "idle_timeout"
        }
    };

    record_runtime_event_with_details(
        &state.config.data_dir,
        "last_shutdown.json",
        "graceful_shutdown_started",
        "stdio",
        serde_json::json!({
            "reason": shutdown_reason,
            "idle_timeout_minutes": idle_timeout,
        }),
    );

    tracing::info!("Initiating graceful shutdown...");
    let _ = state.shutdown_tx.send(true);
    if let Err(e) = state.storage.shutdown().await {
        tracing::warn!("Database shutdown error: {}", e);
    }
    tracing::info!("Shutdown complete");
    record_runtime_event_with_details(
        &state.config.data_dir,
        "last_shutdown.json",
        "graceful_shutdown_complete",
        "stdio",
        serde_json::json!({
            "reason": shutdown_reason,
        }),
    );
    Ok(())
}

async fn run_http_mode(
    server: MemoryMcpServer,
    bind: String,
    port: u16,
    state: Arc<AppState>,
) -> anyhow::Result<()> {
    let config = HttpServerConfig { bind, port };

    tracing::info!(
        "Server starting in HTTP SSE mode on http://{}:{}",
        config.bind,
        config.port
    );
    serve_http_sse(move || Ok(server.clone()), config, state).await
}
