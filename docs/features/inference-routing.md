# Feature: Inference Routing by Complexity

## Scenario: Simple task uses smallest code model
  Given Ollama has models: qwen2.5-coder:7b, deepseek-coder:33b, codellama:34b
  When I request a model for complexity "simple"
  Then it selects qwen2.5-coder:7b (≤10B, code-specialized)

## Scenario: Medium task uses mid-size model
  Given Ollama has models: qwen2.5-coder:7b, deepseek-coder:33b
  When I request a model for complexity "medium"
  Then it selects deepseek-coder:33b (10-35B range)

## Scenario: Complex task prefers Claude
  Given Claude backend is configured
  And Ollama has models available
  When I request a model for complexity "complex"
  Then it selects a Claude Sonnet model

## Scenario: Complex task falls back to largest Ollama if no Claude
  Given no Claude backend configured
  And Ollama has models: qwen2.5-coder:7b, codellama:34b
  When I request a model for complexity "complex"
  Then it selects codellama:34b (largest available)

## Scenario: Fallback when preferred backend fails
  Given Claude backend returns an error
  And Ollama has qwen2.5-coder:7b
  When I send a chat with complexity "complex"
  Then it falls back to Ollama
  And the response backend is Ollama

## Scenario: Model escalation for retry
  Given the model "qwen2.5-coder:7b" (7B) failed
  And Ollama has deepseek-coder:33b (33B)
  When I request escalation
  Then it returns deepseek-coder:33b

## Scenario: Escalation to Claude when no larger Ollama
  Given only qwen2.5-coder:7b on Ollama (failed)
  And Claude is configured
  When I request escalation
  Then it returns a Claude model

## Scenario: No models available
  Given no backends are configured
  When I request a model for any complexity
  Then it returns an error "No models available"

## Scenario: Code models preferred over general models
  Given Ollama has: llama3:8b (general), qwen2.5-coder:7b (code)
  When I request a model for complexity "simple"
  Then it selects qwen2.5-coder:7b (code-specialized)
