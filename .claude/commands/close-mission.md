---
name: close-mission
description: "Synthesize reusable knowledge from the current mission into Knowledge Cards before closing. TRIGGER when: user says 'close mission', 'end mission', 'wrap up', 'synthesize knowledge', or all tasks in BACKLOG.md are DONE. DO NOT TRIGGER for: ending a session (use normal session-end protocol instead)."
---

# Close Mission

You are closing a Limina mission. Your job is to **distill reusable knowledge** from this mission into Knowledge Cards that future missions can use. This is NOT a session end — this is a mission end.

## Workflow

### Step 1: Validate the KB

Run `python3 scripts/kb_validate.py`. Fix any errors before proceeding. The KB must be consistent before synthesis.

### Step 2: Identify reusable knowledge

Read the following artifacts and identify items that would be valuable in future missions (not just this one):

1. **All findings** (`kb/research/findings/`) — focus on HIGH and MEDIUM impact
2. **DECISIONS.md** — focus on decisions with broad applicability (not mission-specific)
3. **Lessons Learned** section in CLAUDE.md — focus on patterns that transcend this mission
4. **Strategic reviews** (`kb/reports/SR*.md`) — look for direction-changing insights
5. **Retrospectives** (`kb/engineering/retrospectives/`) — look for reusable engineering patterns

For each item, ask: **"Would a future agent benefit from knowing this, even in a different domain?"** If yes, it becomes a Knowledge Card.

### Step 3: Generate Knowledge Cards

For each reusable item, create a Knowledge Card using the template at `templates/knowledge-card.md`.

- **ID**: Sequential K{NUM} — check the shared knowledge INDEX.md for the last used ID
- **Source mission**: The name of this project/repo
- **Confidence**: HIGH if backed by multiple experiments, MEDIUM if backed by one, LOW if analytical/theoretical
- **What Works / What Doesn't Work**: Be specific. Include the approach AND the evidence.
- **Conditions**: Be explicit about when this applies and when it doesn't. Overgeneralization is worse than undergeneralization.
- **Pitfalls**: Things that wasted time or caused errors. These are often the most valuable part.

### Step 4: Write cards to shared knowledge

If `shared-knowledge/` exists in the project root:
- Write cards to `shared-knowledge/cards/K{NUM}-slug.md`
- Update `shared-knowledge/INDEX.md` with one-line summaries

If `shared-knowledge/` does NOT exist:
- Write cards to `kb/cards/K{NUM}-slug.md` (create the directory if needed)
- Tell the user: "Knowledge Cards saved locally. To share across missions, set up a shared-knowledge directory (see README)."

### Step 5: Write mission summary

Create a mission summary file:
- Location: `shared-knowledge/missions/{mission-name}.md` (or `kb/missions/{mission-name}.md` if no shared dir)
- Content:
  ```markdown
  # Mission: {name}

  > **Date**: {start} — {end}
  > **Outcome**: {one-line result}
  > **Cards generated**: K{NUM}, K{NUM}, ...

  ## Objective
  {From CHALLENGE.md}

  ## Key Results
  {2-3 bullet points}

  ## Knowledge Cards Generated
  | ID | Title | Domain | Confidence |
  |---|---|---|---|
  ```

### Step 6: Update the shared INDEX

If `shared-knowledge/INDEX.md` exists, add entries for all new cards and the mission summary.

### Step 7: Final summary

Post a concise summary in chat:
- How many Knowledge Cards were generated
- What domains they cover
- Where they were saved
- Remind the user to commit and push shared-knowledge/ if it's a git repo

### Guidelines

- **Quality over quantity**: 3 excellent cards > 10 mediocre ones. Only synthesize genuinely reusable knowledge.
- **Be specific**: "LLM caching with diskcache breaks on streaming" is better than "Be careful with caching."
- **Include negative results**: What DIDN'T work is often more valuable than what did.
- **Link back**: Always reference the source artifacts so a future agent can dig deeper if needed.
- **Don't duplicate**: If a finding is too mission-specific to generalize, don't force it into a card.
