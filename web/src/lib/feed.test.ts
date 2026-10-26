import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  api: {
    events: vi.fn(async () => ({ events: [{ sequence: 1, type: "project.created", actor: "a", artifact_id: null, detail: {}, created_at: "" }], cursor: 1, has_earlier: false })),
    liveTicket: vi.fn(async () => ({ ticket: "t", expires_at: "" })),
  },
}));

class FakeSocket {
  static instances: FakeSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((message: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  closed = false;
  constructor(readonly url: string, readonly protocols: string[]) {
    FakeSocket.instances.push(this);
    queueMicrotask(() => this.onopen?.());
  }
  close() {
    this.closed = true;
    this.onclose?.();
  }
}

const { ProjectFeed } = await import("./feed");
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("ProjectFeed", () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeSocket);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("survives a StrictMode start → close → start cycle with exactly one live socket", async () => {
    const feed = new ProjectFeed("demo");
    feed.start();
    feed.close();
    feed.start();
    await settle();
    await settle();
    const open = FakeSocket.instances.filter((socket) => !socket.closed);
    expect(open).toHaveLength(1);
    expect(open[0].protocols).toEqual(["limina.v1", "limina.ticket.t"]);
    expect(feed.getState().connection).toBe("live");
    expect(feed.getState().events.map((event) => event.sequence)).toEqual([1]);
  });

  it("appends streamed events once and notifies listeners", async () => {
    const feed = new ProjectFeed("demo");
    const seen: number[] = [];
    feed.onEvents((events) => seen.push(...events.map((event) => event.sequence)));
    feed.start();
    await settle();
    await settle();
    const socket = FakeSocket.instances.at(-1)!;
    const frame = { type: "event", value: { sequence: 2, type: "runtime.codex", actor: "L", artifact_id: null, detail: {}, created_at: "" } };
    socket.onmessage?.({ data: JSON.stringify(frame) });
    socket.onmessage?.({ data: JSON.stringify(frame) });
    expect(seen).toEqual([2]);
    expect(feed.getState().events.map((event) => event.sequence)).toEqual([1, 2]);
    feed.close();
  });
});
