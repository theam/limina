import { ArrowDown, Chat, Play, Renew } from "@carbon/icons-react";
import { useMutation } from "@tanstack/react-query";
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Composer,
  type ComposerHandle,
} from "../../components/conversation/Composer";
import {
  ActivityGroup,
  AgentMessage,
  ArtifactEvent,
  Checkpoint,
  HumanMessage,
  Notice,
  TurnDivider,
} from "../../components/conversation/Items";
import { MissionBrief } from "../../components/conversation/MissionBrief";
import { useLifecycle } from "../../components/project/Lifecycle";
import { Skeleton } from "../../components/ui/Empty";
import { Elapsed } from "../../components/ui/Time";
import { useToast } from "../../components/ui/Toast";
import { api } from "../../lib/api";
import { engineLabel, isBlocked } from "../../lib/format";
import {
  useGuidance,
  useMe,
  useProjectStatus,
  useRuns,
  useWorkspace,
} from "../../lib/queries";
import { buildTranscript, type PendingMessage } from "../../lib/transcript";
import type { GuidanceKind, Project } from "../../lib/types";
import { useFeedState, useProjectContext } from "./ProjectContext";

const NEAR_BOTTOM = 96;
let localCounter = 0;
const localMessageId = () =>
  `${Date.now().toString(36)}-${(localCounter += 1)}`;

function Banner({
  project,
  answered,
  onFocus,
}: {
  project: Project;
  answered: boolean;
  onFocus: () => void;
}) {
  const lifecycle = useLifecycle(project.slug);
  const canAct = project.role !== "VIEWER";
  if (project.status === "WAITING" && isBlocked(project.blocker) && answered) {
    return (
      <div className="convo-banner" data-tone="neutral">
        <div>
          <div className="convo-banner-title">Reply sent</div>
          <div className="convo-banner-body">
            Limina picks it up at the start of the next turn.
          </div>
        </div>
      </div>
    );
  }
  if (project.status === "WAITING" && isBlocked(project.blocker)) {
    return (
      <div className="convo-banner" data-tone="warning">
        <div>
          <div className="convo-banner-title">Limina needs your input</div>
          <div className="convo-banner-body">{project.blocker}</div>
        </div>
        {canAct && (
          <button className="btn btn-sm" onClick={onFocus}>
            Answer
          </button>
        )}
      </div>
    );
  }
  const states: Partial<
    Record<
      string,
      {
        title: string;
        body: string;
        action?: "start" | "resume";
        label?: string;
      }
    >
  > = {
    CREATED: {
      title: "This project hasn’t started",
      body: "Start it to begin the first turn.",
      action: "start",
      label: "Start",
    },
    PAUSED: {
      title: "Paused",
      body: "Messages are kept and delivered when you resume.",
      action: "resume",
      label: "Resume",
    },
    STOPPED: {
      title: "Stopped",
      body: "Messages are kept and read when the project starts again.",
      action: "start",
      label: "Start",
    },
    FAILED: {
      title: "The runtime needs attention",
      body: isBlocked(project.blocker)
        ? project.blocker
        : "The last turn failed.",
      action: "resume",
      label: "Retry",
    },
    COMPLETE: {
      title: "Mission complete",
      body: "Limina reported that the success criteria are met.",
    },
    ARCHIVED: { title: "Archived", body: "This conversation is read-only." },
  };
  const state = states[project.status];
  if (!state) return null;
  return (
    <div
      className="convo-banner"
      data-tone={
        project.status === "FAILED"
          ? "critical"
          : project.status === "COMPLETE"
            ? "success"
            : "neutral"
      }
    >
      <div>
        <div className="convo-banner-title">{state.title}</div>
        <div className="convo-banner-body">{state.body}</div>
      </div>
      {canAct && state.action && (
        <button
          className="btn btn-sm btn-primary"
          onClick={() => lifecycle.mutate(state.action!)}
          disabled={lifecycle.isPending}
        >
          {project.status === "FAILED" ? (
            <Renew size={14} />
          ) : (
            <Play size={14} />
          )}
          {state.label}
        </button>
      )}
    </div>
  );
}

export function Conversation() {
  const { slug, feed } = useProjectContext();
  const feedState = useFeedState();
  const status = useProjectStatus(slug);
  const runs = useRuns(slug, 25);
  const assessment = useWorkspace(slug);
  const guidance = useGuidance(slug);
  const vault = undefined;
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyOffset, setHistoryOffset] = useState(0);
  const historyRegion = useRef<HTMLDetailsElement>(null);
  const me = useMe();
  const toast = useToast();
  const project = status.data?.project;

  const visitStarted = useRef(Date.now());
  const [pending, setPending] = useState<PendingMessage[]>([]);
  const [deliveries, setDeliveries] = useState<
    Record<string, "LIVE" | "QUEUED">
  >({});
  const transcript = useMemo(
    () => buildTranscript(feedState.events, pending, guidance.data?.items),
    [feedState.events, pending, guidance.data],
  );
  const visitMessages = transcript.items
    .filter(
      (item) =>
        item.type === "human" &&
        (Date.parse(item.at) >= visitStarted.current ||
          pending.some(
            (message) =>
              message.guidanceId === item.guidanceId &&
              item.guidanceId !== null,
          )),
    )
    .slice(-20);
  const historyItems = transcript.items.slice(
    Math.max(0, transcript.items.length - historyOffset - 50),
    Math.max(0, transcript.items.length - historyOffset),
  );
  const renderHuman = (item: (typeof transcript.items)[number]) =>
    item.type === "human" ? (
      <HumanMessage
        key={item.key}
        item={item}
        delivery={item.guidanceId ? deliveries[item.guidanceId] : undefined}
        onRetry={() =>
          void send(item.body, item.kind, item.key.replace(/^pending:/, ""))
        }
      />
    ) : null;

  const scroller = useRef<HTMLDivElement>(null);
  const composer = useRef<ComposerHandle>(null);
  const stick = useRef(false);
  const oriented = useRef(false);
  const [showJump, setShowJump] = useState(false);
  const prependAnchor = useRef<{ height: number; top: number } | null>(null);

  const runIndex = useMemo(() => {
    const items = runs.data?.items ?? [];
    const total = runs.data?.total ?? items.length;
    return new Map(
      items.map((run, index) => [run.id, { run, number: total - index }]),
    );
  }, [runs.data]);

  const scrollToBottom = useCallback((smooth = false) => {
    const element = scroller.current;
    if (!element) return;
    element.scrollTo({
      top: element.scrollHeight,
      behavior: smooth ? "smooth" : "auto",
    });
  }, []);

  // Restore the reading position after older events are prepended (even if none are visible).
  const firstSequence = feedState.events[0]?.sequence;
  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element || !prependAnchor.current) return;
    const { height, top } = prependAnchor.current;
    element.scrollTop = element.scrollHeight - height + top;
    prependAnchor.current = null;
  }, [firstSequence]);

  // Keep the newest activity in view while the reader is at the bottom; otherwise offer a jump.
  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element || prependAnchor.current) return;
    if (stick.current) scrollToBottom();
    else if (oriented.current && historyOpen) setShowJump(true);
  }, [
    transcript.items.length,
    transcript.activeLabel,
    scrollToBottom,
    historyOpen,
  ]);

  useLayoutEffect(() => {
    if (!feedState.loaded || assessment.isPending || oriented.current) return;
    oriented.current = true;
    if (scroller.current) scroller.current.scrollTop = 0;
  }, [feedState.loaded, assessment.isPending]);

  const loadEarlier = useCallback(async () => {
    const element = scroller.current;
    if (
      !element ||
      !feedState.hasEarlier ||
      feedState.loadingEarlier ||
      prependAnchor.current
    )
      return;
    prependAnchor.current = {
      height: element.scrollHeight,
      top: element.scrollTop,
    };
    await feed.loadEarlier();
    if (
      prependAnchor.current &&
      feed.getState().events[0]?.sequence === firstSequence
    )
      prependAnchor.current = null;
  }, [feed, feedState.hasEarlier, feedState.loadingEarlier, firstSequence]);

  const onScroll = () => {
    const element = scroller.current;
    if (!element) return;
    const distance =
      element.scrollHeight - element.scrollTop - element.clientHeight;
    stick.current = distance < NEAR_BOTTOM;
    if (stick.current) setShowJump(false);
  };

  const actor = me.data?.display_name ?? "You";
  const send = useCallback(
    async (body: string, kind: GuidanceKind, localId = localMessageId()) => {
      const at = new Date().toISOString();
      setPending((items) => [
        ...items.filter((item) => item.localId !== localId),
        {
          localId,
          body,
          kind,
          actor,
          at,
          guidanceId: null,
          delivery: "SENDING",
        },
      ]);
      stick.current = true;
      try {
        const receipt = await api.steer(slug, body, kind);
        setDeliveries((current) => ({
          ...current,
          [receipt.id]: receipt.delivery,
        }));
        setPending((items) =>
          items.map((item) =>
            item.localId === localId
              ? { ...item, guidanceId: receipt.id, delivery: receipt.delivery }
              : item,
          ),
        );
      } catch (error) {
        setPending((items) =>
          items.map((item) =>
            item.localId === localId ? { ...item, delivery: "FAILED" } : item,
          ),
        );
        toast("Message not sent", {
          body: (error as Error).message,
          tone: "critical",
        });
      }
    },
    [slug, actor, toast],
  );

  const interrupt = useMutation({
    mutationFn: () =>
      api.steer(slug, "Pause and await human direction.", "INTERRUPT"),
    onSuccess: () =>
      toast("Interrupted", {
        body: "The project is paused until you resume it.",
      }),
    onError: (error) =>
      toast("Couldn’t interrupt", {
        body: (error as Error).message,
        tone: "critical",
      }),
  });

  if (!project) {
    return (
      <div className="convo">
        <div className="convo-scroll">
          <div className="convo-inner stack" style={{ paddingTop: 32 }}>
            <Skeleton width="40%" height={14} />
            <Skeleton height={60} />
            <Skeleton width="70%" height={40} />
          </div>
        </div>
      </div>
    );
  }

  const running =
    project.status === "RUNNING" && Boolean(transcript.activeRunId);
  const lastCheckpoint = transcript.items.findLastIndex(
    (item) => item.type === "checkpoint",
  );
  const answered =
    transcript.items.findLastIndex((item) => item.type === "human") >
    lastCheckpoint;
  const needsAnswer =
    project.status === "WAITING" && isBlocked(project.blocker);
  const readOnly =
    project.role === "VIEWER" ||
    project.status === "ARCHIVED" ||
    project.status === "COMPLETE";
  const placeholder = needsAnswer
    ? "Answer Limina…"
    : project.status === "RUNNING"
      ? "Steer the work — Limina reads this during the current turn"
      : "Leave guidance for the next turn…";

  return (
    <div className="convo">
      <div className="convo-scroll" ref={scroller} onScroll={onScroll}>
        <div className="convo-inner">
          {!feedState.loaded ? (
            <div className="stack" style={{ paddingTop: 32 }}>
              <Skeleton width="40%" height={14} />
              <Skeleton height={60} />
              <Skeleton width="70%" height={40} />
            </div>
          ) : (
            <>
              {assessment.data &&
                visitMessages
                  .filter(
                    (item) =>
                      Date.parse(item.at) <=
                      Date.parse(assessment.data!.created_at),
                  )
                  .map(renderHuman)}
              {assessment.isPending ? (
                <Skeleton height={200} />
              ) : assessment.error ? (
                <div className="panel panel-body">
                  <p>Couldn’t load the mission assessment.</p>
                  <button
                    className="btn btn-sm"
                    onClick={() => assessment.refetch()}
                  >
                    Try again
                  </button>
                </div>
              ) : assessment.data ? (
                <MissionBrief
                  assessment={assessment.data}
                  slug={slug}
                  canReply={!readOnly}
                  sending={pending.some((item) => item.delivery === "SENDING")}
                  answered={
                    feedState.events.some(
                      (event) =>
                        event.type === "guidance.received" &&
                        event.sequence > assessment.data!.sequence,
                    ) ||
                    pending.some(
                      (item) =>
                        item.delivery !== "FAILED" &&
                        item.delivery !== "SENDING" &&
                        Date.parse(item.at) >
                          Date.parse(assessment.data!.created_at),
                    )
                  }
                  onApprove={(body) => void send(body, "APPROVAL")}
                  onDiscuss={() => composer.current?.focus()}
                />
              ) : (
                <div className="convo-start">
                  <div className="convo-start-icon">
                    <Chat size={20} />
                  </div>
                  <h2 className="convo-start-title">
                    Work with your research agent
                  </h2>
                  <p className="muted">
                    Direct the mission, review evidence and decide what happens
                    next. Limina keeps the working state between visits.
                  </p>
                </div>
              )}
              {visitMessages
                .filter(
                  (item) =>
                    !assessment.data ||
                    Date.parse(item.at) >
                      Date.parse(assessment.data.created_at),
                )
                .map(renderHuman)}
              <details
                ref={historyRegion}
                className="mission-execution"
                open={historyOpen}
                onToggle={(event) => setHistoryOpen(event.currentTarget.open)}
              >
                <summary>Explore the work and conversation history</summary>
                {historyOpen && (
                  <div className="mission-execution-body">
                    <nav
                      className="cluster"
                      aria-label="Execution history pages"
                    >
                      {(historyOffset + 50 < transcript.items.length ||
                        feedState.hasEarlier) && (
                        <button
                          className="btn btn-ghost btn-sm"
                          disabled={feedState.loadingEarlier}
                          onClick={async () => {
                            if (
                              historyOffset + 50 >= transcript.items.length &&
                              feedState.hasEarlier
                            )
                              await loadEarlier();
                            const total = buildTranscript(
                              feed.getState().events,
                              pending,
                            ).items.length;
                            setHistoryOffset(
                              Math.min(
                                historyOffset + 50,
                                Math.max(0, Math.ceil(total / 50) - 1) * 50,
                              ),
                            );
                            requestAnimationFrame(() =>
                              historyRegion.current?.scrollIntoView({
                                block: "start",
                              }),
                            );
                          }}
                        >
                          Earlier page
                        </button>
                      )}
                      {historyOffset > 0 && (
                        <>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() =>
                              setHistoryOffset(Math.max(0, historyOffset - 50))
                            }
                          >
                            Newer page
                          </button>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() => setHistoryOffset(0)}
                          >
                            Latest activity
                          </button>
                        </>
                      )}
                    </nav>

                    {historyItems.map((item) => {
                      switch (item.type) {
                        case "turn": {
                          const entry = item.runId
                            ? runIndex.get(item.runId)
                            : undefined;
                          return (
                            <TurnDivider
                              key={item.key}
                              item={item}
                              run={entry?.run}
                              number={entry?.number}
                              active={
                                item.runId === transcript.activeRunId &&
                                project.status === "RUNNING"
                              }
                            />
                          );
                        }
                        case "message":
                        case "plan":
                          return (
                            <AgentMessage
                              key={item.key}
                              item={item}
                              engine={project.runtime}
                              slug={slug}
                              vault={vault}
                            />
                          );
                        case "activity":
                          return <ActivityGroup key={item.key} item={item} />;
                        case "artifact":
                          return (
                            <ArtifactEvent
                              key={item.key}
                              item={item}
                              slug={slug}
                              vault={vault}
                            />
                          );
                        case "checkpoint":
                          return (
                            <Checkpoint
                              key={item.key}
                              item={item}
                              slug={slug}
                              vault={vault}
                            />
                          );
                        case "human":
                          return (
                            <HumanMessage
                              key={item.key}
                              item={item}
                              delivery={
                                item.guidanceId
                                  ? deliveries[item.guidanceId]
                                  : undefined
                              }
                              onRetry={() => {
                                const localId = item.key.replace(
                                  /^pending:/,
                                  "",
                                );
                                void send(item.body, item.kind, localId);
                              }}
                            />
                          );
                        case "notice":
                          return <Notice key={item.key} item={item} />;
                        default:
                          return null;
                      }
                    })}
                  </div>
                )}
              </details>

              {running && (
                <div className="working" aria-live="polite">
                  <span className="working-mark" aria-hidden="true" />
                  <span className="working-text">
                    {engineLabel(project.runtime)} is working
                    {transcript.activeLabel && (
                      <span className="working-detail">
                        {" "}
                        · {transcript.activeLabel}
                      </span>
                    )}
                  </span>
                  {transcript.activeSince && (
                    <span className="stamp">
                      <Elapsed since={transcript.activeSince} />
                    </span>
                  )}
                </div>
              )}
              {project.status === "RUNNING" && !transcript.activeRunId && (
                <div className="working" aria-live="polite">
                  <span className="working-mark" aria-hidden="true" />
                  <span className="working-text">Preparing the next turn…</span>
                </div>
              )}
            </>
          )}
        </div>
      </div>

      <div className="convo-dock">
        {showJump && (
          <button
            className="jump"
            onClick={() => {
              stick.current = true;
              setShowJump(false);
              scrollToBottom(true);
            }}
          >
            <ArrowDown size={14} /> New activity
          </button>
        )}
        <div className="convo-dock-inner">
          {!(
            assessment.data?.mission_update?.decision &&
            project.status === "WAITING"
          ) && (
            <Banner
              project={project}
              answered={answered}
              onFocus={() => composer.current?.focus()}
            />
          )}
          {!readOnly && (
            <div
              className="mission-suggestions"
              aria-label="Suggested questions"
            >
              {[
                "What remains to achieve the mission?",
                "Why was this approach ruled out?",
                "Show me the decisive evidence.",
              ].map((question) => (
                <button
                  className="btn btn-ghost btn-sm"
                  key={question}
                  onClick={() => composer.current?.insert(question)}
                >
                  {question}
                </button>
              ))}
            </div>
          )}
          {project.role === "VIEWER" ? (
            <p className="convo-readonly">
              You have view access. Ask a project owner for editor access to
              steer.
            </p>
          ) : readOnly ? null : (
            <Composer
              ref={composer}
              placeholder={placeholder}
              defaultKind={needsAnswer ? "ANSWER" : "STEER"}
              running={project.status === "RUNNING"}
              onSend={(body, kind) => void send(body, kind)}
              onInterrupt={() => interrupt.mutate()}
              interrupting={interrupt.isPending}
            />
          )}
        </div>
      </div>
    </div>
  );
}
