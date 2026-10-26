# Limina interface design

Read this before changing the web console, its presentation data, or an API used to navigate it.
This is the product design contract linked from [AGENTS.md](AGENTS.md) and
[CLAUDE.md](CLAUDE.md). Keep it current when user feedback establishes a reusable rule.

## Start with the user's decision

- The project opens into the conversation with its agent. This is the main workspace for
  understanding, directing and reviewing the mission across visits.
- Progress, knowledge, results and configuration support that conversation. Open evidence and
  formatted documents in adjacent tabs while preserving the conversation, draft and reading position.
- The supporting status view answers: where do we stand against the mission, what needs my
  attention, what happens next, and what did the latest evidence establish? It is not a separate
  dashboard the user must visit before talking to the agent.
- Organize around those tasks rather than exposing the database schema or the agent's internals.
- Show information where it changes a decision. Permissions belong in sharing and settings;
  a role label such as `owner` does not belong beside the project title.
- A status has one primary home. Do not repeat the same badge in the toolbar and page heading.

## Design for returning to a long-running project

- A user may miss hours or days of autonomous work. Start with what changed, what was learned,
  what was ruled out, what is still uncertain, and the next decision or action.
- Organize the conversation around mission-level updates and human decisions. Raw provider
  turns and execution detail are available on demand, not the default reading obligation.
- While the agent works, show a short activity description in the primary status. Keep raw
  commands, arguments and file paths in the expandable execution history.
- Distinguish an expected user pause from a failed provider turn. Cancellation should not present
  the provider's missing-final-response diagnostic as an additional failure.
- A return summary has an explicit time range and evidence links. Use “since your last visit”
  only when that visit is actually known; otherwise label it as the current state or a dated update.
- Distinguish synthesis from activity. Do not generate a supposed global assessment merely by
  shortening the last message or counting experiments. The summary needs grounded, durable content.
- Present a pending decision with its trade-off, the agent's recommendation and relevant evidence.
  Answering records the user's direction and changes planned work; it does not manufacture results.
- Keep the composer and project context available while consulting evidence. Notes, previous
  versions and result files should not force users to abandon their conversation and reconstruct it.
- Resolve generated Markdown links to known notes and published results inside the workspace.
  Clearly label unpublished workspace references rather than sending readers to broken pages.
- Reading a historical version is read-only and clearly dated. It does not change the current
  project state or imply a rollback.
- On narrow screens, preserve conversation priority and reveal supporting content progressively.
- Start a returning visit at the assessment headline. Preserve the draft and conversation reading
  position as supporting tabs change; reset retained state when switching projects.
- Typed agent assessments and deliberately published files belong to durable runtime state. Keep
  delivery, incorporation, human approval, observed measurements and completion distinct. See the
  [workspace contract](docs/agent-workspace.md) for the current schema and provider guidance.

## Give each layer a content budget

- A title identifies the object and wraps when necessary. Do not hide its distinguishing words.
- A subtitle is a short description, not a mission document. Use a dedicated short value
  (at most 160 characters) or omit it and provide a clearly named route to the mission.
- Objective and next-step previews should read as short headlines. The stored source may be
  long Markdown; preserve it and make the full version explicitly available.
- Never present a mechanically shortened excerpt as a complete assessment or invented summary.
  Show a disclosure or a named detail link beside the preview.
- Essential content must never terminate at an ellipsis. Every shortened item needs an obvious,
  keyboard-accessible way to read the entire value: expand inline or open its detail view.
- A tooltip, hover card, browser `title`, or mouse-only gesture is supplementary access.
- Detail views render the complete content, wrap long words, and allow normal page scrolling.
- Do not remove valuable content to solve overflow; move it to the correct information layer.

## Show progress toward the mission

- Hypothesis, experiment, finding and turn counts measure activity, not mission progress.
  Use counts for navigation and inventory, never as a progress score or a completion percentage.
- Operational state (working, paused, needs input) and outcome progress are different facts.
- Measured progress needs an explicit metric, baseline, current value, target, unit/direction,
  observation date, and evidence reference. A claimed result should link to its supporting note.
- Do not infer KPI attainment from a project being marked complete or a test being green.
- Missing measurements must be visibly unknown. Do not show zero, a fabricated percentage,
  or a success color in their place. Distinguish proposed goals from observed results.
- Where only qualitative evidence exists, show the latest recorded outcome and success criteria
  as such. Keep the full assessment accessible; do not imply a numeric measurement exists.
- Surface blockers and the next useful action before secondary history or operational statistics.
- Mission, context, and the complete brief belong in a readable detail view, not duplicated
  verbatim beneath the overview.

## Make growing knowledge navigable

- Start with search and a small set of categories. Preserve the H → E → F hierarchy through
  parent links, breadcrumbs and focused related-evidence queries.
- Do not open the entire research graph or a recursively expanded tree by default.
- Search the canonical server index, including artifact IDs, titles and content. A search over
  the first downloaded page is not a search over the project.
- Bound every collection: normally 25 results per page and at most 50 rendered rows. Provide
  explicit pagination, a result count, filters, and a clear way to reset those filters.
- Load a note and its relevant relationships when requested. Reading one note must not require
  exporting every note in the project. Avoid nested scans across the full vault.
- Put search, category, parent scope and pagination in the URL so links and browser history work.
- Keep titles readable and avoid repeating IDs, status or descriptions within the same row.
- Keep loading, no matches, no content, stale data, and errors distinct. Errors offer retry.
- The UI and MCP use the same authorized, filtered, paginated knowledge operations. Agents should
  search, read an ID and follow a parent scope without downloading the complete graph.

## Accessibility is part of the interaction

- Use semantic landmarks, a skip link, one primary heading and a logical heading hierarchy.
- Use links for navigation and buttons for actions. Never nest links inside a clickable card.
- Every input has an accessible label, including search; icons used decoratively are hidden
  from assistive technology. Icon-only actions need specific accessible names.
- Make every action and disclosure usable with a keyboard, with visible focus. Dialogs have
  names, focus management, Escape dismissal and focus return.
- Do not rely only on color, shape, a hover gesture, or a transient notification for meaning.
- Announce asynchronous search status and validation politely without moving focus per keystroke.
- Preserve zoom and reduced motion. Layouts reflow at narrow widths without cutting off content,
  hiding controls, or requiring horizontal scrolling for ordinary prose.
- Maintain adequate text contrast, usable touch targets, and clear hover, focus and disabled states.

## Moving execution to cloud must preserve the user's workflow

- Validate both a new project and a complete existing local project from the user's normal agent.
- Keep instance setup, migration and environment preparation explicit and understandable. A user
  should not need database access or manual container repair to continue a project remotely.
- Account for instructions, lessons, evidence, code, data, dependencies and relevant history when
  transferring a project. Preview omissions; never silently treat a partial copy as complete.
- Check that the actual experiment can run before declaring the environment ready. Provider
  availability and a defined mission do not prove repository, dependency or data readiness.
- Make current results, earlier versions and generated files accessible from both the console
  and the user's local agent. Explain which state is authoritative and how local edits interact.
- Prove continuity with disconnect/reconnect and restart checks on the intended deployment.
  A polished console or a local demo does not establish hosted production continuity.

## Verify the whole experience

- Check realistic short and long text, missing state, errors, and a project with thousands of notes.
- Exercise search for an off-page note, pagination, filter reset, a deep link, browser back,
  a parent/child relationship, and the complete content behind every summary.
- Check keyboard navigation and a narrow viewport in addition to the normal desktop view.
- Verify that any claimed KPI or outcome comes from persisted evidence rather than UI heuristics.
- Run meaningful behavior tests, type checking and a production build. Inspect the result in a
  browser and retain a screenshot of the implemented view.
- Keep this contract about user outcomes. Implementation specifics belong in the
  [console guide](docs/web-console.md).

## References

- [Web Interface Guidelines](https://github.com/vercel-labs/web-interface-guidelines)
- [WCAG 2.2](https://www.w3.org/TR/WCAG22/)
- [Active delivery state](kb/ACTIVE.md)
