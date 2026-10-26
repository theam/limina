import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useState } from "react";
import type { Engine, Project } from "../../lib/types";
import { ModelFields, type ModelChoice } from "./ModelFields";
import { KickoffDialog } from "./Kickoff";

const mocks = vi.hoisted(() => ({ update: vi.fn(), start: vi.fn(), refetch: vi.fn(), failed: false, pending: false, efforts: ["low", "ultra"] }));
vi.mock("../../lib/api", () => ({ api: { updateProject: mocks.update } }));
vi.mock("./useLifecycle", () => ({ useLifecycle: () => ({ mutateAsync: mocks.start }) }));
vi.mock("../../lib/queries", () => ({ useModelCatalog: (engine: Engine) => ({ isPending: mocks.pending, isFetching: mocks.pending, isError: mocks.failed, refetch: mocks.refetch, data: mocks.pending ? undefined : {
  default_model: engine === "codex" ? "test-codex" : "sonnet",
  models: engine === "codex" ? [
    { id: "test-codex", label: "Test Codex", reasoning_efforts: mocks.efforts, default_reasoning_effort: "low" },
    { id: "small", label: "Small", reasoning_efforts: ["low"], default_reasoning_effort: "low" },
  ] : [{ id: "sonnet", label: "Claude Sonnet", reasoning_efforts: ["low", "medium", "high", "xhigh", "max"], default_reasoning_effort: null }],
} }) }));

afterEach(() => { cleanup(); vi.clearAllMocks(); mocks.failed = false; mocks.pending = false; mocks.efforts = ["low", "ultra"]; });

function Picker({ engine = "codex" }: { engine?: Engine }) {
  const [value, onChange] = useState<ModelChoice>({ model: "", reasoning_effort: "" });
  return <><ModelFields engine={engine} value={value} onChange={onChange} /><output>{JSON.stringify(value)}</output></>;
}

it("uses provider model capabilities and resets incompatible reasoning when the model changes", () => {
  render(<Picker />);
  fireEvent.change(screen.getByLabelText("Default model"), { target: { value: "test-codex" } });
  fireEvent.change(screen.getByLabelText("Reasoning mode"), { target: { value: "ultra" } });
  expect(screen.getByRole("status").textContent).toContain('"reasoning_effort":"ultra"');
  fireEvent.change(screen.getByLabelText("Default model"), { target: { value: "small" } });
  expect((screen.getByLabelText("Reasoning mode") as HTMLSelectElement).value).toBe("");
  expect(screen.queryByRole("option", { name: "Ultra" })).toBeNull();
});

it("offers Claude effort and explicit model IDs", () => {
  render(<Picker engine="claude-code" />);
  expect(screen.getByRole("option", { name: "Max" })).toBeTruthy();
  expect(screen.queryByRole("option", { name: "Ultra" })).toBeNull();
  fireEvent.change(screen.getByLabelText("Default model"), { target: { value: "__custom" } });
  fireEvent.change(screen.getByLabelText("Model ID"), { target: { value: "claude-pinned-version" } });
  expect(screen.getByRole("status").textContent).toContain('"model":"claude-pinned-version"');
  expect((screen.getByLabelText("Reasoning mode") as HTMLSelectElement).disabled).toBe(true);
  expect(screen.queryByRole("option", { name: "Max" })).toBeNull();
});

const project = { slug: "demo", name: "Demo", mission: "Mission", runtime: "codex", status: "CREATED", role: "OWNER", model: "test-codex", reasoning_effort: "low" } as Project;
function kickoff() {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { mutations: { retry: false } } })}>
    <KickoffDialog project={project} open onOpenChange={() => {}} />
  </QueryClientProvider>);
}

it("saves kickoff choices before starting the first turn", async () => {
  mocks.update.mockResolvedValue({}); mocks.start.mockResolvedValue({});
  kickoff();
  fireEvent.change(screen.getByLabelText("Reasoning mode"), { target: { value: "ultra" } });
  fireEvent.click(screen.getByRole("button", { name: "Start project" }));
  await waitFor(() => expect(mocks.start).toHaveBeenCalledWith("start"));
  expect(mocks.update).toHaveBeenCalledWith("demo", { model: "test-codex", reasoning_effort: "ultra" });
  expect(mocks.update.mock.invocationCallOrder[0]).toBeLessThan(mocks.start.mock.invocationCallOrder[0]);
});

it("does not start when saving the chosen defaults fails", async () => {
  mocks.update.mockRejectedValue(new Error("Save failed"));
  kickoff();
  fireEvent.change(screen.getByLabelText("Reasoning mode"), { target: { value: "ultra" } });
  fireEvent.click(screen.getByRole("button", { name: "Start project" }));
  await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Save failed"));
  expect(mocks.start).not.toHaveBeenCalled();
});

it("refreshes capabilities and marks an old reasoning choice without inventing alternatives", () => {
  const view = render(<Picker />);
  fireEvent.change(screen.getByLabelText("Reasoning mode"), { target: { value: "ultra" } });
  mocks.efforts = ["low"];
  view.rerender(<Picker />);
  const saved = screen.getByRole("option", { name: "ultra (not advertised)" }) as HTMLOptionElement;
  expect(saved.disabled).toBe(true);
  expect(screen.getByLabelText("Reasoning mode").getAttribute("aria-invalid")).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: "Refresh models" }));
  expect(mocks.refetch).toHaveBeenCalledOnce();
});

it("reports discovery failure and allows retry", () => {
  mocks.failed = true;
  render(<Picker />);
  expect(screen.getByText(/Couldn’t refresh provider models/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Refresh models" }));
  expect(mocks.refetch).toHaveBeenCalledOnce();
});

it("keeps saved choices while loading without labelling them unsupported", () => {
  mocks.pending = true;
  render(<ModelFields engine="claude-code" value={{ model: "saved-model", reasoning_effort: "high" }} onChange={() => {}} />);
  expect(screen.getByRole("option", { name: "saved-model (saved)" })).toBeTruthy();
  expect(screen.getByRole("option", { name: "high (saved)" })).toBeTruthy();
  expect(screen.queryByText(/no longer advertised/)).toBeNull();
  expect(screen.getByLabelText("Reasoning mode").getAttribute("aria-invalid")).toBeNull();
});
