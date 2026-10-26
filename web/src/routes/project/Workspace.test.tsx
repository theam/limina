import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { useState } from "react";
import { Workspace } from "./Workspace";

vi.mock("./ProjectContext", () => ({
  useProjectContext: () => ({ slug: "demo" }),
}));
vi.mock("../../lib/queries", () => ({
  useProjectStatus: () => ({
    data: { project: { name: "A research mission" } },
  }),
}));
vi.mock("./Conversation", () => ({
  Conversation: function Draft() {
    const [body, setBody] = useState("");
    return (
      <textarea
        aria-label="Message to Limina"
        value={body}
        onChange={(event) => setBody(event.target.value)}
      />
    );
  },
}));
beforeEach(() => vi.stubGlobal("matchMedia", () => ({ matches: false })));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
it("preserves the conversation draft while opening, closing and switching adjacent evidence", async () => {
  const router = createMemoryRouter(
    [
      {
        path: "/projects/:slug",
        element: <Workspace />,
        children: [
          { index: true, element: <p>Current state</p> },
          {
            path: "knowledge",
            element: <a href="/projects/demo/knowledge/F001">Search results</a>,
          },
          { path: "knowledge/:key", element: <p>Complete evidence note</p> },
          { path: "results", element: <p>Published files</p> },
        ],
      },
    ],
    { initialEntries: ["/projects/demo"] },
  );
  render(<RouterProvider router={router} />);
  fireEvent.change(screen.getByLabelText("Message to Limina"), {
    target: { value: "Keep my question while I check the evidence." },
  });
  fireEvent.click(screen.getByRole("link", { name: "Results" }));
  await screen.findByText("Published files");
  expect(document.title).toBe("Results · Limina");
  expect(
    (screen.getByLabelText("Message to Limina") as HTMLTextAreaElement).value,
  ).toContain("Keep my question");
  await act(() => router.navigate("/projects/demo/knowledge/F001"));
  await screen.findByText("Complete evidence note");
  expect(document.title).toBe("F001 · Limina");
  fireEvent.click(
    screen.getByRole("button", { name: "Close project context" }),
  );
  expect(
    screen
      .getByRole("button", { name: "Show context" })
      .getAttribute("aria-expanded"),
  ).toBe("false");
  expect(
    (screen.getByLabelText("Message to Limina") as HTMLTextAreaElement).value,
  ).toContain("Keep my question");
  fireEvent.click(screen.getByRole("button", { name: "Show context" }));
  fireEvent.click(screen.getByRole("button", { name: "Close F001" }));
  await screen.findByText("Search results");
  expect(document.title).toBe("Knowledge · Limina");
  expect(
    (screen.getByLabelText("Message to Limina") as HTMLTextAreaElement).value,
  ).toContain("Keep my question");
});
