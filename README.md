# Code Context MCP (Rust)

A Rust implementation of Code Context MCP server for semantic code search.

> 📚 **Implementation Details**: See [IMPLEMENTATION.md](./IMPLEMENTATION.md) for architecture decisions, reserved features explanation, and complete configuration guide.

## Features

- 🦀 **Pure Rust** - High performance, low memory footprint
- 🔍 **Semantic Search** - Vector-based code search using embeddings
- 🌐 **Multi-language** - Support for Rust, TypeScript, JavaScript, Python, Go, Java, C++, C#
- 🧠 **AST-based Chunking** - Intelligent code splitting using tree-sitter
- 📦 **MCP Protocol** - Compatible with Claude Code, Cursor, and other MCP clients
- 💾 **Incremental Indexing** - Only re-index changed files using file hashing

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Code Context MCP                          │
├─────────────────────────────────────────────────────────────┤
│  MCP Server (JSON-RPC 2.0 over stdio)                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │ index_codebase│  │ search_code  │  │ clear_index  │       │
│  └──────────────┘  └──────────────┘  └──────────────┘       │
├─────────────────────────────────────────────────────────────┤
│  Core Components                                             │
│  ┌──────────────────┐  ┌──────────────────┐                 │
│  │ Embedding        │  │ Vector Database  │                 │
│  │ (Ollama/OpenAI)  │  │ (Milvus)         │                 │
│  └──────────────────┘  └──────────────────┘                 │
│  ┌──────────────────┐                                        │
│  │ Code Parser      │                                        │
│  │ (tree-sitter)    │                                        │
│  └──────────────────┘                                        │
└─────────────────────────────────────────────────────────────┘
```

## Prerequisites

1. **Ollama** (for local embeddings)
   ```bash
   # Install Ollama
   curl -fsSL https://ollama.ai/install.sh | sh
   
   # Pull embedding model
   ollama pull nomic-embed-text
   ```

2. **Milvus** (vector database)
   ```bash
   # Using Docker
   docker run -d -p 19530:19530 milvusdb/milvus:v2.3.21
   ```

## Installation

### Option 1: Cargo Install (Recommended)

```bash
# Install from local source
cd /home/de/works/code-context-mcp
cargo install --path .

# Binary will be installed to ~/.cargo/bin/code-context-mcp
# You can run it directly: code-context-mcp
```

### Option 2: Build Manually

```bash
# Build
cargo build --release

# The binary will be at target/release/code-context-mcp
# Run manually: ./target/release/code-context-mcp
```

### Option 3: Use Directly After Build

```bash
# Run without installing
cargo run --release
```

## Configuration

Copy `.env.example` to `.env` and configure:

```bash
cp .env.example .env
```

Edit `.env`:
```env
OLLAMA_HOST=http://127.0.0.1:11434
EMBEDDING_MODEL=nomic-embed-text
MILVUS_ADDRESS=http://127.0.0.1:19530
MAX_INDEXED_PROJECTS=10
RUST_LOG=info
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `OLLAMA_HOST` | No | `http://127.0.0.1:11434` | Ollama service address |
| `EMBEDDING_MODEL` | No | `nomic-embed-text` | Embedding model name |
| `MILVUS_ADDRESS` | No | `http://127.0.0.1:19530` | Milvus vector database address |
| `SNAPSHOT_PATH` | No | `~/.code-context/snapshot.json` | Snapshot storage path |
| `MAX_INDEXED_PROJECTS` | No | `10` | Max indexed projects (LRU eviction) |
| `RUST_LOG` | No | - | Log level (info/debug/error) |

## Usage with MCP Clients

### OpenCode

Add to `~/.config/opencode/opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "code-context-mcp": {
      "type": "local",
      "command": ["code-context-mcp"],
      "enabled": true,
      "environment": {
        "OLLAMA_HOST": "http://127.0.0.1:11434",
        "EMBEDDING_MODEL": "nomic-embed-text",
        "MILVUS_ADDRESS": "http://127.0.0.1:19530",
        "RUST_LOG": "info"
      }
    }
  }
}
```

> **Note**: MCP uses **stdio** for communication. The server reads JSON-RPC requests from `stdin` and writes responses to `stdout`. Logs are written to `stderr` and won't interfere with the protocol. No need to specify `stdin` in the configuration.

### Claude Code

### Claude Code

```bash
claude mcp add code-context-mcp \
  -- npx -y code-context-mcp
```

Or with custom path:
```bash
claude mcp add code-context-mcp-rust \
  -e OLLAMA_HOST=http://127.0.0.1:11434 \
  -e MILVUS_ADDRESS=http://127.0.0.1:19530 \
  -- /path/to/code-context-mcp/target/release/code-context-mcp
```

### Cursor

Add to `~/.cursor/mcp.json`:
```json
{
  "mcpServers": {
    "code-context-mcp": {
      "command": "/path/to/code-context-mcp/target/release/code-context-mcp",
      "env": {
        "OLLAMA_HOST": "http://127.0.0.1:11434",
        "MILVUS_ADDRESS": "http://127.0.0.1:19530"
      }
    }
  }
}
```

### OpenAI Codex CLI

Add to `~/.codex/config.toml`:
```toml
[mcp_servers.code-context-mcp]
command = "/path/to/code-context-mcp/target/release/code-context-mcp"
env = { OLLAMA_HOST = "http://127.0.0.1:11434", MILVUS_ADDRESS = "http://127.0.0.1:19530" }
```

## Available Tools

### `index_codebase`

Index a codebase directory for semantic search.

**Multi-Project Support**: Each project is indexed independently. When `MAX_INDEXED_PROJECTS` is exceeded, the oldest project is automatically evicted (LRU).

```json
{
  "name": "index_codebase",
  "arguments": {
    "path": "/absolute/path/to/codebase",
    "force": false
  }
}
```

### `search_code`

Search the indexed codebase.

**Cross-Project Search**: Set `cross_project: true` or use `path: "all"` to search across all indexed projects.

```json
{
  "name": "search_code",
  "arguments": {
    "path": "/absolute/path/to/codebase",
    "query": "find functions that handle authentication",
    "limit": 10,
    "cross_project": false
  }
}
```

### `clear_index`

Clear the search index. Use `path: "all"` to clear all indexed projects.

```json
{
  "name": "clear_index",
  "arguments": {
    "path": "/absolute/path/to/codebase"
  }
}
```

### `get_indexing_status`

Get indexing status. Use `path: "all"` to see all indexed projects.

```json
{
  "name": "get_indexing_status",
  "arguments": {
    "path": "/absolute/path/to/codebase"
  }
}
```

## Supported Languages

| Language | Extensions | Parser |
|----------|------------|--------|
| Rust | .rs | tree-sitter-rust |
| TypeScript | .ts, .tsx | tree-sitter-typescript |
| JavaScript | .js | tree-sitter-javascript |
| Python | .py | tree-sitter-python |
| Go | .go | tree-sitter-go |
| C++ | .cpp, .cc | tree-sitter-cpp |
| Java | .java | tree-sitter-java |
| C# | .cs | tree-sitter-c-sharp |

## Reserved Features

This implementation uses a **progressive development** strategy. Some features are reserved for future use:

- **OpenAI Embedding** - Currently uses Ollama (local, free). OpenAI provider is implemented but not enabled by default.
- **Notification Support** - MCP notification protocol is reserved for future push notifications (indexing progress, file changes).
- **Roots Capability** - Multi-project/monorepo support is planned.
- **Variable-level Search** - Currently focuses on function/class level. Variable search is reserved.

See [IMPLEMENTATION.md](./IMPLEMENTATION.md) for detailed explanations.

## Development

```bash
# Run in development mode
cargo run

# Run tests
cargo test

# Build release
cargo build --release

# Check code
cargo clippy

# Format code
cargo fmt
```

## Comparison with JavaScript Version

| Feature | Rust | JavaScript |
|---------|------|------------|
| Performance | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ |
| Memory Usage | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ |
| AST Parsing | tree-sitter | tree-sitter |
| Embedding | Ollama/OpenAI | Ollama/OpenAI/Voyage |
| Vector DB | Milvus REST | Milvus SDK |
| Binary Size | ~10MB | Node.js required |
| Startup Time | <100ms | ~500ms |

## License

MIT
