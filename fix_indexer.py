import re

with open("src/codebase/indexer.rs", "r") as f:
    content = f.read()

# Fix MAX_CONCURRENT_PARSES usages
content = content.replace("MAX_CONCURRENT_PARSES", "max_concurrent_parses")

# Remove the logging blocks that reference MAX_CHUNKS_PER_FILE which no longer exists
log_block_chunks = r'if chunks\.len\(\) > MAX_CHUNKS_PER_FILE.*?tracing::warn!.*?\}'
content = re.sub(log_block_chunks, '', content, flags=re.DOTALL)

log_block_symbols = r'if symbols\.len\(\) > MAX_SYMBOLS_PER_FILE.*?tracing::warn!.*?\}'
content = re.sub(log_block_symbols, '', content, flags=re.DOTALL)

with open("src/codebase/indexer.rs", "w") as f:
    f.write(content)
