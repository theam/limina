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
- **Confidence**: Numeric 0.0-1.0 (0.85+ if backed by multiple experiments, 0.60-0.85 if backed by one, <0.60 if analytical/theoretical)
- **Description**: One sentence that says WHAT this card is about AND WHEN to use it — optimized for search retrieval
- **Staleness metadata**: Set `valid_from` to today, `last_validated` to today, `status: ACTIVE`, `superseded_by: null`
- **Related cards**: List IDs of cards that relate to this one (if any exist)
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

### Step 7: Commit and push shared knowledge

If `shared-knowledge/` is a git repo (has a `.git` directory):

```bash
cd shared-knowledge
git add -A
git commit -m "Add Knowledge Cards from mission: {mission-name}"
git push
cd ..
```

If the push fails (auth, permissions), tell the user and provide the commands to run manually.

### Step 8: Final summary

Post a concise summary in chat:
- How many Knowledge Cards were generated
- What domains they cover
- Where they were saved
- Whether they were pushed to the shared repo (or if manual push is needed)

### Guidelines

- **Quality over quantity**: 3 excellent cards > 10 mediocre ones. Only synthesize genuinely reusable knowledge.
- **Be specific**: "LLM caching with diskcache breaks on streaming" is better than "Be careful with caching."
- **Include negative results**: What DIDN'T work is often more valuable than what did.
- **Link back**: Always reference the source artifacts so a future agent can dig deeper if needed.
- **Don't duplicate**: If a finding is too mission-specific to generalize, don't force it into a card.
