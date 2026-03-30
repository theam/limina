# Literature Search — Structured Research Methodology

A systematic approach to finding, evaluating, and synthesizing sources for a research question. Use this whenever surveying the landscape for a research task.

## Workflow

### Phase 1: Query Formulation

Before searching, formulate your search strategy explicitly. Write it down:

1. **Core question**: What specifically are you trying to find? (not vague — "approaches to reranking in RAG" not "RAG stuff")
2. **Key terms**: List 5-10 specific terms, including synonyms and related concepts
3. **Exclusion terms**: What are you NOT looking for? (prevents rabbit holes)
4. **Known anchors**: Papers/authors/repos you already know that are relevant (use these as starting points for citation chains)
5. **Time scope**: How recent does the work need to be? (default: last 2 years for SOTA, any date for foundational work)

### Phase 2: Systematic Search (use all 4 channels)

Search is not "google it once." Use these channels in order:

**Channel 1 — Academic papers**: Search arXiv, Semantic Scholar, Google Scholar
- Use specific technical terms, not natural language queries
- Check the "cited by" and "references" of each relevant paper (snowball search)
- Prefer papers from the last 2 years unless looking for foundational work
- Check if papers have associated code/data repositories
- Look for survey papers — they compress months of reading into one document

**Channel 2 — Code & implementations**: Search GitHub, HuggingFace
- Search for implementations of approaches found in Channel 1
- Check stars, recency, and whether the project is actively maintained
- Read the README for limitations and failure modes the paper didn't mention
- Check issues/discussions for real-world problems with the approach
- Look for benchmark results in the repo

**Channel 3 — Practitioner knowledge**: Search blogs, tech talks, forums
- Search for "X in production" or "X at scale" — practitioners reveal failure modes papers hide
- Check HackerNews, Reddit (r/MachineLearning, r/LocalLLaMA), Twitter/X threads
- Look for post-mortems and case studies
- Prefer posts with benchmarks or production experience over opinion
- Conference talks (NeurIPS, ICML, ACL) often have practical insights not in the paper

**Channel 4 — Benchmarks & leaderboards**: Search for evaluation results
- Find the relevant benchmark for your domain
- Check what approaches are at the top AND what they sacrifice (latency, cost, complexity)
- Note which metrics are measured — what's missing?
- Check if the benchmark is still relevant or has been superseded

### Phase 3: Source Evaluation

For each source found, assess on 5 dimensions:

1. **Recency**: When was it published? Is it pre- or post-key developments in the field?
2. **Reproducibility**: Is there code? Can you verify the claims? Are the experimental details sufficient?
3. **Relevance**: Does it address YOUR specific setting (model size, data type, latency constraints), or a different one?
4. **Rigor**: Peer-reviewed? Multiple baselines? Ablation studies? Statistical significance reported? Multiple seeds?
5. **Bias**: Who funded it? Does the author work for a company selling the approach? Is there a conflict of interest?

**Red flags**: No comparison to baselines, cherry-picked metrics, single-seed experiments, claims without confidence intervals, "we outperform SOTA" without specifying which SOTA.

### Phase 4: Synthesis

After reviewing sources:

1. **Register everything**: Create a literature file (L{NUM}) for each significant source in `kb/research/literature/`
2. **Identify consensus**: What do most sources agree on?
3. **Identify contradictions**: Where do sources disagree? (these are often the most valuable insights — they reveal assumptions and boundary conditions)
4. **Identify gaps**: What has nobody studied? (these become hypotheses)
5. **Establish baselines**: What is the current best approach, and what are its metrics under conditions closest to ours?
6. **Rate confidence**: How confident are you in the landscape survey? "High" = 10+ sources across all 4 channels. "Medium" = 5-10 sources, some channels sparse. "Low" = fewer than 5 sources.

### Phase 5: Report

Send a structured report to the team lead:

```
Literature Search Report for T{NUM}

Query: {core question}
Sources reviewed: {count} across {channels covered}
Confidence: HIGH | MEDIUM | LOW

Top findings:
1. {most important finding with source reference}
2. {second finding}
3. {third finding}

Recommended baselines:
- {approach} — {metrics} — {source}

Contradictions found:
- {source A says X, source B says Y — possible explanation}

Gaps (potential hypotheses):
- {nobody has tested X with Y}

Literature files created: L{NUM}, L{NUM}, ...
```
