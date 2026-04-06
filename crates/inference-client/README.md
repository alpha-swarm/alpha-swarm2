# inference-client

Multi-backend LLM inference router. Selects model by complexity tier, supports Ollama and Claude, with native tool calling.

## Backends

- **OllamaBackend** — HTTP client for Ollama API (`/api/chat`, `/api/embed`, `/api/tags`)
- **ClaudeBackend** — Anthropic Messages API
- **MockBackend** — For testing

## Model Routing

```
Complexity::Simple  → largest deepseek-coder/qwen model
Complexity::Medium  → largest deepseek-coder/qwen model  
Complexity::Complex → Claude Sonnet (if API key), else largest Ollama
```

Preferred models: deepseek-coder, qwen2.5-coder (over codellama which doesn't follow structured output).

## Native Tool Calling

`OllamaBackend::chat_with_tools()` passes tools as JSON schema to Ollama's `tools` API parameter. Models that support it (qwen2.5 family) return structured `tool_calls`. Models that don't get an error, triggering fallback to text-based `<<<EDIT>>>` format.

## Timeouts

- Inference: 10 minutes
- Embeddings: 2 minutes  
- Metadata (tags, ps): 30 seconds
- Connect: 10 seconds
