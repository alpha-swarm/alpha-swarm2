/**
 * MCP Client — typed wrapper around JSON-RPC calls to the alpha-swarm MCP server.
 *
 * All dashboard data flows through this single client.
 * Transport: HTTP POST to /mcp endpoint (MCP Streamable HTTP).
 */

const DEFAULT_MCP_URL = "http://localhost:8090/mcp";

// --- JSON-RPC types ---

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

interface JsonRpcResponse<T = unknown> {
  jsonrpc: "2.0";
  id: number;
  result?: T;
  error?: { code: number; message: string };
}

// --- MCP response types ---

interface McpToolResult {
  content: Array<{ type: string; text: string }>;
}

interface McpResourceResult {
  contents: Array<{ uri: string; mimeType: string; text: string }>;
}

// --- Client ---

let requestId = 0;
let mcpUrl = DEFAULT_MCP_URL;
let sessionId: string | null = null;

export function configure(url: string) {
  mcpUrl = url;
}

async function rpc<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  const req: JsonRpcRequest = {
    jsonrpc: "2.0",
    id: ++requestId,
    method,
    params,
  };

  const resp = await fetch(mcpUrl, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(sessionId ? { "mcp-session-id": sessionId } : {}),
    },
    body: JSON.stringify(req),
  });

  // Capture session ID from response
  const sid = resp.headers.get("mcp-session-id");
  if (sid) sessionId = sid;

  const json: JsonRpcResponse<T> = await resp.json();

  if (json.error) {
    throw new Error(`MCP error ${json.error.code}: ${json.error.message}`);
  }

  return json.result as T;
}

// --- Initialize ---

export async function initialize() {
  return rpc<{
    protocolVersion: string;
    serverInfo: { name: string; version: string };
    capabilities: Record<string, unknown>;
  }>("initialize", {
    protocolVersion: "2025-11-25",
    clientInfo: { name: "alpha-swarm-dashboard", version: "0.1.0" },
  });
}

// --- Tools (actions) ---

function extractText(result: McpToolResult): string {
  return result.content.map((c) => c.text).join("\n");
}

export async function submitTask(project: string, goal: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "submit_task",
    arguments: { project, goal },
  });
  return extractText(result);
}

export async function createPlan(project: string, goal: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "create_plan",
    arguments: { project, goal },
  });
  return extractText(result);
}

export async function approvePlan(runId: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "approve_plan",
    arguments: { run_id: runId },
  });
  return extractText(result);
}

export async function planFeedback(runId: string, feedback: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "plan_feedback",
    arguments: { run_id: runId, feedback },
  });
  return extractText(result);
}

export async function getRunStatus(runId: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "get_run_status",
    arguments: { run_id: runId },
  });
  return extractText(result);
}

export async function getPlans(runId: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "get_plans",
    arguments: { run_id: runId },
  });
  return extractText(result);
}

export async function editPlan(runId: string, subTasks: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "edit_plan",
    arguments: { run_id: runId, sub_tasks: subTasks },
  });
  return extractText(result);
}

export async function createProject(name: string, repoUrl: string, description?: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "create_project",
    arguments: { name, repo_url: repoUrl, ...(description ? { description } : {}) },
  });
  return extractText(result);
}

export async function deleteProject(name: string): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "delete_project",
    arguments: { name },
  });
  return extractText(result);
}

export async function findSimilarRuns(project: string, query: string, limit?: number): Promise<string> {
  const result = await rpc<McpToolResult>("tools/call", {
    name: "find_similar_runs",
    arguments: { project, query, ...(limit ? { limit } : {}) },
  });
  return extractText(result);
}

// --- Resources (read-only data) ---

function parseResourceJson<T>(result: McpResourceResult): T {
  const text = result.contents[0]?.text ?? "[]";
  return JSON.parse(text) as T;
}

export async function readResource<T = unknown>(uri: string): Promise<T> {
  const result = await rpc<McpResourceResult>("resources/read", { uri });
  return parseResourceJson<T>(result);
}

// Typed resource helpers
export const resources = {
  projects: () => readResource("swarm://projects"),
  models: () => readResource("swarm://models"),
  systemResources: () => readResource("swarm://resources"),
  health: () => readResource("swarm://health"),
  live: () => readResource("swarm://live"),
  dashboard: () => readResource("swarm://dashboard"),

  projectRuns: (project: string) => readResource(`swarm://projects/${project}/runs`),
  projectGoals: (project: string) => readResource(`swarm://projects/${project}/goals`),
  projectMetrics: (project: string) => readResource(`swarm://projects/${project}/metrics`),

  runDetail: (id: string) => readResource(`swarm://runs/${id}`),
  subRuns: (id: string) => readResource(`swarm://runs/${id}/sub-runs`),
  plans: (id: string) => readResource(`swarm://runs/${id}/plans`),
  timeline: (id: string) => readResource(`swarm://runs/${id}/timeline`),
};
