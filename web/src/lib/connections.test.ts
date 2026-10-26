import { afterEach, describe, expect, it, vi } from "vitest";
import {
  authLabel,
  canDisconnect,
  connectErrorMessage,
  connectLabel,
  connectionStatus,
  connectionsApi,
  headerNameProblem,
  inactiveReasonLabel,
  projectConnectionState,
  projectsPhrase,
  readConnectReturn,
  safeAuthorizationUrl,
  serverUrlProblem,
} from "./connections";

describe("connection status", () => {
  it("maps each status to a label and tone", () => {
    expect(connectionStatus("CONNECTED")).toEqual({ label: "Connected", tone: "success" });
    expect(connectionStatus("NEEDS_AUTHORIZATION")).toEqual({ label: "Needs authorization", tone: "warning" });
    expect(connectionStatus("ERROR")).toEqual({ label: "Error", tone: "critical" });
    expect(connectionStatus("NOT_CONNECTED")).toEqual({ label: "Not connected", tone: "neutral" });
  });

  it("offers Connect the first time and Reconnect afterwards", () => {
    expect(connectLabel({ status: "NOT_CONNECTED", credential_configured: false, connected_at: null })).toBe("Connect");
    expect(connectLabel({ status: "NOT_CONNECTED", credential_configured: true, connected_at: null })).toBe("Reconnect");
    expect(connectLabel({ status: "ERROR", credential_configured: false, connected_at: null })).toBe("Reconnect");
    expect(connectLabel({ status: "CONNECTED", credential_configured: true, connected_at: "2026-01-01T00:00:00Z" })).toBe(
      "Reconnect",
    );
  });

  it("only offers Disconnect when something is connected or stored", () => {
    expect(canDisconnect({ status: "NOT_CONNECTED", credential_configured: false })).toBe(false);
    expect(canDisconnect({ status: "NOT_CONNECTED", credential_configured: true })).toBe(true);
    expect(canDisconnect({ status: "CONNECTED", credential_configured: false })).toBe(true);
  });

  it("describes how a connection authorizes", () => {
    expect(authLabel("oauth")).toBe("Sign-in at the provider");
    expect(authLabel("none")).toBe("No sign-in");
    expect(authLabel("bearer")).toBe("Bearer token");
    expect(authLabel("header", "X-API-Key")).toBe("Header X-API-Key");
  });
});

describe("project connections", () => {
  it("explains why an enabled connection is inactive", () => {
    expect(inactiveReasonLabel("not_connected")).toBe("Not connected — its owner must reconnect it");
    expect(inactiveReasonLabel("owner_left")).toBe("Its owner is no longer a member");
    expect(inactiveReasonLabel(null)).toBeNull();
    expect(inactiveReasonLabel("not_connected", true)).toMatch(/reconnect it/);
  });

  it("summarizes whether the next turn uses it", () => {
    expect(projectConnectionState({ active: true, inactive_reason: null, owned_by_you: false, status: "CONNECTED" })).toEqual({
      label: "Active",
      tone: "success",
      detail: null,
    });
    expect(
      projectConnectionState({ active: false, inactive_reason: "owner_left", owned_by_you: false, status: "CONNECTED" }),
    ).toEqual({ label: "Inactive", tone: "warning", detail: "Its owner is no longer a member" });
    expect(projectConnectionState({ active: false, inactive_reason: null, owned_by_you: true, status: "ERROR" }).detail).toBe(
      "Error",
    );
  });

  it("names the projects a change affects", () => {
    expect(projectsPhrase([{ name: "Alpha" }])).toBe("Alpha");
    expect(projectsPhrase([{ name: "Alpha" }, { name: "Beta" }])).toBe("Alpha and Beta");
    expect(projectsPhrase([{ name: "A" }, { name: "B" }, { name: "C" }])).toBe("3 projects");
  });
});

describe("provider sign-in", () => {
  it("reads the outcome the runtime redirects back with", () => {
    expect(readConnectReturn(new URLSearchParams("connected=c1"))).toEqual({ kind: "connected", id: "c1" });
    expect(readConnectReturn(new URLSearchParams("connection=c1&connection_error=denied"))).toEqual({
      kind: "error",
      id: "c1",
      code: "denied",
    });
    expect(readConnectReturn(new URLSearchParams("tab=all"))).toBeNull();
  });

  it("explains each error code", () => {
    expect(connectErrorMessage("denied")).toMatch(/Permission wasn’t granted/);
    expect(connectErrorMessage("expired")).toMatch(/took too long/);
    expect(connectErrorMessage("browser")).toMatch(/same browser/);
    expect(connectErrorMessage("failed")).toMatch(/refused/);
    expect(connectErrorMessage("failed", "invalid_scope: notes.write")).toBe("invalid_scope: notes.write");
    expect(connectErrorMessage("something_new")).toMatch(/didn’t complete/);
  });

  it("follows only web authorization URLs", () => {
    expect(safeAuthorizationUrl("https://auth.example.com/authorize?x=1")).toBe("https://auth.example.com/authorize?x=1");
    expect(safeAuthorizationUrl("javascript:alert(1)")).toBeNull();
    expect(safeAuthorizationUrl("data:text/html,hi")).toBeNull();
    expect(safeAuthorizationUrl(null)).toBeNull();
  });
});

describe("form checks", () => {
  it("accepts https, and plain http only for loopback", () => {
    expect(serverUrlProblem("")).toBeNull();
    expect(serverUrlProblem("https://mcp.example.com/mcp")).toBeNull();
    expect(serverUrlProblem("http://localhost:8080/mcp")).toBeNull();
    expect(serverUrlProblem("http://127.0.0.1:8080/mcp")).toBeNull();
    expect(serverUrlProblem("http://[::1]:8080/mcp")).toBeNull();
    expect(serverUrlProblem("http://mcp.example.com/mcp")).toMatch(/https/);
    expect(serverUrlProblem("mcp.example.com")).toMatch(/full server URL/);
    expect(serverUrlProblem("https://user:pass@mcp.example.com/mcp")).toMatch(/credentials/);
    expect(serverUrlProblem("ftp://mcp.example.com")).toMatch(/https/);
  });

  it("checks header names", () => {
    expect(headerNameProblem("X-API-Key")).toBeNull();
    expect(headerNameProblem("")).toBeNull();
    expect(headerNameProblem("X API Key")).toMatch(/without spaces/);
    expect(headerNameProblem("X-Key:")).not.toBeNull();
  });
});

describe("connectionsApi", () => {
  afterEach(() => vi.unstubAllGlobals());

  const stubFetch = (body: unknown, status = 200) => {
    const fetch = vi.fn(async () => new Response(JSON.stringify(body), { status }));
    vi.stubGlobal("fetch", fetch);
    return fetch;
  };
  const call = (fetch: ReturnType<typeof stubFetch>) => {
    const [path, init] = fetch.mock.calls[0] as unknown as [string, RequestInit & { headers: Record<string, string> }];
    return { path, init, body: init.body ? JSON.parse(String(init.body)) : undefined };
  };

  it("creates a connection as an idempotent command", async () => {
    const fetch = stubFetch({ id: "c1" }, 201);
    await connectionsApi.create({ name: "Notes", server_url: "https://mcp.example.com/mcp", auth: "bearer", secret: "s" });
    const { path, init, body } = call(fetch);
    expect(path).toBe("/v1/connections");
    expect(init.method).toBe("POST");
    expect(init.headers["Idempotency-Key"]).toBeTruthy();
    expect(body).toEqual({ name: "Notes", server_url: "https://mcp.example.com/mcp", auth: "bearer", secret: "s" });
  });

  it("asks to come back to the given console path after connecting", async () => {
    const fetch = stubFetch({ connection: { id: "c 1" }, authorization_url: null });
    await connectionsApi.connect("c 1", "/connections");
    const { path, init, body } = call(fetch);
    expect(path).toBe("/v1/connections/c%201/connect");
    expect(init.method).toBe("POST");
    expect(body).toEqual({ return_to: "/connections" });
  });

  it("enables and disables a connection in a project", async () => {
    let fetch = stubFetch({});
    await connectionsApi.enable("my-project", "c1");
    expect(call(fetch).path).toBe("/v1/projects/my-project/connections/c1");
    expect(call(fetch).init.method).toBe("PUT");
    expect(call(fetch).init.body).toBeUndefined();

    fetch = stubFetch({});
    await connectionsApi.disable("my-project", "c1");
    expect(call(fetch).init.method).toBe("DELETE");
  });

  it("surfaces the runtime's error message", async () => {
    stubFetch({ error: { code: "unreachable", message: "The MCP server didn’t answer.", details: {} } }, 502);
    await expect(connectionsApi.connect("c1")).rejects.toMatchObject({
      status: 502,
      code: "unreachable",
      message: "The MCP server didn’t answer.",
    });
  });
});
