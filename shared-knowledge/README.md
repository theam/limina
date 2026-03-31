# Shared Knowledge

Cross-mission knowledge store for Limina. Contains synthesized Knowledge Cards from completed missions.

## Structure

```
shared-knowledge/
├── INDEX.md       ← Index of all cards and missions (read this first)
├── cards/         ← K001-xxx.md, K002-xxx.md — reusable knowledge
├── missions/      ← One summary per completed mission
└── README.md      ← This file
```

## How it works

1. **At mission close**: The agent runs `/close-mission`, which distills findings, decisions, and lessons into Knowledge Cards.
2. **At mission start**: The agent reads `INDEX.md` and greps `cards/` for terms related to the new challenge.
3. **Cards are read-only** from the perspective of a running mission — they are written only during `/close-mission`.

## Setup options

### Option A: Local directory (simplest)

The `shared-knowledge/` directory lives inside the Limina template. Each cloned mission gets a copy. To share across missions, copy the directory or use symlinks.

### Option B: Git repo (recommended)

Maintain `shared-knowledge/` as a separate git repo. At mission setup, clone it into the project:

```bash
git clone <your-shared-knowledge-repo> shared-knowledge
```

The `/limina` setup skill will ask for this repo URL during project creation.

### Option C: Git submodule

Add as a submodule to the Limina template:

```bash
git submodule add <your-shared-knowledge-repo> shared-knowledge
```
