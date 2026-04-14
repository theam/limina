#!/usr/bin/env python3
"""
Extract references and verifiable claims from markdown drafts.

Parses ## References sections and inline claims (numbers, quotes,
attributions) from markdown files. Outputs two CSVs:
  - references/sources.csv  (one row per unique source)
  - references/claims.csv   (one row per verifiable claim)

Stdlib-only. No external dependencies.
"""

from __future__ import annotations

import argparse
import csv
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------

@dataclass
class Source:
    source_id: str
    source_name: str
    title: str
    year: str
    url: str
    url_type: str
    draft_files: list[str] = field(default_factory=list)
    lit_file: str = ""
    fetch_status: str = "PENDING"


@dataclass
class Claim:
    claim_id: str
    source_id: str
    draft_file: str
    line_number: int
    claim_type: str
    claim_text: str
    extracted_assertion: str
    verification_status: str = "PENDING"
    source_evidence: str = ""
    discrepancy_detail: str = ""


# ---------------------------------------------------------------------------
# Reference parsing
# ---------------------------------------------------------------------------

# Matches: - Author. "Title." ... [label](url)
# Also:    - "Title." ... [label](url)
# Also:    - Org. "Title." ... (no url)
REF_LINE_RE = re.compile(
    r'^-\s+'
    r'(?P<author_part>.*?)'     # everything before the title
    r'"(?P<title>[^"]+)"'       # "Title"
    r'(?P<after_title>.*?)'     # venue, year, etc.
    r'$',
    re.DOTALL,
)

URL_RE = re.compile(r'\[([^\]]*)\]\((?P<url>[^)]+)\)')
YEAR_RE = re.compile(r'\b(1[89]\d{2}|20[0-3]\d)\b')

# Inline claim patterns
PERCENT_RE = re.compile(
    r'(?P<context>[^.;]*?)'
    r'(?P<number>\d+(?:\.\d+)?%)'
    r'(?P<after>[^.;]*)',
)
RATIO_RE = re.compile(r'(\d+:\d+)')
MULTIPLIER_RE = re.compile(r'(\d+(?:\.\d+)?[-–]\d+(?:\.\d+)?x|\d+(?:\.\d+)?x)')
QUOTE_RE = re.compile(r'["\u201c](?P<quote>[^"\u201d]{15,})["\u201d]')
ATTRIBUTION_RE = re.compile(
    r'(?P<who>[A-Z][a-z]+(?:\s+(?:et\s+al\.?|and\s+[A-Z][a-z]+))?'
    r'|[A-Z][a-z]+\.ai|IronCore\s+Labs|Microsoft|Gartner|IBM|OWASP|NIST|Deloitte'
    r'|Cleanlab|Silverfort|Obsidian\s+Security|Help\s+Net\s+Security|Proofpoint'
    r'|CSA|Cloud\s+Security\s+Alliance|Tonic\.ai|CyberArk|Glean|NStarX'
    r'|LangChain|Anthropic|Salesforce|Permit\.io|Snowflake|Fortanix|Datavant'
    r'|UC\s+Berkeley|Google\s+Research|Google\s+DeepMind|Meta'
    r'|Kim\s+et\s+al\.?|Morris\s+et\s+al\.?|Huang\s+et\s+al\.?|Zhang\s+et\s+al\.?'
    r'|Zhu\s+et\s+al\.?|Fan\s+et\s+al\.?|Edge\s+et\s+al\.?|Gupta\s+et\s+al\.?'
    r'|Sarthi\s+et\s+al\.?|Rezazadeh\s+et\s+al\.?|Karpathy)'
    r'\s+(?P<verb>found|demonstrated|showed|achieved|reported|predicted|estimated'
    r'|published|released|proposed|introduced|developed|validated|recovered'
    r'|identifies|calls|emphasizes|extends|addresses|recommends|requires'
    r'|describes|surveyed|discovered|proved|noted|observed|measured'
    r'|argues|claims|contends|warns|concluded|designed|built|created'
    r'|integrated|deployed|tested|verified|confirmed|rejected|debunked)\s+'
    r'(?P<what>.+?)(?:\.|$)',
    re.IGNORECASE,
)


def classify_url(url: str) -> str:
    """Classify a URL into a type for fetch strategy."""
    if not url:
        return "none"
    if "arxiv.org" in url:
        return "arxiv"
    if "aclanthology.org" in url:
        return "acl"
    if "openreview.net" in url:
        return "openreview"
    if "usenix.org" in url:
        return "usenix"
    if "dl.acm.org" in url:
        return "acm"
    if "springer.com" in url:
        return "springer"
    if "vldb.org" in url:
        return "vldb"
    if "x.com" in url or "twitter.com" in url:
        return "x.com"
    if url.endswith(".pdf"):
        return "pdf"
    return "web"


def normalize_title(title: str) -> str:
    """Normalize a title for deduplication."""
    return re.sub(r'\s+', ' ', title.strip().lower().rstrip('.'))


def parse_ref_line(line: str) -> dict | None:
    """Parse a single reference line into components."""
    line = line.strip()
    if not line.startswith('-'):
        return None

    # Extract title in quotes
    title_match = re.search(r'["\u201c]([^"\u201d]+)["\u201d]', line)
    if not title_match:
        return None

    title = title_match.group(1)
    before_title = line[2:title_match.start()].strip()  # skip "- "
    after_title = line[title_match.end():]

    # Extract URL
    url = ""
    url_match = URL_RE.search(after_title)
    if url_match:
        url = url_match.group("url")

    # Extract year
    year = ""
    year_match = YEAR_RE.search(after_title)
    if year_match:
        year = year_match.group(1)
    elif YEAR_RE.search(before_title):
        year = YEAR_RE.search(before_title).group(1)

    # Author/source name
    source_name = before_title.rstrip('.')
    if not source_name:
        # Title-first format: use first significant word of title
        source_name = title.split(':')[0].split('—')[0].strip()

    return {
        "source_name": source_name,
        "title": title,
        "year": year,
        "url": url,
        "url_type": classify_url(url),
    }


def find_literature_file(title: str, lit_dir: Path) -> str:
    """Find a matching literature file for a source title."""
    if not lit_dir.exists():
        return ""
    norm = normalize_title(title)
    for path in lit_dir.glob("L*.md"):
        text = path.read_text(encoding="utf-8")
        # Check if the title appears in the literature file
        if norm in normalize_title(text[:2000]):
            return path.name
    # Fallback: check if key words from the title appear
    key_words = [w for w in norm.split() if len(w) > 5]
    for path in lit_dir.glob("L*.md"):
        text = normalize_title(path.read_text(encoding="utf-8")[:2000])
        if sum(1 for w in key_words if w in text) >= max(2, len(key_words) // 2):
            return path.name
    return ""


def extract_references(drafts_dir: Path) -> tuple[list[Source], dict[str, str]]:
    """Extract all references from draft files. Returns (sources, title_to_id)."""
    sources: list[Source] = []
    title_to_id: dict[str, str] = {}  # normalized_title -> source_id
    source_counter = 0

    lit_dir = drafts_dir.parent / "kb" / "research" / "literature"

    draft_files = sorted(drafts_dir.glob("*.md"))
    for draft_path in draft_files:
        text = draft_path.read_text(encoding="utf-8")
        lines = text.splitlines()
        draft_name = draft_path.stem  # e.g., "01-embeddings-are-not-private"
        draft_num = draft_name.split('-')[0] if draft_name[0].isdigit() else draft_name

        # Find ## References section
        in_refs = False
        for line in lines:
            stripped = line.strip()
            if stripped == "## References":
                in_refs = True
                continue
            if in_refs and stripped.startswith("## "):
                break
            if not in_refs:
                continue
            if not stripped.startswith('-'):
                continue

            parsed = parse_ref_line(stripped)
            if not parsed:
                continue

            norm_title = normalize_title(parsed["title"])

            if norm_title in title_to_id:
                # Existing source — add this draft
                sid = title_to_id[norm_title]
                for s in sources:
                    if s.source_id == sid:
                        if draft_num not in s.draft_files:
                            s.draft_files.append(draft_num)
                        # Update URL if this ref has one and the existing doesn't
                        if parsed["url"] and not s.url:
                            s.url = parsed["url"]
                            s.url_type = parsed["url_type"]
                        break
            else:
                source_counter += 1
                sid = f"S{source_counter:03d}"
                title_to_id[norm_title] = sid
                lit_file = find_literature_file(parsed["title"], lit_dir)
                sources.append(Source(
                    source_id=sid,
                    source_name=parsed["source_name"],
                    title=parsed["title"],
                    year=parsed["year"],
                    url=parsed["url"],
                    url_type=parsed["url_type"],
                    draft_files=[draft_num],
                    lit_file=lit_file,
                ))

    return sources, title_to_id


# ---------------------------------------------------------------------------
# Claim extraction
# ---------------------------------------------------------------------------

# Pattern for inline parenthetical citations: (Author, Year) or (Author Year)
INLINE_CITE_RE = re.compile(
    r'\(([^()]{2,60}?),?\s*(\d{4})\)'
)


def _find_inline_citations(text: str, sources: list[Source]) -> list[tuple[int, int, str]]:
    """Find parenthetical citations like (Cleanlab, 2025). Returns [(start, end, source_id)]."""
    results = []
    for m in INLINE_CITE_RE.finditer(text):
        cite_name = m.group(1).strip()
        cite_year = m.group(2)
        cite_lower = cite_name.lower()

        # Skip affiliations — real citations are short (1-4 words)
        if len(cite_name.split()) > 5:
            continue

        best_source = None
        best_score = 0
        for source in sources:
            s_name = source.source_name.lower()
            s_first = s_name.split(',')[0].split()[0] if s_name else ""
            if not s_first or len(s_first) < 3:
                continue

            score = 0
            # Full name match is strongest
            if s_name in cite_lower:
                score = 3
            # First word/surname match
            elif re.search(r'\b' + re.escape(s_first) + r'\b', cite_lower):
                score = 2
            # Partial match (substring) — weak
            elif s_first in cite_lower:
                score = 1

            if score == 0:
                continue

            # Year match bonus
            if source.year == cite_year:
                score += 2

            if score > best_score:
                best_score = score
                best_source = source.source_id

        if best_source and best_score >= 3:  # require at least name match + year match
            results.append((m.start(), m.end(), best_source))
    return results


def _find_author_mentions(text: str, sources: list[Source]) -> list[tuple[int, str]]:
    """Find author name mentions in text. Returns [(char_position, source_id)].
    Uses word-boundary matching to avoid false positives."""
    matches = []
    text_lower = text.lower()
    for source in sources:
        name = source.source_name
        if not name or len(name) < 3:
            continue
        name_lower = name.lower()
        # Try full name (exact substring) — only for distinctive names (6+ chars)
        if len(name_lower) >= 6:
            pos = text_lower.find(name_lower)
            if pos >= 0:
                matches.append((pos, source.source_id))
                continue
        # For "et al." authors, look for "Surname et al" pattern first
        is_et_al = "et al" in name_lower
        first_word = name.split(',')[0].split()[0] if name else ""
        if is_et_al and first_word and len(first_word) >= 3:
            # Look for "Kim et al" pattern — very specific, low false positive
            et_al_pat = re.compile(
                r'\b' + re.escape(first_word.lower()) + r'\s+et\s+al',
                re.IGNORECASE,
            )
            m = et_al_pat.search(text_lower)
            if m:
                matches.append((m.start(), source.source_id))
                continue
        # Try surname with word boundary — must be 5+ chars to avoid noise
        if first_word and len(first_word) >= 5:
            pattern = re.compile(r'\b' + re.escape(first_word.lower()) + r'\b')
            m = pattern.search(text_lower)
            if m:
                matches.append((m.start(), source.source_id))
    return sorted(matches, key=lambda x: x[0])


def find_nearest_source(line: str, line_num: int, lines: list[str],
                        title_to_id: dict[str, str],
                        sources: list[Source],
                        claim_char_pos: int = -1) -> str:
    """Find the source_id most likely attributed to a claim on a given line.

    Strategy priority:
    1. Parenthetical citation nearest the claim: (Author, Year)
    2. Author name mention nearest the claim on the same line
    3. Author name mention in surrounding lines
    4. Title keyword match in surrounding lines
    """
    # Strategies 1+2 combined: collect all candidates on the current line,
    # then pick the one closest to the claim position.
    candidates: list[tuple[int, str]] = []  # (position, source_id)

    # Author mentions (e.g., "Kim et al.", "Morris et al.")
    line_mentions = _find_author_mentions(line, sources)
    for pos, sid in line_mentions:
        candidates.append((pos, sid))

    # Parenthetical citations (e.g., "(Cleanlab, 2025)")
    citations = _find_inline_citations(line, sources)
    for start, end, sid in citations:
        candidates.append((start, sid))

    if candidates and claim_char_pos >= 0:
        best = None
        best_dist = float('inf')
        for pos, sid in candidates:
            # Prefer mentions/citations near or just after the claim
            if pos >= claim_char_pos:
                dist = pos - claim_char_pos
            else:
                dist = (claim_char_pos - pos) * 2
            if dist < best_dist:
                best_dist = dist
                best = sid
        if best:
            return best
    elif candidates:
        return candidates[0][1]

    # Strategy 3: Check surrounding lines
    search_range = lines[max(0, line_num - 5):line_num + 3]
    search_text = ' '.join(search_range)

    # Try citations first in surrounding text
    nearby_cites = _find_inline_citations(search_text, sources)
    if nearby_cites:
        return nearby_cites[-1][2]  # last citation is usually most relevant

    nearby_mentions = _find_author_mentions(search_text, sources)
    if nearby_mentions:
        return nearby_mentions[0][1]

    # Strategy 4: Try matching title keywords in nearby text
    for norm_title, sid in title_to_id.items():
        words = [w for w in norm_title.split() if len(w) > 5]
        if words and sum(1 for w in words if w in search_text.lower()) >= min(2, len(words)):
            return sid

    return ""


def extract_claims(drafts_dir: Path, sources: list[Source],
                   title_to_id: dict[str, str]) -> list[Claim]:
    """Extract verifiable claims from draft body text."""
    claims: list[Claim] = []
    claim_counter = 0
    seen_claims: set[str] = set()  # dedup key: (draft, claim_text_normalized)

    draft_files = sorted(drafts_dir.glob("*.md"))
    for draft_path in draft_files:
        text = draft_path.read_text(encoding="utf-8")
        lines = text.splitlines()
        draft_name = draft_path.name

        # Skip the references section for claim extraction
        ref_start = len(lines)
        for i, line in enumerate(lines):
            if line.strip() == "## References":
                ref_start = i
                break

        for line_num, line in enumerate(lines[:ref_start], start=1):
            stripped = line.strip()
            if not stripped or stripped.startswith('#') or stripped.startswith('```'):
                continue
            # Skip table header/separator rows
            if stripped.startswith('|') and ('---' in stripped or 'Attack' in stripped
                                             or 'Level' in stripped or 'Description' in stripped):
                continue

            # --- Extract percentage claims ---
            for m in PERCENT_RE.finditer(stripped):
                number = m.group('number')
                context = m.group('context') + number + m.group('after')
                context = context.strip().strip(',').strip()
                if len(context) < 10:
                    continue

                dedup_key = f"{draft_name}:{number}:{context[:50]}"
                if dedup_key in seen_claims:
                    continue
                seen_claims.add(dedup_key)

                source_id = find_nearest_source(stripped, line_num - 1, lines,
                                                title_to_id, sources,
                                                claim_char_pos=m.start())
                claim_counter += 1
                claims.append(Claim(
                    claim_id=f"C{claim_counter:03d}",
                    source_id=source_id,
                    draft_file=draft_name,
                    line_number=line_num,
                    claim_type="number",
                    claim_text=context[:200],
                    extracted_assertion=f"The source reports {number} in context: {context[:150]}",
                ))

            # --- Extract ratio claims (50:1) ---
            for m in RATIO_RE.finditer(stripped):
                ratio = m.group(1)
                context_start = max(0, m.start() - 60)
                context_end = min(len(stripped), m.end() + 60)
                context = stripped[context_start:context_end].strip()

                dedup_key = f"{draft_name}:{ratio}"
                if dedup_key in seen_claims:
                    continue
                seen_claims.add(dedup_key)

                source_id = find_nearest_source(stripped, line_num - 1, lines,
                                                title_to_id, sources,
                                                claim_char_pos=m.start())
                claim_counter += 1
                claims.append(Claim(
                    claim_id=f"C{claim_counter:03d}",
                    source_id=source_id,
                    draft_file=draft_name,
                    line_number=line_num,
                    claim_type="number",
                    claim_text=context[:200],
                    extracted_assertion=f"The source reports ratio {ratio} in context: {context[:150]}",
                ))

            # --- Extract multiplier claims (2-6x) ---
            for m in MULTIPLIER_RE.finditer(stripped):
                mult = m.group(1)
                context_start = max(0, m.start() - 60)
                context_end = min(len(stripped), m.end() + 60)
                context = stripped[context_start:context_end].strip()

                dedup_key = f"{draft_name}:{mult}"
                if dedup_key in seen_claims:
                    continue
                seen_claims.add(dedup_key)

                source_id = find_nearest_source(stripped, line_num - 1, lines,
                                                title_to_id, sources,
                                                claim_char_pos=m.start())
                claim_counter += 1
                claims.append(Claim(
                    claim_id=f"C{claim_counter:03d}",
                    source_id=source_id,
                    draft_file=draft_name,
                    line_number=line_num,
                    claim_type="number",
                    claim_text=context[:200],
                    extracted_assertion=f"The source reports {mult} in context: {context[:150]}",
                ))

            # --- Extract quoted phrases (15+ chars) ---
            for m in QUOTE_RE.finditer(stripped):
                quote = m.group('quote')
                # Skip quotes that are section titles or UI text
                if quote.startswith('#') or len(quote) > 500:
                    continue

                dedup_key = f"{draft_name}:quote:{quote[:40]}"
                if dedup_key in seen_claims:
                    continue
                seen_claims.add(dedup_key)

                source_id = find_nearest_source(stripped, line_num - 1, lines,
                                                title_to_id, sources,
                                                claim_char_pos=m.start())
                claim_counter += 1
                claims.append(Claim(
                    claim_id=f"C{claim_counter:03d}",
                    source_id=source_id,
                    draft_file=draft_name,
                    line_number=line_num,
                    claim_type="quote",
                    claim_text=quote[:200],
                    extracted_assertion=f'Verbatim quote from source: "{quote[:150]}"',
                ))

            # --- Extract attribution claims ---
            for m in ATTRIBUTION_RE.finditer(stripped):
                who = m.group('who')
                verb = m.group('verb')
                what = m.group('what').strip()
                if len(what) < 10:
                    continue

                dedup_key = f"{draft_name}:attr:{who}:{what[:30]}"
                if dedup_key in seen_claims:
                    continue
                seen_claims.add(dedup_key)

                source_id = find_nearest_source(stripped, line_num - 1, lines,
                                                title_to_id, sources,
                                                claim_char_pos=m.start())
                claim_counter += 1
                claims.append(Claim(
                    claim_id=f"C{claim_counter:03d}",
                    source_id=source_id,
                    draft_file=draft_name,
                    line_number=line_num,
                    claim_type="attribution",
                    claim_text=f"{who} {verb} {what}"[:200],
                    extracted_assertion=f"{who} {verb} {what}"[:200],
                ))

    return claims


# ---------------------------------------------------------------------------
# Table-row claim extraction
# ---------------------------------------------------------------------------

TABLE_ROW_RE = re.compile(r'^\|(.+)\|$')


def extract_table_claims(drafts_dir: Path, sources: list[Source],
                         title_to_id: dict[str, str],
                         existing_claims: list[Claim]) -> list[Claim]:
    """Extract claims from markdown table rows (e.g., comparison tables)."""
    claims: list[Claim] = []
    claim_counter = len(existing_claims)
    seen = {(c.draft_file, c.claim_text) for c in existing_claims}

    draft_files = sorted(drafts_dir.glob("*.md"))
    for draft_path in draft_files:
        text = draft_path.read_text(encoding="utf-8")
        lines = text.splitlines()
        draft_name = draft_path.name

        ref_start = len(lines)
        for i, line in enumerate(lines):
            if line.strip() == "## References":
                ref_start = i
                break

        for line_num, line in enumerate(lines[:ref_start], start=1):
            stripped = line.strip()
            m = TABLE_ROW_RE.match(stripped)
            if not m:
                continue
            cells = [c.strip() for c in m.group(1).split('|')]
            # Skip header/separator rows
            if any(c.startswith('---') for c in cells):
                continue
            if any(c in ('Attack', 'Level', 'Description') for c in cells):
                continue

            row_text = ' | '.join(cells)
            if not any(c for c in cells if PERCENT_RE.search(c) or RATIO_RE.search(c)):
                continue

            dedup_key = (draft_name, row_text[:80])
            if dedup_key in seen:
                continue
            seen.add(dedup_key)

            source_id = find_nearest_source(stripped, line_num - 1, lines,
                                            title_to_id, sources)
            claim_counter += 1
            claims.append(Claim(
                claim_id=f"C{claim_counter:03d}",
                source_id=source_id,
                draft_file=draft_name,
                line_number=line_num,
                claim_type="number",
                claim_text=row_text[:200],
                extracted_assertion=f"Table row claims: {row_text[:180]}",
            ))

    return claims


# ---------------------------------------------------------------------------
# CSV I/O
# ---------------------------------------------------------------------------

SOURCE_FIELDS = [
    "source_id", "source_name", "title", "year", "url", "url_type",
    "draft_files", "lit_file", "fetch_status",
]

CLAIM_FIELDS = [
    "claim_id", "source_id", "draft_file", "line_number", "claim_type",
    "claim_text", "extracted_assertion", "verification_status",
    "source_evidence", "discrepancy_detail",
]


def write_sources_csv(sources: list[Source], path: Path) -> None:
    with open(path, 'w', newline='', encoding='utf-8') as f:
        writer = csv.DictWriter(f, fieldnames=SOURCE_FIELDS)
        writer.writeheader()
        for s in sources:
            writer.writerow({
                "source_id": s.source_id,
                "source_name": s.source_name,
                "title": s.title,
                "year": s.year,
                "url": s.url,
                "url_type": s.url_type,
                "draft_files": ",".join(s.draft_files),
                "lit_file": s.lit_file,
                "fetch_status": s.fetch_status,
            })


def write_claims_csv(claims: list[Claim], path: Path) -> None:
    with open(path, 'w', newline='', encoding='utf-8') as f:
        writer = csv.DictWriter(f, fieldnames=CLAIM_FIELDS)
        writer.writeheader()
        for c in claims:
            writer.writerow({
                "claim_id": c.claim_id,
                "source_id": c.source_id,
                "draft_file": c.draft_file,
                "line_number": c.line_number,
                "claim_type": c.claim_type,
                "claim_text": c.claim_text,
                "extracted_assertion": c.extracted_assertion,
                "verification_status": c.verification_status,
                "source_evidence": c.source_evidence,
                "discrepancy_detail": c.discrepancy_detail,
            })


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract references and claims from markdown drafts."
    )
    parser.add_argument(
        "drafts_dir",
        help="Directory containing markdown draft files",
    )
    parser.add_argument(
        "--output", "-o",
        default="references",
        help="Output directory for CSVs (default: references/)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    drafts_dir = Path(args.drafts_dir).resolve()
    output_dir = Path(args.output).resolve()

    if not drafts_dir.is_dir():
        print(f"Error: {drafts_dir} is not a directory", file=sys.stderr)
        return 1

    output_dir.mkdir(parents=True, exist_ok=True)

    # Phase 1: Extract references
    print(f"Scanning drafts in {drafts_dir}...")
    sources, title_to_id = extract_references(drafts_dir)
    print(f"  Found {len(sources)} unique sources across {len(list(drafts_dir.glob('*.md')))} drafts")

    # Phase 2: Extract inline claims
    print("Extracting inline claims...")
    claims = extract_claims(drafts_dir, sources, title_to_id)
    print(f"  Found {len(claims)} inline claims")

    # Phase 3: Extract table claims
    table_claims = extract_table_claims(drafts_dir, sources, title_to_id, claims)
    claims.extend(table_claims)
    print(f"  Found {len(table_claims)} table claims ({len(claims)} total)")

    # Phase 4: Stats by type
    by_type: dict[str, int] = {}
    for c in claims:
        by_type[c.claim_type] = by_type.get(c.claim_type, 0) + 1
    for ct, count in sorted(by_type.items()):
        print(f"    {ct}: {count}")

    # Phase 5: Stats by source attribution
    attributed = sum(1 for c in claims if c.source_id)
    print(f"  Attributed to source: {attributed}/{len(claims)}")

    # Phase 6: Write CSVs
    sources_path = output_dir / "sources.csv"
    claims_path = output_dir / "claims.csv"
    write_sources_csv(sources, sources_path)
    write_claims_csv(claims, claims_path)
    print(f"\nWritten:")
    print(f"  {sources_path}")
    print(f"  {claims_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
