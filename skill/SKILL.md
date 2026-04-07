---
name: limina
description: "Set up and launch an autonomous research project with Limina. Use when the user wants a persistent research agent workflow with hypotheses, experiments, findings, and review artifacts."
---

# Limina

Limina is a small research-first agent template. It keeps durable evidence in `kb/` without dragging a large operational ledger into every session.

## Workflow

### Step 1: Introduce Limina

Before asking setup questions, explain:

> **What you're setting up:** an autonomous research project with a persistent knowledge base, a narrow active-state file, and a required `H -> E -> F` evidence flow.
>
> The agent can work across long sessions, but the always-on context stays small: mission brief, active state, and only the relevant artifacts for the current step.

### Step 2: Ask for a project name

Default: `limina-research`.

### Step 3: Clone the template

Clone `https://github.com/theam/limina.git` into `./<project-name>/`, then remove `.git` and initialize a fresh repo.

### Step 4: Check prerequisites

Verify that `python3`, `git`, and either Claude Code or Codex are available.

Install Python dependencies:

```bash
pip install -r requirements.txt
```

### Step 5: Define the mission

Ask for:

1. objective
2. context or baseline
3. success criteria

Use concise examples and keep the prompt focused on the research problem, not on project administration.

### Step 6: Write the mission brief

Create `kb/mission/CHALLENGE.md` with:

```markdown
---
aliases: ["CHALLENGE"]
type: mission
---

# Research Mission

## Objective

<objective>

## Context

<context or "No additional context provided.">

## Success Criteria

<success criteria>

## Constraints

- Ask when blocked on access, trust in the evaluation, or strategic decisions.
- Persist durable evidence in `kb/`.
- Keep active state in `kb/ACTIVE.md`.

## Links

- Active State: [[ACTIVE]]
- Dashboard: [[DASHBOARD]]
```

Create `kb/ACTIVE.md` with:

```markdown
---
aliases: ["ACTIVE"]
type: active-state
---

# Active State

## Current Objective

Initialize the research loop for the mission.

## Next Step

Read the mission and form the first concrete research question.

## Blocker

None.

## Links

- Mission: [[CHALLENGE]]
```

### Step 7: Initial commit

```bash
git add -A
git commit -m "Initialize Limina research project"
```

### Step 8: Tell the user how to start

Tell the user:

> Your research project is ready at `./<project-name>/`.
>
> Open Claude Code in the project directory:
>
> ```bash
> cd <project-name> && claude
> ```
>
> Or open the folder in Codex.
>
> The runtime loads the mission brief and active state at startup. Hooks enforce `H -> E -> F`, validate kb writes, and run a final kb validation before stop.
>
> To install the bundled repo-local companion skills (`experiment-rigor`, `exploratory-sota-research`, `article-strategy`, `notion-sync-kb`) into Claude Code and/or Codex from inside the project repo, run:
>
> ```bash
> bash scripts/install_skills.sh
> ```
