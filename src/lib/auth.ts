import { NextRequest, NextResponse } from "next/server";

/**
 * Auth middleware. Only enforced when MISSION_API_KEY is set in .env.
 * When unset, all requests are allowed (local development mode).
 */
export function requireAuth(
  request: NextRequest
): NextResponse | null {
  const apiKey = process.env.MISSION_API_KEY;

  // No API key configured — allow all requests (local mode)
  if (!apiKey) {
    return null;
  }

  const authHeader = request.headers.get("authorization");
  const providedKey = authHeader?.replace("Bearer ", "");

  if (providedKey !== apiKey) {
    return NextResponse.json(
      { error: "Unauthorized" },
      { status: 401 }
    );
  }

  return null;
}
