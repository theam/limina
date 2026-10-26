import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { Markdown } from "./Markdown";

afterEach(cleanup);

function show(source: string, publishedOutput = false) {
  render(<MemoryRouter><Markdown source={source} slug="demo" publishedOutput={publishedOutput} /></MemoryRouter>);
}

it("opens file-style research and state references in the project", () => {
  show("[Evidence](../kb/research/findings/F028-state.md#limits) [State](../kb/ACTIVE.md)");
  expect(screen.getByRole("link", { name: "Evidence" }).getAttribute("href")).toBe("/projects/demo/knowledge/F028#limits");
  expect(screen.getByRole("link", { name: "State" }).getAttribute("href")).toBe("/projects/demo/knowledge/ACTIVE");
  expect(screen.getByRole("link", { name: "Evidence" }).getAttribute("target")).toBeNull();
});

it("opens published files and peer report links in result tabs", () => {
  show("[Report](outputs/report.md) [Provenance](validacion-local-procedencia.md#sources) [Export](outputs/F028-state.md)", true);
  expect(screen.getByRole("link", { name: "Report" }).getAttribute("href")).toBe("/projects/demo/results?file=report.md");
  expect(screen.getByRole("link", { name: "Provenance" }).getAttribute("href")).toBe("/projects/demo/results?file=validacion-local-procedencia.md#sources");
  expect(screen.getByRole("link", { name: "Export" }).getAttribute("href")).toBe("/projects/demo/results?file=F028-state.md");
});

it("does not turn private workspace paths into broken public links", () => {
  show("[Raw data](../kb/research/data/E019/manifest.json)", true);
  expect(screen.queryByRole("link", { name: "Raw data" })).toBeNull();
  expect(screen.getByText(/workspace file; ask Limina to publish it/)).toBeTruthy();
});

it("keeps external URLs external even when their file name resembles a note", () => {
  show("[Source](https://example.com/F028-source.md)");
  expect(screen.getByRole("link", { name: "Source" }).getAttribute("href")).toBe("https://example.com/F028-source.md");
  expect(screen.getByRole("link", { name: "Source" }).getAttribute("target")).toBe("_blank");
});

it("handles malformed encoding and unsafe URLs without an active link", () => {
  show("[Malformed](../kb/F%XX.md) [Unsafe](javascript:alert%281%29)");
  expect(screen.queryByRole("link", { name: "Malformed" })).toBeNull();
  expect(screen.queryByRole("link", { name: "Unsafe" })).toBeNull();
});
