// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { Summary } from "./Summary";
afterEach(cleanup);
it("provides an explicit disclosure and preserves all long Markdown content", () => {
  const end = "The complete next action includes a final rollback decision.";
  const source = `${"A detailed objective with evidence and constraints. ".repeat(30)}\n\n${end}`;
  const { container } = render(<MemoryRouter><Summary label="Next step" source={source} slug="demo" /></MemoryRouter>);
  const disclosure = container.querySelector("details")!;
  expect(disclosure.open).toBe(false);
  expect(screen.getByText("Read full next step")).toBeTruthy();
  fireEvent.click(disclosure.querySelector("summary")!);
  expect(disclosure.open).toBe(true);
  expect(screen.getByText(end)).toBeTruthy();
  expect(container.querySelector(".clamp-4")).toBeNull();
});
