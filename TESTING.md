# Code Context MCP - Testing Guide

## Quick Test

### 1. Test MCP Protocol

```bash
# Test initialize
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | code-context-mcp

# Expected response:
# {"jsonrpc":"2.0","id":1,"result":{"capabilities":{"tools":{"listChanged":true}},"protocolVersion":"2024-11-05","serverInfo":{"name":"code-context-mcp","version":"0.1.0"}}}
```

### 2. Test Tools List

```bash
cat << 'EOF' | timeout 5 code-context-mcp
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
EOF
```

### 3. Test Tool Call

```bash
cat << 'EOF' | timeout 5 code-context-mcp
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_indexing_status","arguments":{"path":"/tmp"}}}
EOF
```

## About stdio Transport

MCP uses **stdio** for communication between client and server:

```
┌─────────────┐         ┌──────────────┐
│   Client    │  stdin  │    Server    │
│  (OpenCode) │────────▶│ (code-context│
│             │         │     -mcp)    │
│             │◀───────▶│              │
│             │ stdout  │              │
└─────────────┘         └──────────────┘
                              │
                          stderr
                              ▼
                        (logs only)
```

### Stream Usage

| Stream | Direction | Content |
|--------|-----------|---------|
| `stdin` | Client → Server | JSON-RPC requests |
| `stdout` | Server → Client | JSON-RPC responses |
| `stderr` | Server → Client | Logs (ignored by client) |

### Configuration

**No need to specify stdin/stdout in config:**

```json
{
  "mcp": {
    "code-context-mcp": {
      "type": "local",
      "command": ["code-context-mcp"],  // ✅ Correct
      "environment": {...}
    }
  }
}
```

**NOT like this:**

```json
{
  "mcp": {
    "code-context-mcp": {
      "command": ["code-context-mcp", "stdin"],  // ❌ Wrong
      ...
    }
  }
}
```

## Debug Mode

Enable debug logging:

```bash
RUST_LOG=debug code-context-mcp
```

Logs will appear on stderr, separate from protocol messages.

## Common Issues

### Issue: No response from server

**Check:**
1. Ollama is running: `ollama serve`
2. Milvus is running: `docker ps | grep milvus`
3. Environment variables are set

### Issue: JSON parse errors

**Check:**
- Requests must be valid JSON-RPC 2.0
- Each request on a separate line
- No trailing commas

### Issue: Binary not found

**Solution:**
```bash
# Install
cargo install --path .

# Or use full path
~/.cargo/bin/code-context-mcp
```

## Performance Test

```bash
# Time server startup
time (echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | code-context-mcp)

# Expected: <100ms for Rust version
```
