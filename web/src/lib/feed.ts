/**
 * Live project event feed.
 *
 * Loads the newest page of durable events, pages backwards on demand, and follows new events
 * through the ticketed WebSocket (`limina.v1` subprotocol). If the socket cannot be held open
 * the feed falls back to cursor polling, so the transcript never silently stalls.
 */

import { api } from "./api";
import type { LiminaEvent, ProjectStatusResponse } from "./types";

export type Connection = "connecting" | "live" | "polling" | "offline";

export interface FeedState {
  events: LiminaEvent[];
  hasEarlier: boolean;
  loadingEarlier: boolean;
  loaded: boolean;
  connection: Connection;
  error: string | null;
}

type Listener = () => void;
type EventsListener = (events: LiminaEvent[]) => void;
type SnapshotListener = (status: ProjectStatusResponse) => void;

const PAGE = 300;
const POLL_MS = 2500;

export class ProjectFeed {
  private state: FeedState = {
    events: [],
    hasEarlier: false,
    loadingEarlier: false,
    loaded: false,
    connection: "connecting",
    error: null,
  };
  private listeners = new Set<Listener>();
  private eventListeners = new Set<EventsListener>();
  private snapshotListeners = new Set<SnapshotListener>();
  private socket: WebSocket | null = null;
  private pollTimer: number | null = null;
  private retryTimer: number | null = null;
  private failures = 0;
  private closed = true;
  /** Bumped on every start/close so async work from a previous lifecycle cannot act. */
  private generation = 0;
  private seen = new Set<number>();

  constructor(readonly slug: string) {}

  getState = (): FeedState => this.state;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  onEvents(listener: EventsListener): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  onSnapshot(listener: SnapshotListener): () => void {
    this.snapshotListeners.add(listener);
    return () => this.snapshotListeners.delete(listener);
  }

  /** Start (or restart) following the project. Safe under React StrictMode double effects. */
  start(): void {
    if (!this.closed) return;
    this.closed = false;
    const generation = ++this.generation;
    document.addEventListener("visibilitychange", this.onVisibility);
    const ready = this.state.loaded ? this.catchUp() : this.loadLatest();
    void ready.then(() => {
      if (generation === this.generation) void this.connect(generation);
    });
  }

  close(): void {
    this.closed = true;
    this.generation += 1;
    document.removeEventListener("visibilitychange", this.onVisibility);
    this.stopPolling();
    if (this.retryTimer) window.clearTimeout(this.retryTimer);
    this.socket?.close();
    this.socket = null;
  }

  async loadEarlier(): Promise<void> {
    const first = this.state.events[0];
    if (!first || !this.state.hasEarlier || this.state.loadingEarlier) return;
    this.set({ loadingEarlier: true });
    try {
      const page = await api.events(this.slug, { before: first.sequence, limit: PAGE });
      const fresh = page.events.filter((event) => !this.seen.has(event.sequence));
      fresh.forEach((event) => this.seen.add(event.sequence));
      this.set({
        events: [...fresh, ...this.state.events],
        hasEarlier: Boolean(page.has_earlier),
        loadingEarlier: false,
      });
    } catch (error) {
      this.set({ loadingEarlier: false, error: (error as Error).message });
    }
  }

  private get cursor(): number {
    const events = this.state.events;
    return events.length ? events[events.length - 1].sequence : 0;
  }

  private set(patch: Partial<FeedState>): void {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((listener) => listener());
  }

  private append(events: LiminaEvent[]): void {
    const fresh = events.filter((event) => !this.seen.has(event.sequence));
    if (!fresh.length) return;
    fresh.forEach((event) => this.seen.add(event.sequence));
    this.set({ events: [...this.state.events, ...fresh].sort((a, b) => a.sequence - b.sequence) });
    this.eventListeners.forEach((listener) => listener(fresh));
  }

  private async loadLatest(): Promise<void> {
    try {
      const page = await api.events(this.slug, { before: Number.MAX_SAFE_INTEGER, limit: PAGE });
      page.events.forEach((event) => this.seen.add(event.sequence));
      this.set({ events: page.events, hasEarlier: Boolean(page.has_earlier), loaded: true, error: null });
    } catch (error) {
      this.set({ loaded: true, error: (error as Error).message });
    }
  }

  private async catchUp(): Promise<void> {
    try {
      const page = await api.events(this.slug, { after: this.cursor, limit: 1000 });
      this.append(page.events);
    } catch {
      /* the socket or polling will retry */
    }
  }

  private async connect(generation = this.generation): Promise<void> {
    if (this.closed || generation !== this.generation) return;
    if (this.failures >= 3) {
      // Poll meanwhile, and try the socket again later so a recovered runtime goes live again.
      this.startPolling();
      if (this.retryTimer) window.clearTimeout(this.retryTimer);
      this.retryTimer = window.setTimeout(() => {
        this.failures = 2;
        void this.connect(generation);
      }, 30_000);
      return;
    }
    this.set({ connection: this.state.connection === "live" ? "live" : "connecting" });
    let ticket: string;
    try {
      ticket = (await api.liveTicket(this.slug)).ticket;
    } catch {
      this.failures += 1;
      this.scheduleReconnect();
      return;
    }
    if (this.closed || generation !== this.generation) return;
    const scheme = window.location.protocol === "https:" ? "wss" : "ws";
    const url = `${scheme}://${window.location.host}/v1/projects/${encodeURIComponent(this.slug)}/live?after=${this.cursor}`;
    const socket = new WebSocket(url, ["limina.v1", `limina.ticket.${ticket}`]);
    this.socket = socket;
    let opened = false;
    socket.onopen = () => {
      opened = true;
      this.failures = 0;
      this.stopPolling();
      this.set({ connection: "live", error: null });
    };
    socket.onmessage = (message) => {
      try {
        const payload = JSON.parse(message.data as string);
        if (payload.type === "event") this.append([payload.value as LiminaEvent]);
        else if (payload.type === "snapshot")
          this.snapshotListeners.forEach((listener) => listener(payload.value as ProjectStatusResponse));
      } catch {
        /* ignore malformed frames */
      }
    };
    socket.onclose = () => {
      if (this.socket !== socket) return;
      this.socket = null;
      if (this.closed) return;
      if (!opened) this.failures += 1;
      this.scheduleReconnect();
    };
  }

  private scheduleReconnect(): void {
    if (this.closed) return;
    // Keep the transcript moving with polling while the socket is re-established.
    this.startPolling();
    const delay = Math.min(15_000, 1000 * 2 ** this.failures);
    if (this.retryTimer) window.clearTimeout(this.retryTimer);
    const generation = this.generation;
    this.retryTimer = window.setTimeout(() => void this.connect(generation), delay);
  }

  private startPolling(): void {
    if (this.pollTimer !== null || this.closed) return;
    this.set({ connection: navigator.onLine ? "polling" : "offline" });
    const tick = async () => {
      if (document.visibilityState === "visible") {
        try {
          const page = await api.events(this.slug, { after: this.cursor, limit: 500 });
          this.append(page.events);
          if (this.state.connection !== "live") this.set({ connection: "polling" });
        } catch {
          this.set({ connection: navigator.onLine ? "polling" : "offline" });
        }
      }
      if (this.pollTimer !== null) this.pollTimer = window.setTimeout(tick, POLL_MS);
    };
    this.pollTimer = window.setTimeout(tick, POLL_MS);
  }

  private stopPolling(): void {
    if (this.pollTimer !== null) window.clearTimeout(this.pollTimer);
    this.pollTimer = null;
  }

  private onVisibility = () => {
    if (document.visibilityState === "visible" && !this.socket && !this.closed) {
      this.failures = 0;
      void this.catchUp().then(() => this.connect(this.generation));
    }
  };
}
