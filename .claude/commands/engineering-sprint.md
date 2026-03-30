# Engineering Sprint — Agent Team for Engineering Tasks

Spawn an agent team to execute an engineering task with parallel roles: implementation, code review, and adversarial challenge.

## When to Use

Use this when an engineering task (type: `engineering`) would benefit from parallel work:
- The feature is complex enough to benefit from review while building
- The approach needs challenging while implementation proceeds
- You want a reviewer checking architecture and code quality in real-time

## Workflow

### Phase 1: Identify the Engineering Target

Ask the user what to build using `AskUserQuestion`:

**Question**: Which engineering task should the team work on?
- Show the current engineering tasks from `kb/mission/BACKLOG.md` as options
- Allow "New task" if the user wants to create one

If new: create the task file in `kb/tasks/` and update BACKLOG.md before proceeding.

### Phase 2: Read Context

Read:
1. The target task file (`kb/tasks/T{NUM}-slug.md`)
2. `kb/mission/BACKLOG.md` — for Last IDs and related tasks
3. `kb/mission/DECISIONS.md` — for prior decisions
4. Any linked artifacts (investigations, feature specs)
5. `kb/project/ARCHITECTURE.md` if it exists

### Phase 3: Plan Before Building

**IMPORTANT**: Require the Builder to plan before implementing.

1. Create the feature spec (FT{NUM}) if it doesn't exist
2. Create the investigation (INV{NUM}) if approach hasn't been decided
3. Document the approach decision in DECISIONS.md

### Phase 4: Create Task Breakdown

Break the engineering task into subtasks:

1. **Builder tasks**: Implementation, tests, documentation
2. **Reviewer tasks**: Code review, architecture check, test coverage validation
3. **Critic tasks**: Challenge approach, find edge cases, security review

Set up dependencies:
- Builder starts implementing after feature spec is approved
- Reviewer reviews as Builder produces code
- Critic challenges the approach from the start

### Phase 5: Spawn the Team

Create an agent team and spawn 3 teammates:

**Builder** (subagent_type: general-purpose, agent: builder, mode: plan):
```
You are the Builder for engineering task T{NUM}: "{task title}".
Your job: implement the feature according to the spec.
Read the task file at kb/tasks/T{NUM}-slug.md for full context.
Read the feature spec at kb/engineering/features/FT{NUM}-slug.md.
Plan your implementation first (you're in plan mode — the lead must approve before you code).
Create IMP{NUM} in kb/engineering/implementations/ to track progress.
Write clean, tested, secure code. Report completion to the team lead.
```

**Reviewer** (subagent_type: general-purpose, agent: reviewer):
```
You are the Reviewer for engineering task T{NUM}: "{task title}".
Your job: review code quality, architecture alignment, test coverage, and security.
Read the task file at kb/tasks/T{NUM}-slug.md for full context.
Read kb/project/ARCHITECTURE.md if it exists for architecture guidelines.
Review the Builder's code as it's produced. Check for bugs, security issues, missing tests.
Report issues to the team lead with specific file paths and line numbers.
```

**Critic** (subagent_type: general-purpose, agent: devil-advocate):
```
You are the Devil's Advocate for engineering task T{NUM}: "{task title}".
Your job: challenge the engineering approach and find what could go wrong.
Read the task file at kb/tasks/T{NUM}-slug.md for full context.
Read kb/mission/DECISIONS.md for the approach decision.
Challenge: Why this approach? What happens at 10x scale? What if a dependency breaks?
What edge cases were missed? What would a senior engineer push back on?
Write your review to kb/reports/CR{NUM}-engineering-sprint-T{NUM}.md.
```

### Phase 6: Coordinate

As team lead:
1. **Use delegate mode** — STEER, don't execute. Review subagent output critically and course-correct.
2. **Approve Builder's plan** before letting them implement
3. If the Builder's approach is weak, send it back with specific feedback. If the Reviewer's review is shallow, push for depth. If the Critic is missing edge cases, redirect them.
4. Route Reviewer's findings to Builder for fixes
5. Route Critic's challenges to Builder for response (address or justify dismissal)
6. Don't close until Reviewer approves and Critic's critical issues are addressed
7. If you find yourself writing feature code, tests, or implementation details — STOP. You have left delegate mode.

### Phase 7: Synthesize and Close

After all teammates finish:
1. Verify implementation passes all tests
2. Verify Reviewer has approved (or all requested changes are addressed)
3. Verify Critic's critical issues are resolved
4. Update the task file with final status and linked artifacts
5. Update `kb/mission/BACKLOG.md` with task status and new Last IDs
6. Update `kb/INDEX.md` with all new artifacts
7. Run `python3 scripts/kb_validate.py`
8. Clean up the team
9. Post a concise user summary in chat

### Phase 8: Communicate

Post a concise summary in the active session:
```
Engineering Sprint for T{NUM} complete.

Task: {title}
Status: {delivered / needs iteration}
Files: {count} created/modified
Tests: {pass/fail count}
Review: {approved / approved with comments}
Critical issues resolved: {count}

Artifacts: {list of new KB artifacts}
Next steps: {follow-up work if any}
```
