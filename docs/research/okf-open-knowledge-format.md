# OKF — Open Knowledge Format: what it actually is

Research note for **reeve**. The user recalled reading "a paper from Google" introducing something
called **OKF — Open Knowledge Format**, described as a new framework for documentation
("docs as knowledge").

Question: *does a credible Google primary source for "Open Knowledge Format" exist, and if so, what
does it actually specify?*

**Bottom line: OKF is real, and it is from Google — but it is not a paper, and it is not exactly a
documentation framework.** It is an open *specification* announced on the Google Cloud Blog on
2026-06-12 and published on GitHub, defining a vendor-neutral format for "knowledge bundles":
directories of Markdown files with YAML frontmatter, designed as curated context for AI agents.
The "docs as knowledge" recollection is a reasonable paraphrase of its "LLM-wiki" framing, but its
center of gravity is enterprise data/metadata context (BigQuery tables, metrics, playbooks), not
software project documentation. No arXiv or research.google paper exists as of this writing.

---

## 1. Primary sources

- **Announcement:** "How the Open Knowledge Format can improve data sharing", Google Cloud Blog,
  2026-06-12, by Sam McVeety (Tech Lead, Data Analytics) and Amir Hormati (Tech Lead, BigQuery)
  ([cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing)).
- **Spec:** `okf/SPEC.md` in the `GoogleCloudPlatform/knowledge-catalog` repo, currently at v0.2
  ([github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)).
- **Reference tooling and samples:** same repo — an enrichment agent that drafts OKF documents from
  BigQuery datasets, a static HTML graph visualizer, and three sample bundles (GA4 e-commerce,
  Stack Overflow, Bitcoin public datasets)
  ([github.com/GoogleCloudPlatform/knowledge-catalog/tree/main/okf](https://github.com/GoogleCloudPlatform/knowledge-catalog/tree/main/okf)).

So the correct citation is "a Google Cloud spec + blog post", not "a Google paper". A search for an
arXiv or research.google publication under this name returns only the blog post and third-party
coverage; there is no academic paper.

## 2. What the spec actually says (v0.2)

An **OKF bundle** is a directory tree of UTF-8 Markdown files. Each non-reserved `.md` file is a
**concept** — a table, dataset, metric, API endpoint, playbook, runbook — with two parts: YAML
frontmatter and a Markdown body. The design is deliberately minimal:

- **One required field:** `type` (free-form string; no fixed taxonomy — e.g. `BigQuery Table`,
  `Metric`, `Playbook`, `Attested Computation`).
- **Recommended fields:** `title`, `description`, `resource` (canonical URI of the underlying
  asset), `tags`.
- **Trust and lifecycle fields (new in v0.2):** `generated: {by, at}`, `verified: [{by, at}]`,
  `status: draft|stable|deprecated`, `stale_after`, and `sources` with credibility signals. An
  actor convention distinguishes agents (`producer/version`), people (`human:<id>`), and
  automation (`process:<id>`). Verification level derives trust tiers: unverified →
  machine-confirmed → human-reviewed.
- **Reserved files:** `index.md` (directory listing, one bullet per concept with description) and
  `log.md` (reverse-chronological change history grouped by date).
- **Linking:** ordinary Markdown links between concepts (bundle-absolute paths recommended), which
  is what turns a flat directory into a navigable knowledge graph.
- **Conformance is permissive:** consumers must not reject a bundle for unknown types, extra
  frontmatter keys, broken links, or missing indexes. "No SDK, runtime, or registry."

Explicit design principles from the announcement: minimally opinionated; producer/consumer
independence; "a format, not a platform". Google Cloud's Knowledge Catalog ingests OKF and serves
it to agents; adoption beyond Google is nascent — e.g. the independent `openknowledge-sh/openknowledge`
CLI/runtime implementing OKF v0.1 with Claude Code / Codex / OpenCode integration, Apache-2.0,
~25 stars as of 2026-07 ([github.com/openknowledge-sh/openknowledge](https://github.com/openknowledge-sh/openknowledge)).

## 3. Identity check — things OKF is *not*

Similar names that a search (or a memory) can conflate:

- **Open Knowledge Foundation** (also historically abbreviated OKF/OKFN, [okfn.org](https://okfn.org)) —
  the open-data nonprofit behind CKAN and **Frictionless Data** ([frictionlessdata.io](https://frictionlessdata.io),
  Data Packages: JSON descriptors for datasets). Unrelated to Google; the most likely source of
  abbreviation confusion.
- **Google Knowledge Graph / schema.org** — structured data for search, not an agent-context format.
- **llms.txt** ([llmstxt.org](https://llmstxt.org)), **AGENTS.md** ([agents.md](https://agents.md)),
  **MCP**, **Agent2Agent** — adjacent agent-context conventions and protocols, none of which are OKF,
  though llms.txt/AGENTS.md are its closest conceptual relatives (see §4).

## 4. Relevance to reeve's documentation practice

OKF is, in effect, a formalization of what this repo already does informally: agent-oriented
knowledge as plain Markdown in git. The mapping is direct —

| reeve practice | OKF equivalent |
|---|---|
| `CONTEXT.md` glossary | Concept documents (`type: Definition`-style) |
| `docs/adr/` | Concepts with lifecycle (`status`, dated history) |
| LLD atlas / doc atlases | `index.md` directory listings linking concepts |
| Research notes citing sources | `sources` frontmatter with provenance |
| Git history as audit trail | `log.md` + `generated`/`verified` trust fields |

Ideas worth stealing even without adopting the format: (1) one required `type` field in frontmatter
makes a doc corpus machine-queryable at near-zero authoring cost; (2) `stale_after` and the
unverified → machine-confirmed → human-reviewed trust ladder are a lightweight answer to "can an
agent trust this doc?"; (3) permissive-consumer conformance keeps tooling from ossifying the docs.
Adopting OKF wholesale is premature — v0.2, weeks old, data-catalog-centric examples — but it is
the first vendor-neutral spec pointing at the same "docs as agent context" territory as
`CONTEXT.md`/AGENTS.md/llms.txt, and worth tracking.

## 5. Search trail

Queries: `"Open Knowledge Format" Google paper documentation`; `"Open Knowledge Format" OKF spec`;
`arXiv "Open Knowledge Format" Google OKF paper`. First two immediately surfaced the Google Cloud
blog post and the GitHub spec plus secondary coverage (heise, MarkTechPost, Search Engine Journal,
multiple Medium posts, all June–July 2026). The arXiv-focused query surfaced no paper. Confidence:
**high** that the sources above are the primary sources, and **high** that no academic paper exists
under this name as of 2026-07-27.
