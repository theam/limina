# Worked Example: Information Retrieval

Use this as a pattern when the problem is about retrieval, search quality, ranking, filtering, candidate generation, query understanding, RAG retrieval, or information access.

## User request
"Improve information retrieval for a production system. Prefer generalizable approaches, not narrow hacks."

## Step 1 — Frame the problem

### Likely objective
Improve retrieval quality in a way that transfers across query types and survives future product evolution.

### Likely success criteria
- recall@k
- nDCG / MRR
- robustness on long-tail queries
- out-of-domain performance
- latency and cost within budget
- reproducible offline-to-online correlation

### Likely constraints
- latency budget
- serving cost
- limited labeled relevance data
- domain shift
- noisy documents / metadata
- multilingual or mixed-format corpus
- deployment complexity

## Step 2 — Challenge map

### Slice: query intent understanding
- Symptom: relevant items are missed for ambiguous or underspecified queries
- Suspected root causes:
  - embeddings fail to preserve intent distinctions for hard query classes
  - query text lacks the information needed for direct matching
  - training data overrepresents easy head queries
- Why current systems fail:
  - lexical match is brittle
  - dense retrieval may blur fine-grained intent distinctions
  - reranking happens too late to recover missing candidates
- Mechanisms to search:
  - query reformulation
  - intent-aware encoders
  - contrastive representation learning
  - query decomposition
  - feedback-informed retrieval

### Slice: candidate generation quality
- Symptom: reranker cannot rescue results because good candidates never enter the pool
- Suspected root causes:
  - recall bottleneck in the first-stage retriever
  - embeddings poorly aligned to domain semantics
  - ANN / index trade-offs hurt recall
- Mechanisms to search:
  - sparse+dense hybrid retrieval
  - better representation learning
  - domain adaptation
  - training objective redesign
  - candidate expansion strategies

### Slice: filtering and ranking
- Symptom: top results include semantically related but task-irrelevant documents
- Suspected root causes:
  - score calibration mismatch
  - insufficient cross-encoder capacity
  - missing task-specific signals in ranking features
- Mechanisms to search:
  - reranking
  - calibration / uncertainty estimation
  - multi-stage ranking
  - distillation for efficient reranking

### Slice: evaluation mismatch
- Symptom: offline gains do not produce useful online gains
- Suspected root causes:
  - benchmark mismatch
  - relevance labels too shallow
  - overfitting to one query distribution
- Mechanisms to search:
  - benchmark redesign
  - hard-negative mining studies
  - robustness / out-of-domain evaluation
  - user-feedback-aware evaluation

## Step 3 — Mechanism families to search

Search at least these families:
- sparse retrieval
- dense retrieval
- hybrid sparse+dense retrieval
- late interaction methods
- reranking and cross-encoders
- query reformulation
- representation learning / contrastive learning
- domain adaptation
- calibration and uncertainty
- distillation for low-latency ranking
- data curation / hard negatives
- benchmark redesign and evaluation robustness

## Step 4 — Query families

### Direct task queries
- "dense retrieval domain shift long-tail queries"
- "hybrid retrieval production latency tradeoff"
- "reranking hard negatives information retrieval"

### Mechanism queries
- "contrastive representation learning retrieval generalization"
- "calibration for ranking uncertainty retrieval"
- "retrieval distillation low latency reranker"

### Failure queries
- "limitations of dense retrieval under domain shift"
- "when hybrid retrieval fails"
- "offline online mismatch retrieval evaluation"

### Adjacent-field queries
- "recommender systems ranking calibration"
- "query understanding web search intent modeling"
- "representation learning hard negatives metric learning"

## Step 5 — What the synthesis should look like

A strong answer should not end with "read these 12 papers."
It should say something like:
- first-stage recall is likely the critical path,
- hybrid or better representations are more general than narrow business-rule filters,
- reranking helps only after candidate generation is fixed,
- small calibration or evaluation redesign gains may compose with retrieval gains,
- and the cheapest useful experiment is a controlled comparison across hybrid retrieval, improved encoders, and efficient reranking under realistic latency constraints.

## Example recommendation pattern

### Now
- Compare the current retriever against a hybrid baseline and a stronger representation-learning variant on the same candidate-generation budget.
- Run one robustness slice specifically on ambiguous and long-tail queries.

### Next
- Evaluate whether a compact reranker adds value once candidate recall improves.
- Audit label quality and hard-negative construction.

### Explore
- Intent-aware query reformulation or decomposition if ambiguity is a major failure mode.
- Calibration-aware ranking if confidence and filtering are weak.

### Avoid / Deprioritize
- Narrow heuristic patches that only fix one query subtype unless they are being used as a short-term stopgap.
- Tiny leaderboard gains with large serving cost and no sign of transfer.

## How to persist this in Limina

After the landscape pass:
- write the serious external sources as `L` notes
- update `kb/ACTIVE.md` with the current working set
- open or revise a hypothesis if the mechanism map points to a concrete test
- open a `CR` or `SR` only if the search invalidates the current direction or demands a reset
