import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock fetch globally
const mockFetch = vi.fn();
vi.stubGlobal("fetch", mockFetch);

// Import after mock
import { initialize, submitTask } from "../mcp";

function mockJsonRpcResponse(result: unknown) {
  return {
    ok: true,
    headers: new Headers({ "mcp-session-id": "test-session" }),
    json: () => Promise.resolve({ jsonrpc: "2.0", id: 1, result }),
  };
}

beforeEach(() => {
  mockFetch.mockReset();
});

describe("MCP Client — initialize", () => {
  it("sends initialize request", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonRpcResponse({
      protocolVersion: "2025-11-25",
      serverInfo: { name: "alpha-swarm", version: "0.1.0" },
      capabilities: {},
    }));

    const result = await initialize();
    expect(result.protocolVersion).toBe("2025-11-25");
    expect(result.serverInfo.name).toBe("alpha-swarm");

    const call = mockFetch.mock.calls[0];
    const body = JSON.parse(call[1].body);
    expect(body.method).toBe("initialize");
  });

  it("throws on MCP error response", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      headers: new Headers(),
      json: () => Promise.resolve({
        jsonrpc: "2.0", id: 1,
        error: { code: -32601, message: "Method not found" },
      }),
    });

    await expect(initialize()).rejects.toThrow("Method not found");
  });
});

describe("MCP Client — submitTask", () => {
  it("sends tools/call for submitTask", async () => {
    mockFetch.mockResolvedValueOnce(mockJsonRpcResponse({
      content: [{ type: "text", text: "Task submitted. Run ID: agent_run:abc123" }],
    }));

    const result = await submitTask("my-project", "Add tests");
    expect(result).toContain("Task submitted");
    expect(result).toContain("abc123");

    const call = mockFetch.mock.calls[0];
    const body = JSON.parse(call[1].body);
    expect(body.method).toBe("tools/call");
    expect(body.params.name).toBe("submit_task");
    expect(body.params.arguments.project).toBe("my-project");
    expect(body.params.arguments.goal).toBe("Add tests");
  });
});
