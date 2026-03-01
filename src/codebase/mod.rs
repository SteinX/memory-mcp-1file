pub mod chunker;
pub mod index_worker;
pub mod indexer;
pub mod manager;
pub mod parser;
pub mod relations;
pub mod scanner;
pub mod symbol_index;
pub mod watcher;

pub use index_worker::{IndexJob, IndexJobSender, IndexWorker};
pub use indexer::{incremental_index, index_project};
pub use manager::CodebaseManager;
pub use parser::CodeParser;
pub use relations::{create_symbol_relations, detect_containment_references, RelationStats};
pub use scanner::{detect_language, is_code_file, scan_directory};
pub use symbol_index::{ResolutionContext, SymbolIndex};
pub use watcher::FileWatcher;
