#!/bin/bash
# Code Context MCP - Installation Script

set -e

echo "🦀 Code Context MCP - Installation"
echo "==================================="
echo ""

# Check prerequisites
echo "📋 Checking prerequisites..."

# Check Rust
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust/Cargo not found. Please install Rust first:"
    echo "   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi
echo "✅ Rust: $(rustc --version)"

# Check Ollama
if ! command -v ollama &> /dev/null; then
    echo "⚠️  Ollama not found. Install with:"
    echo "   curl -fsSL https://ollama.ai/install.sh | sh"
else
    echo "✅ Ollama: $(ollama --version 2>&1 | head -1)"
fi

# Check Milvus (Docker)
if command -v docker &> /dev/null && docker ps &> /dev/null; then
    if docker ps | grep -q milvus; then
        echo "✅ Milvus: Running in Docker"
    else
        echo "⚠️  Milvus not running. Start with:"
        echo "   docker run -d -p 19530:19530 milvusdb/milvus:v2.3.21"
    fi
else
    echo "⚠️  Docker not running or not installed"
fi

echo ""
echo "📦 Installing code-context-mcp..."
cargo install --path .

echo ""
echo "✅ Installation complete!"
echo ""
echo "📍 Binary location: ~/.cargo/bin/code-context-mcp"
echo ""
echo "🔧 Next steps:"
echo "   1. Copy .env.example to .env and configure"
echo "   2. Start Ollama: ollama serve"
echo "   3. Start Milvus: docker run -d -p 19530:19530 milvusdb/milvus:v2.3.21"
echo "   4. Add to your MCP client configuration (see README.md)"
echo ""
echo "🚀 Run: code-context-mcp"
echo ""
