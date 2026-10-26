# Agent-centered project workspace

The project workspace is implemented by the project route's `Workspace` layout. The conversation
stays mounted while State, Knowledge, Results and bounded document/file tabs change beside it.
A project change resets the workspace. Reading a note preserves the current conversation draft
and scroll position. On entry the assessment starts at its headline; execution history is opt-in and paginated at 50 items.
On narrow screens conversation comes first and context follows it, with an explicit reveal control.

## Durable agent output

Both existing CLI adapters receive the same structured checkpoint schema. The optional
`mission_update` carries `headline`, `established`, `ruled_out`, `uncertain`, up to eight evidence
IDs, and an optional decision with `question`, `recommendation`, `trade_off`. Its synthesis is
cumulative across the mission. It is agent-authored, not extracted by shortening a log entry.

The parser bounds prose and validates reference syntax. The supervisor checks that evidence notes
exist in the project, redacts secrets, and persists the assessment on the completed-turn event.
Existence checks are not semantic proof: the agent must still verify the underlying evidence.
The next turn receives the prior assessment along with canonical mission, ACTIVE and new guidance.
The UI reads the newest completed checkpoint independently of transcript pagination. A dated
assessment never claims “since your last visit”; no visit watermark is invented. Older checkpoints
retain their wire shape and show a clearly labeled latest-outcome fallback.

Approving a recommendation sends normal durable `APPROVAL` guidance quoting the question and recommendation. The existing delivery/acknowledgment mechanism applies. The console also reads canonical inbox receipts because checkpoint acknowledgment need not emit an individual incorporation event.
It does not synchronously resolve the decision, modify KPIs, or declare the mission complete.
The agent incorporates the response and removes a resolved question from its next checkpoint.

## Read surfaces

- `GET /v1/projects/{slug}/workspace`: latest durable checkpoint assessment, or null.
- `GET …/knowledge/{id}/history?before={version}`: 25 historical versions, `next_before` cursor.
  Existing unpaginated revisions remain compatible. History is read-only, dated, and rendered
  from that revision's fields; it never reconstructs old content from the current note.
- `GET …/outputs?offset=0`: 25 published files per page, with `next_offset`. Scanning stops after
  1,000 directory entries and reports truncation. Files over 10 MiB are not published.
- `GET …/outputs/{name}`: UTF-8 preview (128 KiB), explicit truncation, or binary download hint.
- `GET …/outputs/{name}/download`: authorized attachment, octet-stream, nosniff and no-store.

Only flat non-hidden regular files in the project's `outputs/` are exposed. Path components and
symbolic links are rejected; Unix file opens use O_NOFOLLOW. The agent is instructed to publish
finished deliverables, keeping secrets/internal state outside that directory. This is deliberate
publication by the project agent, not a general file browser or a sandbox for hostile agent code.

HTTP and four additive MCP tools call the same Viewer-authorized operations: workspace assessment,
output list, output preview, and paginated note history. Editors retain the existing KPI editor.
Conversation Markdown links open focused documents without loading a full project snapshot.

## Vendor harness guidance used

Reviewed 2026-10-23 against the exact requested models:

- [OpenAI: Using GPT-6](https://developers.openai.com/api/docs/guides/latest-model): the GPT-6.1 Sol
  section and shared prompting guidance. Applied explicit goal/context/constraints/completion,
  autonomous follow-through within authorized scope, proportional verification, durable state and
  schema-constrained output. Provider sessions/continuations remain native; no extra model loop or
  new API credential path is introduced.
- [Anthropic: Prompting Claude Opus 5.5](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-opus-5-5):
  applied explicit unattended completion, bounded recovery, observable progress and concise user
  updates. A text-only end is not completion; the runtime still requires its structured checkpoint.
  Private thinking is not requested or rendered as the mission assessment. Native CLI owns thinking
  blocks; no unsupported forced tool choice or thinking-disable parameter is added.

Both vendors recommend calibrating effort to the workload. Project model/effort choices continue
to come from the live provider catalog; level names are not assumed equivalent across models, and
higher effort is not forced globally. Provider adapters continue to use native schema support.

## Verification boundaries

Tests cover structured-output rejection/redaction, legacy compatibility, evidence access,
independent checkpoint recovery, bounded history/output access, attachment bytes, and approval
without fabricated KPI values. UI tests and browser checks cover the retained draft and adjacent
context. The isolated scripted runtime supplies explicitly illustrative evidence and outputs for
visual inspection. These checks do not establish production cloud continuity or measure the output
quality of paid live Opus 5.5 / GPT-6.1 Sol runs.

## Links

- [Shared design contract](../design.md)
- [Deployment boundaries](deployment-boundaries.md)
