# `dijkstral context`: classifier reranking (research prototype)

> The prototype (an external reranker hook in `context::Scan::rank` and the scripts in
> `bench/agent/rerank/`) was not merged; it lives on branch
> `feature/dijkstral-context-rerank-prototype` (`3669587`). This note records the measurements.

A measured exploration, not a shipped feature. `dijkstral context` (`dijkstral-context.md`) ranks
the definitions whose code and comments match a question lexically (§1, code text). This
prototype asks a hosted classifier, TypeSafe AI's Jev, whether each of the best candidates holds
the answer, and reorders them by its probability. Embeddings (`dijkstral-context-semantic.md`)
change which candidates are found; a classifier reads each candidate with the question, so it
judges relevance rather than similarity. Nothing here changes `dijkstral`.

## 1. Prototype

- Hook: when `DIJKSTRAL_CTX_RERANK=<command>` is set, `rank` sends its best 12 candidates (node,
  label, file, line, the line where the next definition starts, lexical score, test flag) with
  the question and its terms as JSON on the command's stdin, and takes the node ids it prints, best
  first; candidates it omits keep their order after them. A failing command leaves the lexical
  order. Every `rank` call goes through it: code-text anchors, the two after named symbols and
  each end of a flow question.
- `bench/agent/rerank/jevrerank.py`: one Jev `noul` (yes/no) question per candidate, in parallel
  (8 at a time), on `api.typesafe.ai/v1/systemone` with `jev-1.14.0`. The state is an object: the
  question, the symbol (`enclosing › label`), its location and up to 40 numbered source lines of
  its definition. Instructions: "does it implement, define or directly decide what the question
  asks about?", with a yes and a no criterion (a test, example or helper that only mentions the
  words is a no). Answers are cached by model, instructions and state. Orders: `pure` (Jev's
  probability), `rrf` (reciprocal rank fusion with the lexical order, k = 10) and `mix:W`
  ((1 − W) × lexical score / best + W × probability); tests stay last in all of them.
- The key is read from the environment (`TYPESAFE_API_KEY`), never stored by the scripts.

## 2. Results

### 2.1 The benchmark questions

`../bench/agent/context_eval.py`, one answer per question (60), shipped `context` at `782cbbb`:

| order | all | tuning (fastapi, ripgrep) | held out (Kubernetes) | complete answers | median chars |
|---|---:|---:|---:|---:|---:|
| lexical (shipped) | 0.74 | 0.79 | 0.64 | 41/60 | 2994 |
| Jev `pure` | **0.77** | **0.82** | 0.68 | **46/60** | 2844 |
| Jev `mix:0.7` | 0.77 | 0.82 | 0.68 | 46/60 | 2822 |
| Jev `mix:0.5` | 0.76 | 0.81 | 0.68 | 45/60 | 2805 |
| Jev `rrf` | 0.76 | 0.79 | 0.69 | 43/60 | 2877 |
| Jev `mix:0.3` | 0.76 | 0.79 | **0.70** | 44/60 | 2952 |

By kind (`pure`): impact 8 → 11 of 12 complete, callers 12 → 13 of 15, locate 9 → 11 of 15;
flow flat (held-out flow 1 → 0 of 3 complete at equal recall).

### 2.2 The questions agents actually asked

`bench/agent/rerank/agentq.py`: every distinct `dijkstral context` query in the three
`gpt-6-luna` v12 runs (`782cbbb`'s benchmark, `../bench/README.md`), 207 (task, query) pairs, of
which 160 are the agent's own rephrasings rather than the task's question, each scored against its
task's key facts:

| queries | n | recall, lexical → Jev `pure` | complete | Jev better / worse |
|---|---:|---:|---:|---:|
| tuning, all | 121 | 0.705 → **0.734** | 74 → **82** | 20 / 12 |
| tuning, rephrased | 88 | 0.659 → **0.691** | 49 → **54** | 16 / 8 |
| held out, all | 86 | 0.529 → **0.560** | 40 → 41 | 17 / 8 |
| held out, rephrased | 72 | 0.501 → **0.539** | 31 → 33 | 15 / 6 |

Rephrased, by kind: locate 0.49 → 0.58, flow 0.49 → 0.55, architecture 0.76 → 0.71, impact and
callers level. `mix:0.3` keeps architecture (0.75) but gains less on the held-out repository
(0.50 → 0.52); `pure` is the best order overall.

### 2.3 Cost and speed

- 836 calls and 0.73M input tokens for the five Jev orders of §2.1 (each answer cached after
  the first), 2,062 calls and 1.75M tokens for §2.2: about 870 input tokens a candidate, 8–12k a
  question, roughly $0.0005 a question at the vendor's published price. 11 reranker runs had a
  failed call and kept the lexical order (2 in §2.1, 9 in §2.2).
- About 0.3 s a call; 1.5–2 s a question uncached with 8 in parallel, against 20–40 ms for
  `context` on fastapi and ripgrep; the first calls of a session took up to 30 s (a cold start).

## 3. Findings

1. **A consistent gain, on held-out code too.** About +3 points of key-fact coverage on the
   benchmark questions and on the agents' own rephrasings, tuning and held out alike; more
   answers complete. It is the largest single gain since comments counted as descriptions.
2. **It helps where wording misleads.** Locate and flow rephrasings gain most: the classifier
   drops candidates that only repeat the question's words (`OAuth2PasswordRequestForm` for a
   question about request bodies) and lifts the function that decides.
3. **It hurts broad questions.** Architecture answers lose: a classifier judging one definition
   at a time prefers a specific function to the structural code an overview needs.
4. **The agent benchmark was not run.** Codex runs the benchmark's shell commands in a read-only
   sandbox without network or sockets, so a `context` that calls Jev cannot reach it from there.
   The gain is also about the size of the earlier offline gains (+2 to +6 points) whose agent
   effect stayed within three runs' spread, so it would take 6–10 runs to show.
5. **The same sandbox applies to users.** Under Codex's default sandbox a networked reranker in
   the CLI would silently fall back to the lexical order. It has to run where the network is
   available: the MCP server (started by the assistant outside its sandbox) or a hosted service.

## 4. Recommendation

Not shipped in `dijkstral`. A reranker sends code to a third party, needs the network and adds
seconds to a call, so it can never be the default (code extraction and `context` stay local). It
is a candidate for the hosted product as an explicit opt-in, run server side or in the MCP
server, with `pure` ordering and architecture questions (§1, structure) left to the lexical
order. Its agent benchmark belongs there, through the MCP server or the hosted endpoint, with
enough runs to resolve a 3-point coverage gain.

## 5. Not evaluated

- One request ranking all candidates (a `choice` question over the 12) instead of 12 requests.
- Reranking a larger pool (24 or 48 candidates), or the anchors' callers.
- Other Jev models (`jev-preview`), instructions or snippet lengths.
- A local cross-encoder in the same hook, which would avoid the network and the third party.
