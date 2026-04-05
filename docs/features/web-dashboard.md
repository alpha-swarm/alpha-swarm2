# Feature: Web Dashboard

## Scenario: Dashboard serves HTML at /
  When I GET /
  Then status code is 200
  And Content-Type is text/html
  And body contains <title>alpha-swarm</title>
  And body contains EventSource for /api/events

## Scenario: Health endpoint returns ok
  When I GET /api/health
  Then status code is 200
  And body is {"status":"ok"}

## Scenario: Models endpoint proxies to Ollama
  Given Ollama is running at localhost:11434 with models
  When I GET /api/models
  Then status code is 200
  And body is a JSON array of model objects

## Scenario: Models endpoint returns 502 when Ollama down
  Given Ollama is not running
  When I GET /api/models
  Then status code is 502
  And body contains "ollama" error message

## Scenario: SSE events endpoint
  When I GET /api/events
  Then Content-Type is text/event-stream
  And Cache-Control is no-cache
  And body contains "event: status\ndata:"

## Scenario: Unknown routes return 404
  When I GET /nonexistent
  Then status code is 404
  And body is JSON with "error":"not found"

## Scenario: Dashboard displays active agents via SSE
  Given SSE emits agent_started event
  Then the dashboard shows an agent card with model and task
  And the active agent count increases

## Scenario: Dashboard auto-reconnects SSE
  Given the SSE connection drops
  Then EventSource automatically reconnects
  And the status indicator changes to "disconnected" then "connected"
