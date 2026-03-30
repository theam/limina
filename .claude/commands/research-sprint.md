# Research Sprint — Agent Team for Research Tasks

Spawn an agent team to execute a research task with parallel roles: execution, literature survey, and adversarial review.

## When to Use

Use this when a research task (type: `research`) would benefit from parallel work:
- The hypothesis requires both literature context AND experiment execution
- The research direction needs challenging while experiments run
- Multiple experiments could run in parallel

## Workflow

### Phase 1: Identify the Research Target

Ask the user what to research using `AskUserQuestion`:

**Question**: Which research task should the team work on?
- Show the current research tasks from `kb/mission/BACKLOG.md` as options
- Allow "New task" if the user wants to create one

If new: create the task file in `kb/tasks/` and update BACKLOG.md before proceeding.

### Phase 2: Read Context

Read:
1. The target task file (`kb/tasks/T{NUM}-slug.md`)
2. `kb/mission/BACKLOG.md` — for Last IDs and related tasks
3. `kb/mission/DECISIONS.md` — for prior decisions on this topic
4. Any linked artifacts (prior hypotheses, experiments, findings)

### Phase 3: Create Task Breakdown

Break the research task into subtasks for each role. Create tasks using Claude Code's task list system:

1. **Surveyor tasks**: Literature review, SOTA identification, baseline establishment
2. **Researcher tasks**: Experiment design, execution, data collection, analysis
3. **Critic tasks**: Review hypothesis quality, challenge experiment design, validate findings

Set up dependencies:
- Surveyor tasks have no dependencies (can start immediately)
- Researcher tasks may depend on surveyor establishing baselines
- Critic tasks depend on having something to review (but can start reviewing hypotheses immediately)

### Phase 4: Spawn the Team

Create an agent team and spawn 3 teammates:

**Surveyor** (subagent_type: general-purpose, agent: surveyor):
```
You are the Surveyor for research task T{NUM}: "{task title}".
Your job: find and synthesize existing work relevant to this task.
Read the task file at kb/tasks/T{NUM}-slug.md for full context.
Follow the /literature-search methodology: use all 4 search channels (academic, code, practitioner, benchmarks),
evaluate sources on recency/reproducibility/relevance/rigor/bias, and use snowball citations.
Create literature files (L{NUM}) in kb/research/literature/ for each significant source.
Focus on: SOTA approaches, baselines we should compare against, and approaches nobody has tried.
Report findings to the team lead when done.
```

**Researcher** (subagent_type: general-purpose, agent: researcher):
```
You are the Researcher for research task T{NUM}: "{task title}".
Your job: design and execute experiments to test the hypotheses.
Read the task file at kb/tasks/T{NUM}-slug.md for full context.
Follow the H→E→F chain strictly. Check kb/mission/BACKLOG.md for Last IDs before creating artifacts.
Save all data to kb/research/data/. Make everything reproducible.
Report results to the team lead when done.
```

**Critic** (subagent_type: general-purpose, agent: devil-advocate):
```
You are the Devil's Advocate for research task T{NUM}: "{task title}".
Your job: challenge everything the team produces — hypotheses, experiment design, findings.
Read the task file at kb/tasks/T{NUM}-slug.md for full context.
Start by reviewing the hypothesis — is it truly falsifiable? Are the metrics meaningful?
After experiments run, challenge the results — is the baseline fair? Does the data support the conclusion?
Write your review to kb/reports/CR{NUM}-research-sprint-T{NUM}.md.
Report critical issues to the team lead immediately.
```

### Phase 5: Coordinate

As team lead:
1. **Use delegate mode** — STEER, don't execute. Your job is to review subagent output critically and course-correct.
2. If a researcher's experiment design is weak, send it back with specific feedback. If the surveyor missed an important angle, redirect them. If the critic's review is superficial, push for depth.
3. Wait for surveyor to establish baselines before researcher finalizes experiment design
4. Route critic's feedback to the researcher (e.g., "Critic found issue X with your experiment design — address before running")
5. Synthesize all findings when teammates complete their work
6. If you find yourself writing experiment code, literature files, or data analysis — STOP. You have left delegate mode. Send the work to the appropriate teammate.

### Phase 6: Synthesize and Close

After all teammates finish:
1. Read all artifacts produced (literature files, experiment files, findings, review)
2. Update the task file with final status and linked artifacts
3. Update `kb/mission/BACKLOG.md` with task status and new Last IDs
4. Update `kb/INDEX.md` with all new artifacts
5. Write a summary finding or decision to `kb/`
6. Run `python3 scripts/kb_validate.py`
7. Clean up the team
8. Post a concise user summary in chat

### Phase 7: Communicate

Post a concise summary in the active session:
```
Research Sprint for T{NUM} complete.

Task: {title}
Hypothesis: {confirmed/rejected/inconclusive}
Key finding: {one sentence}
Literature reviewed: {count} sources
Experiments run: {count}
Critical issues from review: {count}

Artifacts: {list of new KB artifacts}
Next steps: {recommended direction}
```
