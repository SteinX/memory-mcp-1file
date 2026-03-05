with open("src/codebase/indexer.rs", "r") as f:
    lines = f.readlines()

new_lines = []
skip = False
for line in lines:
    # Remove sleep
    if "tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;" in line:
        continue
    if "tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;" in line:
        continue
    if "tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;" in line:
        continue
    
    # Change batch size
    if "let batch_size = 12;" in line:
        line = line.replace("12", "100")
        
    # Remove truncate logic blocks carefully
    if "if chunks.len() > MAX_CHUNKS_PER_FILE" in line:
        skip = True
        continue
    if "if symbols.len() > MAX_SYMBOLS_PER_FILE" in line:
        skip = True
        continue
    if skip and "}" in line and "tracing::warn!" not in line:
        skip = False
        continue
    if skip:
        continue
        
    if "chunks.truncate(MAX_CHUNKS_PER_FILE);" in line:
        continue
    if "symbols.truncate(MAX_SYMBOLS_PER_FILE);" in line:
        continue
    if "const MAX_CHUNKS_PER_FILE: usize" in line:
        continue
    if "const MAX_SYMBOLS_PER_FILE: usize" in line:
        continue
        
    if "const MAX_CONCURRENT_PARSES: usize = 4;" in line:
        line = "    let max_concurrent_parses = std::cmp::max(4, num_cpus::get() / 2);\n"
    
    if "MAX_CONCURRENT_PARSES" in line:
        line = line.replace("MAX_CONCURRENT_PARSES", "max_concurrent_parses")

    # Add num_cpus import if not there
    if "use std::sync::Arc;" in line and "use num_cpus;" not in "".join(new_lines):
        new_lines.append("use num_cpus;\n")
        
    new_lines.append(line)

with open("src/codebase/indexer.rs", "w") as f:
    f.writelines(new_lines)
