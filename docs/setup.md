# Canopy Setup Guide

Complete guide to getting Canopy running from scratch.

## Prerequisites

- Rust toolchain (`rustup`, `cargo`)
- Ollama running locally (`ollama serve`)
- Pull an embedding model: `ollama pull qwen3-embedding:4b`

## 1. Install Canopy

```bash
cargo install --git https://github.com/lioralabs/canopy
```

## 2. Initialize a Project

```bash
cd ~/dev/your-project

canopy init
# Creates .canopy/ directory with config + git hooks
# Does NOT index — gives you a chance to configure first
```

## 3. Configure `.canopy/canopy.toml`

Edit the generated `.canopy/canopy.toml`:

```toml
[project]
name = "your-project"

[indexing]
merge_threshold = 20       # merge chunks smaller than this (lines)
split_threshold = 200      # split chunks larger than this (lines)
concurrency = 8            # parallel embedding requests
# ignore = ["vendor/**", "generated/**"]

[embedding]
provider = "ollama"
model = "qwen3-embedding:4b"
url = "http://localhost:11434"
# dimensions = 2560         # auto-detected on first index

[query]
top_k = 15
graph_hops = 1
min_score = 0.3
symbol_boost = 0.15
graph_seed_top_n = 3
max_graph_entities = 10
symbol_top_k = 5
test_demotion = 0.7
# synthesis_provider = "ollama"
# synthesis_model = "qwen2.5-coder:32b"
```

### OpenAI-compatible embedding providers

Any OpenAI-compatible API works. Set `api_key_env` to the name of the environment variable holding your key:

```toml
[embedding]
provider = "openai"
model = "text-embedding-3-small"
url = "https://api.openai.com"
api_key_env = "OPENAI_API_KEY"
```

### Optional synthesis config

Add synthesis settings to `[query]` to get natural-language answers instead of raw source lists:

```toml
[query]
synthesis_provider = "ollama"
synthesis_model = "qwen2.5-coder:32b"
# synthesis_url = "http://localhost:11434"     # defaults to embedding url
# synthesis_api_key_env = "OPENAI_API_KEY"     # for cloud providers
```

## 4. Index and Query

```bash
canopy reindex                        # full index from scratch
canopy search "how does config work"  # search the codebase
```

The indexing pipeline:
1. **Parse** -- tree-sitter chunks all code files into semantic units
2. **Symbols** -- embed symbol definitions (functions, types, etc.)
3. **Chunks** -- embed code chunks
4. **Graph** -- store entities and call/define/contain edges

Progress bars show rate and ETA for each phase.

### Ongoing use

```bash
canopy index          # incremental index (only changed files since last commit)
canopy reindex        # full re-index from scratch
canopy status         # show file/chunk/symbol counts
canopy clean          # remove index + config, start fresh
```

Git hooks auto-run `canopy index` on commits to main/master.

## Troubleshooting

**Ollama not responding:**
```bash
ollama list                    # check if running
curl http://localhost:11434    # check connectivity
```

**Embedding dimension mismatch after changing models:**
- Changing embedding models invalidates existing vectors
- Run `canopy clean && canopy init && canopy reindex` to start fresh

**Slow indexing:**
- Check which phase is slow (progress bars show rate)
- Bump `concurrency` in `.canopy/canopy.toml` for cloud providers
