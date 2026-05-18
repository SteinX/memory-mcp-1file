pub mod chunker;
pub mod freshness;
pub mod index_worker;
pub mod indexer;
pub mod manager;
pub mod parser;
pub mod project_registry;
pub mod relations;
pub mod resolver;
pub mod scanner;
pub mod session_binding;
pub mod startup;
pub mod symbol_index;
pub mod watcher;

pub use index_worker::{IndexJob, IndexJobSender, IndexWorker};
pub use indexer::{incremental_index, index_project};
pub use manager::{resume_embeddings_for_project, CodebaseManager};
pub use parser::CodeParser;
pub use project_registry::{
    ProjectLifecycle, ProjectLifecycleOptions, ProjectLifecycleState, ProjectLifecycleStatus,
    ProjectManagerHandle, ProjectRegistry, ProjectRegistryError, ProjectRegistryPolicy,
    ProjectRootConflict, ProjectWorkerHandle, ProjectWorkerSenderHandle,
};
pub use relations::{create_symbol_relations, detect_containment_references, RelationStats};
pub use scanner::{detect_language, is_code_file, scan_directory};
pub use session_binding::{SessionBinding, SessionBindingStatus, SessionBindingStore};
pub use symbol_index::{ResolutionContext, SymbolIndex};
pub use watcher::FileWatcher;
