# Code Context MCP - 实现文档

## 📋 概述

本文档说明 Code Context MCP Rust 实现的架构决策、预留功能的原因，以及完整的配置和使用方式。

---

## 🏗️ 架构设计

### 核心组件

```
┌─────────────────────────────────────────────────────────────────┐
│                        MCP Client                               │
│                    (Cursor, Claude Code, etc.)                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ stdio (JSON-RPC 2.0)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Code Context MCP Server                       │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Protocol Layer (protocol.rs)                             │  │
│  │  - JSON-RPC 2.0 解析/序列化                                │  │
│  │  - 请求路由                                                │  │
│  └───────────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Server Layer (server.rs)                                 │  │
│  │  - MCP 协议处理器                                          │  │
│  │  - 工具注册和调用                                          │  │
│  └───────────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Tool Handlers (handlers/)                                │  │
│  │  - index_codebase                                         │  │
│  │  - search_code                                            │  │
│  │  - clear_index                                            │  │
│  │  - get_indexing_status                                    │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
     ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
     │  Embedding  │ │  Vector DB  │ │   Parser    │
     │  Provider   │ │  Provider   │ │  Provider   │
     │  (Ollama/   │ │  (Milvus)   │ │  (tree-     │
     │   OpenAI)   │ │             │ │   sitter)   │
     └─────────────┘ └─────────────┘ └─────────────┘
```

---

## 🔮 预留功能说明

### 为什么需要预留？

本项目采用**渐进式实现**策略，原因如下：

1. **MCP 协议完整性** - 保留标准协议定义，确保与未来客户端兼容
2. **功能扩展性** - 为后续功能迭代预留接口
3. **多 Provider 支持** - 支持多种 Embedding 和向量数据库
4. **代码可维护性** - 清晰的接口定义便于团队协作

### 预留功能列表

#### 1. OpenAI Embedding Provider (`embedding/openai.rs`)

```rust
#![allow(dead_code)] // 预留实现
```

**预留原因：**
- 当前默认使用 Ollama 本地 Embedding（免费、隐私）
- OpenAI 需要 API Key，适合生产环境
- 接口已实现，用户可自行切换

**使用场景：**
```env
# 切换到 OpenAI
EMBEDDING_PROVIDER=openai
OPENAI_API_KEY=sk-xxx
EMBEDDING_MODEL=text-embedding-3-small
```

#### 2. Notification 支持 (`mcp/types.rs`)

```rust
#[allow(dead_code)]
pub struct Notification { ... }
```

**预留原因：**
- MCP 协议支持服务器主动推送通知
- 当前实现为简化版（请求 - 响应模式）
- 未来可实现：
  - 索引进度推送
  - 文件变更通知
  - 错误告警

**未来实现示例：**
```rust
// 推送索引进度
server.send_notification("codecontext/indexing/progress", json!({
    "percentage": 45,
    "file": "src/main.rs"
})).await?;
```

#### 3. Roots Capability (`mcp/types.rs`)

```rust
#[allow(dead_code)]
pub struct RootsCapability { ... }
```

**预留原因：**
- MCP 协议允许客户端声明"roots"（项目根目录）
- 当前实现假设单项目模式
- 未来支持多项目/monorepo 场景

**未来使用：**
```json
{
  "roots": [
    {"uri": "file:///home/user/project-a", "name": "Project A"},
    {"uri": "file:///home/user/project-b", "name": "Project B"}
  ]
}
```

#### 4. SymbolKind::Variable (`parser/mod.rs`)

```rust
#[allow(dead_code)]
Variable,  // 预留
```

**预留原因：**
- 当前 AST 解析主要关注函数、类等大型符号
- 变量级搜索需要更细粒度的分析
- 未来可实现"查找所有使用某变量的位置"

#### 5. invalid_request 错误类型

```rust
#[allow(dead_code)]
pub fn invalid_request() -> Self { ... }
```

**预留原因：**
- JSON-RPC 标准错误类型之一
- 当前实现中请求验证在应用层处理
- 保留用于未来协议层验证

---

## ✅ 已实现功能

### MCP 协议支持

| 方法 | 状态 | 说明 |
|------|------|------|
| `initialize` | ✅ | 握手，交换协议版本和能力 |
| `notifications/initialized` | ✅ | 客户端确认初始化完成 |
| `tools/list` | ✅ | 返回可用工具列表 |
| `tools/call` | ✅ | 执行工具调用 |

### 工具实现

#### 1. `index_codebase`

索引代码库以启用语义搜索。

**参数：**
```json
{
  "path": "/absolute/path/to/codebase",
  "force": false,
  "splitter": "ast"
}
```

**实现细节：**
- 使用 `ignore` crate 遍历目录（自动跳过 `.git`、隐藏文件）
- 文件哈希检测（SHA-256），仅索引变更文件
- tree-sitter AST 解析，按函数/类切分代码块
- 批量生成 Embedding，插入 Milvus

#### 2. `search_code`

语义搜索已索引的代码库。

**参数：**
```json
{
  "path": "/absolute/path/to/codebase",
  "query": "find authentication functions",
  "limit": 10
}
```

**实现细节：**
- 将查询转换为向量（Ollama/OpenAI）
- Milvus 余弦相似度搜索
- 返回带上下文的代码片段

#### 3. `clear_index`

清除索引。

**参数：**
```json
{
  "path": "/absolute/path/to/codebase"
}
```

#### 4. `get_indexing_status`

获取索引状态。

**返回：**
```
Status: Indexed
Collection: code_index_a1b2c3d4
```

### 代码解析支持

| 语言 | 扩展名 | Parser |
|------|--------|--------|
| Rust | .rs | tree-sitter-rust |
| TypeScript | .ts, .tsx | tree-sitter-typescript |
| JavaScript | .js | tree-sitter-javascript |
| Python | .py | tree-sitter-python |
| Go | .go | tree-sitter-go |
| C++ | .cpp, .cc | tree-sitter-cpp |
| Java | .java | tree-sitter-java |
| C# | .cs | tree-sitter-c-sharp |

---

## ⚙️ 配置方式

### 环境变量

| 变量 | 必需 | 默认值 | 说明 |
|------|------|--------|------|
| `OLLAMA_HOST` | 否 | `http://127.0.0.1:11434` | Ollama 服务地址 |
| `EMBEDDING_MODEL` | 否 | `nomic-embed-text` | Embedding 模型名 |
| `MILVUS_ADDRESS` | 否 | `http://127.0.0.1:19530` | Milvus 地址 |
| `SNAPSHOT_PATH` | 否 | `~/.code-context/snapshot.json` | 快照存储路径 |
| `MAX_INDEXED_PROJECTS` | 否 | `10` | 最大索引项目数（超限时 LRU 自动驱逐） |
| `RUST_LOG` | 否 | - | 日志级别 (info/debug/error) |

### OpenCode 配置

在 `~/.config/opencode/opencode.json` 中添加：

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

> **Note**: 
> - 使用 `cargo install --path .` 安装后，二进制文件位于 `~/.cargo/bin/code-context-mcp`
> - MCP 使用 **stdio** 传输，无需显式指定 stdin/stdout
> - 服务器通过 stdin 接收 JSON-RPC 请求，通过 stdout 返回响应
> - 日志输出到 stderr，不会干扰协议通信

### 关于 stdio 传输

MCP 协议通过 **stdio**（标准输入输出）进行通信：

| 流 | 用途 | 说明 |
|----|------|------|
| `stdin` | 接收请求 | MCP 客户端发送 JSON-RPC 请求 |
| `stdout` | 返回响应 | 服务器返回 JSON-RPC 响应（仅协议数据） |
| `stderr` | 日志输出 | 调试信息、错误日志（不影响协议） |

因此配置中**不需要**添加 `stdin` 参数，MCP 客户端会自动处理。

### 其他客户端配置

#### Cursor

`~/.cursor/mcp.json`:
```json
{
  "mcpServers": {
    "code-context-mcp": {
      "command": "/home/de/works/code-context-mcp/target/release/code-context-mcp",
      "env": {
        "OLLAMA_HOST": "http://127.0.0.1:11434",
        "MILVUS_ADDRESS": "http://127.0.0.1:19530"
      }
    }
  }
}
```

#### Claude Code

```bash
claude mcp add code-context-mcp \
  -e OLLAMA_HOST=http://127.0.0.1:11434 \
  -e MILVUS_ADDRESS=http://127.0.0.1:19530 \
  -- /home/de/works/code-context-mcp/target/release/code-context-mcp
```

---

## 🚀 快速开始

### 1. 前置依赖

```bash
# Ollama (Embedding)
curl -fsSL https://ollama.ai/install.sh | sh
ollama pull nomic-embed-text
ollama serve

# Milvus (向量数据库)
docker run -d \
  -p 19530:19530 \
  --name milvus \
  milvusdb/milvus:v2.3.21
```

### 2. 构建

```bash
cd /home/de/works/code-context-mcp
cargo build --release
```

### 3. 配置

```bash
cp .env.example .env
# 编辑 .env 文件
```

### 4. 验证

```bash
# 测试运行
./target/release/code-context-mcp

# 应该看到：
# Starting Code Context MCP server...
# MCP server started, waiting for requests...
```

### 5. 使用

在 OpenCode/Claude Code 中：

```
# 索引当前项目
Index the codebase at /home/de/works/wedevs

# 搜索
Find functions that handle form submission

# 查看状态
Check indexing status
```

---

## 📊 性能基准

| 指标 | Rust 实现 | JavaScript 实现 |
|------|----------|----------------|
| 二进制大小 | 22MB | ~150MB (含 Node.js) |
| 启动时间 | <100ms | ~500ms |
| 内存占用 | ~50MB | ~150MB |
| AST 解析速度 | 快 (原生) | 中 (WASM) |
| 索引 10k 文件 | ~30s | ~60s |

---

## 🔧 开发指南

### 添加新的 Embedding Provider

```rust
// src/embedding/voyage.rs
use super::{Embedding, EmbeddingProvider};

pub struct VoyageEmbedding { ... }

#[async_trait::async_trait]
impl EmbeddingProvider for VoyageEmbedding {
    async fn embed(&self, text: &str) -> Result<Embedding> { ... }
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> { ... }
    fn dimension(&self) -> usize { ... }
}
```

### 添加新的工具

1. 在 `handlers/tool_handlers.rs` 添加处理方法
2. 在 `server.rs` 的 `handle_tools_list` 注册工具
3. 在 `handle_tools_call` 添加路由

### 调试

```bash
# 启用详细日志
RUST_LOG=debug cargo run

# 查看日志输出（stderr）
RUST_LOG=info ./target/release/code-context-mcp 2>&1 | grep DEBUG
```

---

## 📝 许可证

MIT License
