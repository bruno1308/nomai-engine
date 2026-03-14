# Deep Scoring: Wire LLM-Judged Dimensions into Agent Eval

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend the agent eval harness so that `score_game` runs Tier 2/3 LLM-judged metrics (Scene QA, G-Eval, Multi-hop Spatial) when a `snapshot.json` is available, producing richer eval scores beyond basic pass/fail.

**Architecture:** The agent's `game.py` saves a `snapshot.json` (SceneSnapshot serialized via `to_dict()`). After the existing determinism check, `score_game` deserializes the snapshot, instantiates a `ClaudeCodeLLMClient` with a configurable judge model, and runs the three snapshot-based eval dimensions. Results are included in the report under `"llm_scores"`.

**Tech Stack:** Python, nomai-sdk eval framework, ClaudeCodeLLMClient (subprocess to `claude -p`)

---

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Snapshot extraction | game.py saves `snapshot.json` | One extra line, fully structured, reliable |
| Judge model | Configurable via `judge_model`, default sonnet | Strong judge by default, flexible for cost/speed tuning |
| Scoring entry point | Extend `score_game` with optional LLM client | Single API, graceful degradation when snapshot missing |
| Dimensions wired | Scene QA, G-Eval, Multi-hop Spatial | All snapshot-based, shared plumbing, highest ROI |

## Data Flow

```
Agent writes game.py
   ↓
game.py runs: engine simulates 300 ticks
   ↓
game.py saves: snapshot.json (SceneSnapshot.to_json())
   ↓
score_game reads snapshot.json → SceneSnapshot.from_dict()
   ↓
If snapshot exists + judge_model configured:
   ├── Scene QA: generate_scene_questions(snapshot) → LLM answers → accuracy
   ├── G-Eval: geval_all(snapshot, llm) → completeness/clarity/spatial/actionability
   └── Multi-hop: generate_spatial_questions(snapshot) → LLM answers → accuracy
   ↓
All scores bundled into report alongside existing TaskResult
```

## Changes

### AgentConfig
Add `judge_model: str = "sonnet"` field.

### AGENT_SYSTEM_PROMPT
Add instruction: "Save the final snapshot as JSON: `json.dump(snapshot.to_dict(), open('snapshot.json', 'w'))`"

### breakout.md GDD
Add to Output section: "8. Save the final snapshot as `snapshot.json` via `snapshot.to_dict()`"

### score_game
Add optional `judge_model: str | None = None` parameter. When `snapshot.json` exists and `judge_model` is set:
1. Deserialize snapshot via `SceneSnapshot.from_dict()`
2. Create `ClaudeCodeLLMClient(model=judge_model)`
3. Run Scene QA, G-Eval, Multi-hop Spatial
4. Include results in return dict under `"llm_scores"`

### run_agent_eval
- Pass `judge_model` from config to `score_game`
- Print LLM scores in verdict section
- Include in saved JSON report

### run_agent_eval.py CLI
Add `--judge-model` argument (default: "sonnet").

### Report Format
```json
{
  "agent_meta": { ... },
  "task_result": { ... },
  "ground_truth": { ... },
  "llm_scores": {
    "judge_model": "sonnet",
    "scene_qa_accuracy": 0.85,
    "geval_completeness": 0.75,
    "geval_clarity": 0.80,
    "geval_spatial_accuracy": 0.70,
    "geval_actionability": 0.65,
    "multihop_spatial_accuracy": 0.60
  }
}
```

When `snapshot.json` is missing, `"llm_scores"` is `null`.

## Not in Scope

- Observability (needs tick manifests)
- Controllability (needs interactive command testing)
- Verification (needs bug corpus + intent suite)
- Reproducibility beyond determinism (needs hash checkpoints)
- These can be added later by requiring the agent to save additional data files.
