---
aliases: ["DASHBOARD"]
type: dashboard
---

# Limina Dashboard

## Entry Points

- [[ACTIVE]]
- [[CHALLENGE]]

## Links

- Active State: [[ACTIVE]]
- Mission: [[CHALLENGE]]

## Active Working Set

```dataview
TABLE file.outlinks as "Outlinks"
FROM ""
WHERE file.name = "ACTIVE"
```

## Core Graph

```dataview
TABLE file.folder as "Folder", length(file.inlinks) as "In", length(file.outlinks) as "Out"
FROM ""
WHERE startswith(file.path, "research/") OR startswith(file.path, "reports/") OR startswith(file.path, "lessons/")
SORT file.name ASC
```

## Orphan Notes

```dataview
TABLE file.folder as "Folder"
FROM ""
WHERE file.name != "README"
AND file.name != "DASHBOARD"
AND length(file.inlinks) = 0
AND length(file.outlinks) = 0
SORT file.path ASC
```

## Notes Missing Outlinks

```dataview
TABLE file.folder as "Folder", length(file.inlinks) as "In"
FROM ""
WHERE file.name != "README"
AND file.name != "DASHBOARD"
AND length(file.outlinks) = 0
SORT file.path ASC
```

## Notes Missing Inlinks

```dataview
TABLE file.folder as "Folder", length(file.outlinks) as "Out"
FROM ""
WHERE file.name != "README"
AND file.name != "DASHBOARD"
AND length(file.inlinks) = 0
SORT file.path ASC
```
