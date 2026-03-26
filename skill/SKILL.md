---
name: limina
description: "Set up and launch an autonomous AI research project with Limina. TRIGGER when: user types /limina, says 'start a limina project', 'set up a research project', 'I want to research X autonomously', 'create a research agent', 'autonomous research', 'start a new research mission', or asks to investigate/research a hard technical problem systematically with experiments and evidence. Also triggers on 'limina' mentioned as a tool to use. DO NOT TRIGGER for: general coding questions, simple research lookups, or tasks that don't need structured multi-session research."
---

# Limina

Limina is an autonomous AI research agent framework. Give it a hard technical problem — it hypothesizes, experiments, challenges its own direction, and iterates until it finds a solution backed by evidence.

## Workflow

### Step 1: Introduce Limina

Before asking any setup questions, briefly explain what the user is about to get:

> **What you're setting up:** An autonomous research agent that works through hard problems using a structured loop — hypothesize, experiment, find, review, iterate. Everything it does is written to a persistent knowledge base, so you get the full evidence trail, not just an answer.
>
> The agent runs across hours or days. It picks up where it left off after interruptions. When it gets stuck, it asks you instead of guessing.

### Step 2: Ask for a project name

Ask the user what to name their project directory. Default: `limina-research`.

### Step 3: Clone the template

Clone `https://github.com/theam/limina` into `./<project-name>/` inside the user's current working directory.

```bash
git clone https://github.com/theam/limina.git <project-name>
```

Remove the `.git` directory so the user starts with a clean history:

```bash
rm -rf <project-name>/.git
cd <project-name>
git init
```

### Step 4: Check prerequisites

Verify that `python3` and `git` are available. If missing, attempt to install them or tell the user what to install.

### Step 5: Guide mission definition

Ask the user about their research problem step by step. Show examples before each question to help them think. Use AskUserQuestion when available.

**5a. Objective (required)**

Show these examples first:
- "Find the best embedding model for Spanish legal documents"
- "Compare RAG vs fine-tuning for our customer support bot"
- "Why is our search relevance dropping on long queries?"
- "Our API's P99 latency jumped from 120ms to 800ms — find the root cause and a fix"

Then ask: "What should the agent research?"

**5b. Context (optional)**

Show this example: "We use pgvector with OpenAI embeddings. Retrieval is slow on 1M+ rows."

Then ask: "Any context the agent should know? What's been tried, what exists today?"

If the user skips, leave the section empty.

**5c. Success metric (optional)**

Show these examples: "Latency under 200ms at p95" or "A ranked comparison of 3+ approaches"

Then ask: "What does success look like?"

Default if skipped: "A clear recommendation with evidence from at least 2 experiments"

### Step 6: Write the mission brief

Create `kb/mission/CHALLENGE.md` with the user's answers:

```markdown
# Research Mission

## Objective

<objective from step 5a>

## Context & Baseline

<context from step 5b, or "No prior context provided.">

## Success Metric

<success metric from step 5c>

## Constraints

- Autonomy level: Fully autonomous — escalate only when truly blocked
- If evaluation data does not exist, generate it yourself and document how it was created.
- If additional tools, budget, or access are needed, ask with a clear justification via CEO_REQUESTS.md.

## Escalation Rules

When blocked on resources, access, or decisions, create an entry in `kb/mission/CEO_REQUESTS.md` with status PENDING. Do not proceed on blocked items — wait for a response.
```

### Step 7: Initial commit

```bash
git add -A
git commit -m "Research mission initialized"
```

### Step 8: Tell the user the next step

Tell the user:

> Your research project is ready at `./<project-name>/`.
>
> To start the research agent, open Claude Code in the project directory:
>
> ```bash
> cd <project-name> && claude --dangerously-skip-permissions
> ```
>
> Or for Codex, open the project folder in Full Auto mode.
>
> The agent will read the methodology and start researching your problem automatically. For future projects, just type `/limina`.
