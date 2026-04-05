# Feature: Agent-Worker WASI Component

## Scenario: Health check on GET
  When I GET /
  Then status code is 200
  And body is {"status":"ok","component":"agent-worker"}

## Scenario: Execute task via POST
  Given Ollama is running with qwen2.5-coder:7b
  When I POST {"task":"add a double function","model":"qwen2.5-coder:7b","ollama_url":"http://localhost:11434","files":[{"path":"src/main.rs","content":"fn main(){}"}]}
  Then status code is 200
  And body.status is "ok"
  And body.edits >= 0
  And body.model is "qwen2.5-coder:7b"

## Scenario: Modified files returned in response
  Given the LLM returns an EDIT block
  When the task completes
  Then body.modified_files contains the updated file content
  And the edit is applied correctly (old text replaced with new)

## Scenario: Invalid JSON returns 400
  When I POST "not json"
  Then status code is 400
  And body contains "error"

## Scenario: Ollama unreachable returns 500
  When I POST with ollama_url pointing to unreachable host
  Then status code is 500
  And body contains "inference failed"

## Scenario: Empty files list
  When I POST with files=[]
  Then the agent still runs (no file context in prompt)
  And returns a valid response
