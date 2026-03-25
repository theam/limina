import { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

function getAgentServiceUrl(): string {
  return process.env.AGENT_SERVICE_URL || "http://127.0.0.1:3001";
}

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { message, sessionId } = body;

  if (!message || typeof message !== "string") {
    return new Response(JSON.stringify({ error: "message is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Proxy to agent service — it runs the SDK in its own process
  const res = await fetch(`${getAgentServiceUrl()}/chat`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ message, sessionId }),
  });

  if (!res.ok || !res.body) {
    return new Response(
      JSON.stringify({ error: "Agent service unavailable" }),
      { status: 502, headers: { "Content-Type": "application/json" } }
    );
  }

  // Pipe the NDJSON stream from agent service to the browser
  return new Response(res.body, {
    headers: {
      "Content-Type": "application/x-ndjson",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    },
  });
}
