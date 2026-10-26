import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import type { WorkspaceAssessment } from "../../lib/types";
import { MissionBrief } from "./MissionBrief";
const assessment: WorkspaceAssessment = {
  sequence: 104,
  created_at: "2026-10-23T10:00:00Z",
  summary: "Last checkpoint",
  mission_update: {
    headline: "One direction remains credible",
    established: "The baseline remains supported.",
    ruled_out: "The candidate violated the coverage guardrail.",
    uncertain: "Independent validation is pending.",
    evidence: ["F012", "E014"],
    decision: {
      question: "Keep the coverage guardrail?",
      recommendation: "Keep it and try selective verification.",
      trade_off:
        "Removing the guardrail would leave more questions unanswered.",
    },
  },
};
afterEach(cleanup);
it("sends the dated recommendation as guidance and keeps the evidence accessible", () => {
  const approve = vi.fn();
  render(
    <MemoryRouter>
      <MissionBrief
        assessment={assessment}
        slug="demo"
        canReply
        answered={false}
        sending={false}
        onApprove={approve}
        onDiscuss={vi.fn()}
      />
    </MemoryRouter>,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Use this recommendation" }),
  );
  expect(approve).toHaveBeenCalledWith(
    expect.stringContaining("Keep the coverage guardrail?"),
  );
  expect(approve).toHaveBeenCalledWith(
    expect.stringContaining("Keep it and try selective verification."),
  );
  expect(approve).toHaveBeenCalledWith(
    expect.stringContaining("I approve your recommendation:"),
  );
  expect(screen.getByRole("link", { name: "F012" }).getAttribute("href")).toBe(
    "/projects/demo/knowledge/F012",
  );
});
it("offers no approval action to viewers or while a decision reply is already sent", () => {
  const view = render(
    <MemoryRouter>
      <MissionBrief
        assessment={assessment}
        slug="demo"
        canReply={false}
        answered={false}
        sending={false}
        onApprove={vi.fn()}
        onDiscuss={vi.fn()}
      />
    </MemoryRouter>,
  );
  expect(
    screen.queryByRole("button", { name: "Use this recommendation" }),
  ).toBeNull();
  view.rerender(
    <MemoryRouter>
      <MissionBrief
        assessment={assessment}
        slug="demo"
        canReply
        answered
        sending={false}
        onApprove={vi.fn()}
        onDiscuss={vi.fn()}
      />
    </MemoryRouter>,
  );
  expect(screen.getByRole("status").textContent).toContain(
    "Waiting for Limina",
  );
  expect(
    screen.queryByRole("button", { name: "Use this recommendation" }),
  ).toBeNull();
});
