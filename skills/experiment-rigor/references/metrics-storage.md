# Metrics Storage

## Principle

Keep Limina's narrative and decisions in `kb/`.
Keep raw metrics in machine-readable files under `kb/research/data/`.
If you also use an external tracker, treat `kb/` as the canonical cross-session memory and store the external run IDs there.

## Recommended layout

Use one directory per experiment:

```text
kb/research/data/
└── E003/
    ├── manifest.json
    ├── summary.json
    ├── runs.csv
    ├── examples.jsonl
    ├── plots/
    └── artifacts/
```

Use timestamped subfiles only when the same experiment is rerun many times and you need multiple versions.

## Minimum required files

### `manifest.json`
Store the conditions needed to reproduce the run.

Recommended fields:
- `experiment_id`
- `hypothesis_id`
- `created_at_utc`
- `code_revision`
- `command`
- `environment`
- `hardware`
- `dataset`
- `seeds`
- `trial_count`
- `candidate`
- `baseline`
- `external_runs`

Example:

```json
{
  "experiment_id": "E003",
  "hypothesis_id": "H002",
  "created_at_utc": "2026-04-07T12:00:00Z",
  "code_revision": "git:abc1234",
  "command": "python eval.py --config configs/e003.yaml",
  "environment": {
    "python": "3.12.2",
    "packages_file": "requirements-lock.txt"
  },
  "hardware": {
    "device": "A100",
    "count": 1
  },
  "dataset": {
    "name": "catalog-search-heldout",
    "version": "2026-04-05",
    "slices": ["overall", "long-tail", "attribute-heavy"]
  },
  "seeds": [11, 17, 29],
  "trial_count": 3,
  "candidate": {
    "name": "candidate-b",
    "version": "v2"
  },
  "baseline": {
    "name": "incumbent-a",
    "version": "prod-2026-04-01"
  },
  "external_runs": [
    {
      "system": "mlflow",
      "run_id": "d1e2f3",
      "url": "..."
    }
  ]
}
```

### `summary.json`
Store the top-line comparison that the finding and decision will cite.

Recommended fields:
- `primary_metric`
- `secondary_metrics`
- `guardrails`
- `candidate`
- `baseline`
- `delta_absolute`
- `delta_relative`
- `decision_status`
- `method_valid`
- `notes`

Example:

```json
{
  "experiment_id": "E003",
  "primary_metric": {
    "name": "ndcg@10",
    "candidate": 0.472,
    "baseline": 0.443,
    "delta_absolute": 0.029,
    "delta_relative": 0.0655
  },
  "secondary_metrics": {
    "mrr@10": {
      "candidate": 0.511,
      "baseline": 0.498
    }
  },
  "guardrails": {
    "p95_latency_ms": {
      "candidate": 241,
      "baseline": 233,
      "status": "pass"
    }
  },
  "decision_status": "adopt-threshold-cleared",
  "method_valid": true,
  "notes": [
    "Held-out eval set",
    "3 trials",
    "No grader drift detected"
  ]
}
```

### `runs.csv`
Store one row per seed, trial, fold, or independent run.

Recommended columns:
- `experiment_id`
- `trial_id`
- `seed`
- `slice`
- `system_name`
- `metric_name`
- `metric_value`
- `unit`
- `baseline_name`
- `baseline_value`
- `delta_absolute`
- `delta_relative`
- `latency_ms`
- `cost_usd`
- `tokens_input`
- `tokens_output`
- `passed_guardrails`
- `external_run_id`

This file is the easiest place to compare repeated runs and to compute uncertainty later.

### `examples.jsonl`
Use when metrics are aggregated from examples, queries, or tasks.

Recommended fields per line:
- `example_id`
- `slice`
- `input_ref`
- `expected_ref`
- `candidate_output_ref`
- `baseline_output_ref`
- `grade`
- `grader_version`
- `notes`

Store large outputs as files in `artifacts/` and reference them from the JSONL rows instead of inlining huge payloads.

## Comparison rules

Always make comparisons easy:
- store both absolute values and deltas vs baseline
- store slice-level results, not only the overall average
- keep units explicit
- keep trial count explicit
- do not overwrite prior results silently
- do not store only screenshots or prose

## External trackers

If MLflow or W&B exists, log there too.

Record at minimum:
- tracking system name
- project / experiment name
- run ID
- URL
- model or artifact ID if available

Mirror the decisive summary back into `summary.json` and the experiment file so Limina can reason about it in later sessions without depending on the external UI.

## What belongs in `E` vs raw files

Put in `E`:
- decision question
- exact setup summary
- top-line metrics
- interpretation
- next step
- path to the raw metric directory under `kb/research/data/E###/`

Put in raw files:
- per-trial metrics
- per-example grades
- plots
- artifact paths
- large logs

## Minimal completion rule

Do not mark an experiment complete until:
- `manifest.json` exists
- `summary.json` exists
- at least one structured per-run file exists (`runs.csv` or `runs.jsonl`)
- the experiment file links to those files or at least names the data directory explicitly in `Results`
