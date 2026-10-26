import { ArrowRight, Checkmark, Help, Subtract } from "@carbon/icons-react";
import { Link } from "react-router";
import { useRef } from "react";
import { Markdown } from "../knowledge/Markdown";
import type { WorkspaceAssessment } from "../../lib/types";

export function MissionBrief({
  assessment,
  slug,
  canReply,
  answered,
  sending,
  onApprove,
  onDiscuss,
}: {
  assessment: WorkspaceAssessment;
  slug: string;
  canReply: boolean;
  answered: boolean;
  sending: boolean;
  onApprove: (body: string) => void;
  onDiscuss: () => void;
}) {
  const update = assessment.mission_update;
  const decision = update?.decision;
  const decisionRegion = useRef<HTMLElement>(null);
  return (
    <article className="mission-brief">
      <div className="mission-author">
        <span className="mission-avatar" aria-hidden="true">
          L
        </span>
        <strong>Limina</strong>
        <span className="muted">
          Mission assessment ·{" "}
          <time dateTime={assessment.created_at}>
            {new Date(assessment.created_at).toLocaleString(undefined, {
              dateStyle: "medium",
              timeStyle: "short",
            })}
          </time>
        </span>
      </div>
      {update ? (
        <>
          <h2>{update.headline}</h2>
          {decision && (
            <button
              className="mission-review-link link"
              onClick={() => {
                decisionRegion.current?.focus({ preventScroll: true });
                decisionRegion.current?.scrollIntoView({ block: "start" });
              }}
            >
              {answered ? "Review sent direction" : "Review pending decision"} ↓
            </button>
          )}
          <div className="mission-catchup">
            <div className="label">Where the mission stands</div>
            {[
              {
                label: "Established",
                text: update.established,
                icon: <Checkmark size={15} />,
              },
              {
                label: "Ruled out",
                text: update.ruled_out,
                icon: <Subtract size={15} />,
              },
              {
                label: "Still uncertain",
                text: update.uncertain,
                icon: <Help size={15} />,
              },
            ].map((item) => (
              <section className="mission-point" key={item.label}>
                <span aria-hidden="true">{item.icon}</span>
                <div>
                  <h3>{item.label}</h3>
                  <Markdown source={item.text} slug={slug} autolink />
                </div>
              </section>
            ))}
            {update.evidence.length > 0 && (
              <nav
                className="cluster mission-evidence"
                aria-label="Assessment evidence"
              >
                <span className="muted">Evidence</span>
                {update.evidence.map((id) => (
                  <Link
                    className="id-chip"
                    key={id}
                    to={`/projects/${slug}/knowledge/${id}`}
                  >
                    {id}
                  </Link>
                ))}
              </nav>
            )}
          </div>
          {decision && (
            <section
              ref={decisionRegion}
              tabIndex={-1}
              aria-label="Mission decision"
              className="mission-decision"
            >
              <div className="label">
                {answered
                  ? "Your direction has been sent"
                  : "A decision for you"}
              </div>
              <h3>{decision.question}</h3>
              <Markdown source={decision.trade_off} slug={slug} autolink />
              <div className="mission-recommendation">
                <strong>Limina recommends</strong>
                <Markdown
                  source={decision.recommendation}
                  slug={slug}
                  autolink
                />
              </div>
              {answered ? (
                <p className="muted" role="status">
                  Waiting for Limina to incorporate your reply. Measurements
                  stay tied to recorded evidence.
                </p>
              ) : (
                canReply && (
                  <div className="cluster">
                    <button
                      className="btn btn-primary btn-sm"
                      disabled={sending}
                      onClick={() =>
                        onApprove(
                          `I approve your recommendation: ${decision.recommendation}\n\nFor the question: ${decision.question}`,
                        )
                      }
                    >
                      Use this recommendation{" "}
                      <ArrowRight size={14} aria-hidden="true" />
                    </button>
                    <button
                      className="btn btn-ghost btn-sm"
                      onClick={onDiscuss}
                    >
                      Discuss it
                    </button>
                  </div>
                )
              )}
            </section>
          )}
        </>
      ) : (
        <>
          <h2>Latest recorded outcome</h2>
          <Markdown source={assessment.summary} slug={slug} autolink />
          <p className="muted">
            A mission assessment has not been recorded for this checkpoint.
          </p>
        </>
      )}
    </article>
  );
}
