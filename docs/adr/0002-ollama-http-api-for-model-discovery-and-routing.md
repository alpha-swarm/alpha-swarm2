# ADR-0002: Use Ollama HTTP API for Model Discovery and Routing

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

alpha-swarm agents need to discover which LLM models are available locally, query their capabilities (context window, parameter count, specialization), and route prompts to the appropriate model based on task complexity.

The system must work fully offline with local models, while optionally falling back to cloud APIs (Claude) for complex orchestration tasks.

## Decision Drivers

- **Local-first**: System must function without internet access
- **Model-aware routing**: Simple tasks (fmt fix, rename) should use small/fast models; complex tasks should use larger ones
- **Dynamic discovery**: Models may be pulled/removed at any time — the system must adapt
- **Multi-machine**: Different machines may have different models loaded — routing should account for this
- **No subprocess spawning from WASI**: WasmCloud components cannot shell out to `ollama` CLI

## Considered Alternatives

### Ollama CLI (`ollama list`, `ollama show`)
- Simple, well-documented
- Cannot be called from WASI components (no subprocess spawning)
- Output parsing is fragile compared to structured API responses
- No streaming support

### Direct llama.cpp Integration
- Maximum performance, no Ollama dependency
- Massive integration effort — model loading, tokenization, inference all manual
- Loses Ollama's model management (pull, quantization selection, etc.)
- Single-model per process — no multi-model routing

### vLLM / Text Generation Inference
- Production-grade serving with batching
- Heavy dependencies (Python, CUDA toolkit)
- Overkill for local single-user agent workloads
- Poor fit for heterogeneous hardware (laptops + servers)

### Ollama HTTP API (chosen)
- Structured JSON responses from localhost:11434
- `/api/tags` — list all models with metadata (size, family, quantization)
- `/api/show` — detailed model info (context window, parameters, architecture)
- `/api/generate` and `/api/chat` — inference with configurable context window
- `/api/ps` — currently loaded models (memory allocation, GPU usage)
- Callable from any HTTP client — works from WASI components via HTTP capability

## Decision Outcome

**Use Ollama's HTTP API** (localhost:11434) for all model interactions.

Build an `ollama-client` Rust crate that wraps the HTTP API with typed responses. In WasmCloud, this becomes part of an Ollama capability provider that agents import.

### Model Routing Strategy

| Task Complexity | Model Selection | Example |
|---|---|---|
| Simple (single-line fix) | Smallest available code model (qwen2.5-coder:7b) | Fix lint warning, rename variable |
| Medium (function-level change) | Mid-size code model (deepseek-coder-v2:16b) | Add error handling, write a test |
| Complex (multi-file architecture) | Largest available or Claude API fallback | Refactor module, design new component |

Routing logic:
1. Query `/api/tags` to get available models
2. Classify each model by: parameter count, code specialization, context window
3. Match task complexity (estimated by orchestrator) to model tier
4. If no suitable local model exists, escalate to Claude API (opt-in)

## Consequences

### Positive
- Works fully offline — Ollama runs locally with no cloud dependency
- Structured API responses — no CLI output parsing
- Dynamic model discovery — adapts as models are pulled/removed
- HTTP-based — works from WASI components via standard wasi:http
- Multi-machine aware — each host's Ollama provider reports its local models

### Negative
- Ollama must be running as a separate service on each machine
- HTTP overhead vs. direct llama.cpp (negligible for inference-bound workloads)
- Model metadata from Ollama may not include all properties needed for routing (context window requires `/api/show` per model)

### Risks
- Ollama API stability — not semver-guaranteed, though historically stable
- Model routing heuristics may need tuning per hardware configuration
