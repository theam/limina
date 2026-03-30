---
id: "IMP{NUM}"
type: implementation
status: "IN PROGRESS"
feature: "FT{NUM}"
task: "T{NUM}"
investigation: "INV{NUM}"
created: "{date}"
delivered: ""
tags: []
---

# IMP{NUM} — {Implementation Title}

> **Feature**: FT{NUM}
> **Task**: T{NUM}
> **Investigation**: INV{NUM}
> **Status**: IN PROGRESS | DELIVERED | NEEDS ITERATION
> **Started**: {date}
> **Delivered**: {date}

## Approach

_Chosen approach. Reference: INV{NUM}, DECISIONS.md entry #{NUM}._

## Code Location

_Where does this code live?_

- `src/...`
- ...

## Progress

_Update at every stopping point. After compaction, this is how you know where you left off._

- [ ] (`{date}`) {Step description}

## Technical Decisions

_Smaller decisions made during implementation (not big enough for DECISIONS.md)._

- ...

## Surprises

_Unexpected findings during implementation that shaped the approach. Not reusable lessons — local context for this task._

## Validation

_State as observable behavior: exact commands and expected outputs._

- `{test command}` → {expected result, e.g., "12 passed, 0 failed"}
- `{manual verification command}` → {expected output}
- [ ] Unit tests
- [ ] Integration tests
- [ ] Manual verification

## Known Limitations

_What doesn't work yet or was intentionally deferred?_

## Delivery Checklist

- [ ] Code complete
- [ ] Tests passing
- [ ] Documented (if user-facing)
- [ ] Deployed / merged
