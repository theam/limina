---
id: "E001"
aliases: ["E001"]
type: experiment
status: "COMPLETED"
hypothesis: "H001"
created: "2026-01-04"
completed: "2026-01-06"
tags: []
---

# E001 — Offline benchmark: BM25 vs. cross-encoder

> **Status**: COMPLETED
> **Hypothesis**: [[H001]]
> **Created**: 2026-01-04
> **Completed**: 2026-01-06

## Objective

Measure NDCG@10 and p95 latency.

## Setup

Identical hardware and candidates.

## Procedure

1. Freeze candidates.
2. Rerank.
3. Score.

## Expected Outcome

At least +10% NDCG.

## Progress

- [x] (2026-01-05) Baseline NDCG@10 is 0.412. — `s3://evidence/baseline.json`
- [x] (2026-01-05) Treatment NDCG@10 is 0.461 — +12%.
- [x] (2026-01-05) Latency p95 is +8 ms.

## Results

NDCG +12%; p95 +8 ms.

## Analysis

Threshold exceeded.

## Decision

Publish F001.

## Links

- Mission: [[CHALLENGE]]
- Active State: [[ACTIVE]]
- Parent Hypothesis: [[H001]]
- Finding: [[F001]]
