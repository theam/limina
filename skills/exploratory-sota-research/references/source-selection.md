# Source Selection Policy

Use source types intentionally rather than uniformly.

## Tier 1 — Canonical high-signal sources

Use these first for trustworthy coverage:
- arXiv for frontier discovery and recent preprints
- OpenReview for papers plus public reviews when available
- major conference proceedings relevant to the field
- JMLR / TMLR for durable and often more mature work
- ACL Anthology for NLP / LLM / speech / language tasks
- other field-specific top venues when directly relevant

Primary use:
- initial map of mechanism families,
- recent top papers,
- seminal references,
- surveys,
- benchmark papers,
- reviewer discussion when available.

## Tier 2 — Discovery and citation infrastructure

Use these to widen and organize the map:
- Semantic Scholar
- DBLP
- OpenAlex
- Crossref
- citation graphs and backward / forward citation chasing

Primary use:
- finding clusters of work,
- author and venue discovery,
- tracing influential lines,
- locating follow-up or contradictory work,
- surfacing survey hubs.

## Tier 3 — Realism and reproducibility sources

Use these to test whether a direction is practical:
- official repositories
- benchmark suites
- artifact pages
- reproducibility programs
- official leaderboards
- code release pages

Primary use:
- code availability,
- maturity of artifacts,
- standardization of evaluation,
- ease of reproduction,
- realism of benchmark setup.

## Tier 4 — Peripheral but plausible sources

Use carefully and as secondary evidence:
- workshop papers,
- theses,
- technical reports,
- reputable industry research blogs,
- implementation notes by strong research groups,
- adjacent-domain papers with transferable mechanisms.

Primary use:
- discovering emerging mechanisms,
- finding deeper implementation details,
- borrowing ideas from neighboring fields.

Always trace important claims back to primary evidence when possible.

## Tier 5 — Weak-signal sources

Use only for lead generation:
- social posts,
- generic blogs,
- low-rigor summaries,
- listicles.

Do not treat these as primary evidence.

## What to prioritize

Prefer work that has some combination of:
- strong baselines,
- meaningful gains,
- realistic evaluation,
- transparent limitations,
- public code or artifacts,
- signs of transfer beyond one benchmark,
- evidence of influence or reuse.

## What to deprioritize

Deprioritize work that is mostly:
- benchmark gaming,
- weakly controlled comparisons,
- tiny gains with heavy cost,
- no reproducibility signals,
- strong claims with narrow evaluation,
- or obvious domain-specific tricks presented as general solutions.

## Source mixing rule

For each important mechanism cluster, try to combine:
- one canonical or survey-like source,
- one recent frontier source,
- one limitation / failure source,
- and one realism / artifact source.

That mix is usually more informative than reading four papers that all agree.
