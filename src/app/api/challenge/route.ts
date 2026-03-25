import { NextResponse } from "next/server";

function getAgentServiceUrl(): string {
  return process.env.AGENT_SERVICE_URL || "http://127.0.0.1:3001";
}

export async function POST() {
  try {
    const res = await fetch(`${getAgentServiceUrl()}/challenge`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });

    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      return NextResponse.json(
        { error: body.error || "Agent service error" },
        { status: res.status }
      );
    }

    return NextResponse.json({ success: true, message: "Challenge review triggered" });
  } catch {
    return NextResponse.json(
      { error: "Agent service unavailable" },
      { status: 502 }
    );
  }
}
