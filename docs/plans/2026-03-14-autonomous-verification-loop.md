# Autonomous Verification Loop Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable fully autonomous game development by closing the write-verify-fix loop — the agent iterates until the verification engine confirms the game is correct, with zero human intervention.

**Architecture:** Three independent subsystems feed into the agent harness: (1) automated GDD-to-verification-suite generation provides machine-checkable success criteria, (2) engine self-diagnosis surfaces common mistakes in the manifest so the agent can self-correct, (3) multi-iteration agent loop feeds verification failures back as prompts until all intents pass or budget is exhausted.

**Tech Stack:** Python (verification engine, agent harness), Rust (manifest pipeline, diagnostics), PyO3 (bindings)

---

## Task 1: Wire GDD → Verification Suite into Agent Harness

The `parse-gdd` skill, `gdd_pipeline.run_pipeline()`, and `VerificationEngine` all exist but aren't connected to the eval harness. The agent should receive a verification suite alongside the GDD and use it to self-verify.

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py`
- Modify: `eval_tasks/breakout.md`
- Read: `python/nomai-sdk/nomai/gdd_pipeline.py`
- Read: `python/nomai-sdk/nomai/verify.py`
- Read: `python/nomai-sdk/nomai/breakout_intents.py`
- Test: `python/nomai-sdk/tests/test_eval_agent_harness.py`

### Step 1: Generate verification suite from GDD at eval launch time

In `agent_harness.py`, modify `run_agent_eval()` to load the breakout verification suite before launching the agent:

```python
from nomai.breakout_intents import build_breakout_suite

# After reading the GDD, build the verification suite
suite = build_breakout_suite()
suite_json = json.dumps([intent.to_dict() for intent in suite.intents], indent=2)
```

For now, use the hand-crafted `breakout_intents.py`. Later, this can be replaced by `gdd_pipeline.run_pipeline()` for auto-generation from prose.

### Step 2: Include verification suite in agent prompt

Add the suite JSON to the prompt so the agent knows exactly what will be checked:

```python
prompt = (
    f"## Game Design Document\n\n{gdd_content}\n\n"
    f"## SDK Reference\n\n{sdk_ref_content}\n\n"
    f"## WASM Host API\n\n```typescript\n{host_ts_content}\n```\n\n"
    f"## Verification Suite\n\n"
    f"Your game will be verified against these intents. ALL must pass:\n\n"
    f"```json\n{suite_json}\n```\n\n"
    f"## Output Instructions\n\n..."
)
```

### Step 3: Run verification in score_game()

After running game.py successfully, run the verification engine against the collected manifests:

```python
from nomai.verify import VerificationEngine
from nomai.breakout_intents import build_breakout_suite

def score_game(run, *, project_root=None, judge_model=None):
    # ... existing code that runs game.py ...

    # After successful run, collect manifests and verify
    # The game.py should save manifests — or we re-run with manifest collection
    suite = build_breakout_suite()
    engine = VerificationEngine()
    # ... verify against manifests ...
```

Note: This requires the game.py to either save manifests or for score_game to re-run the game and collect them. The simplest approach: have score_game re-run the game script in a mode that collects manifests. Alternatively, add manifest saving to the agent's output requirements.

### Step 4: Include verification results in eval report

Add `verification_report` to the score_game return dict:

```python
ground_truth["verification_passed"] = report.all_passed
ground_truth["verification_results"] = {
    r.intent_name: {
        "passed": r.passed,
        "failure_reason": r.failure_reason,
        "suggestion": r.suggestion.description if r.suggestion else None,
    }
    for r in report.results
}
```

### Step 5: Update succeeded logic

Replace the structural validation with verification engine results when available:

```python
if verification_report is not None:
    succeeded = verification_report.all_passed and entity_count > 0
elif validation is not None:
    succeeded = validation.passed and entity_count > 0
else:
    succeeded = result1.returncode == 0 and entity_count > 0
```

### Step 6: Write tests

- Test that score_game runs verification when suite is available
- Test that verification failures cause succeeded=False
- Test that verification results appear in ground_truth

### Step 7: Commit

```
feat(eval): wire verification suite into agent scoring
```

---

## Task 2: Engine Self-Diagnosis

Add a diagnostics system to the manifest pipeline that catches common mistakes and surfaces them to the agent. Diagnostics are warnings, not errors — they don't stop the simulation.

**Files:**
- Modify: `crates/nomai-manifest/src/manifest.rs` (add DiagnosticEntry, diagnostics field)
- Modify: `crates/nomai-engine/src/tick.rs` (run diagnostics after commands applied)
- Modify: `crates/nomai-python/src/engine.rs` (expose diagnostics to Python)
- Modify: `python/nomai-sdk/nomai/manifest.py` (parse diagnostics)
- Test: `crates/nomai-manifest/src/manifest.rs` (inline tests)
- Test: `python/nomai-sdk/tests/test_manifest.py`

### Step 1: Define DiagnosticEntry in Rust

In `crates/nomai-manifest/src/manifest.rs`:

```rust
/// A diagnostic message surfaced by the engine to help AI agents
/// identify common mistakes without pixel-peeking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DiagnosticEntry {
    /// Severity: "info", "warning", "error"
    pub severity: String,
    /// Human-readable diagnostic message
    pub message: String,
    /// Entity ID affected (if applicable)
    pub entity_id: Option<u64>,
    /// Which system produced this diagnostic
    pub system: String,
}
```

### Step 2: Add diagnostics field to TickManifest

```rust
pub struct TickManifest {
    // ... existing fields ...
    pub diagnostics: Vec<DiagnosticEntry>,
}
```

Update `end_tick()` to include diagnostics in the assembled manifest.

### Step 3: Implement diagnostic checks in tick loop

In `crates/nomai-engine/src/tick.rs`, after Phase 5 (apply commands), run diagnostic checks:

```rust
// Phase 5.5: Run diagnostics
let mut diagnostics = Vec::new();

// Check: entities with physics bodies but no size component
for entity in self.world.alive_entities() {
    let has_physics = self.physics.as_ref()
        .map(|p| p.has_body(entity))
        .unwrap_or(false);
    let has_size = self.world.has_component::<Size>(entity);

    if has_physics && !has_size {
        diagnostics.push(DiagnosticEntry {
            severity: "warning".to_owned(),
            message: format!("Entity {} has physics body but no size component — it will be invisible in visual mode", entity.to_raw()),
            entity_id: Some(entity.to_raw()),
            system: "diagnostics".to_owned(),
        });
    }
}

// Check: entities with position outside declared game bounds
// (requires game bounds to be known — could be stored in world metadata)

// Check: dynamic body with zero velocity for N consecutive ticks
// (track in a HashMap<EntityId, u32> of consecutive-zero-velocity ticks)
```

Note: Some diagnostics (like bounds checking) require knowing the game area dimensions. This could be set via a new `engine.set_game_bounds(width, height)` method, or inferred from wall positions.

### Step 4: Expose diagnostics to Python

In `crates/nomai-python/src/engine.rs`, the `manifest_to_pyobject` function already serializes the full TickManifest via serde_json. Since DiagnosticEntry derives Serialize, it will appear automatically in the JSON.

In `python/nomai-sdk/nomai/manifest.py`, update `TickManifest.from_dict()`:

```python
@dataclass(frozen=True)
class DiagnosticEntry:
    severity: str
    message: str
    entity_id: int | None
    system: str

# In TickManifest.from_dict():
diagnostics = [
    DiagnosticEntry(
        severity=d["severity"],
        message=d["message"],
        entity_id=d.get("entity_id"),
        system=d["system"],
    )
    for d in data.get("diagnostics", [])
]
```

### Step 5: Include diagnostics in agent system prompt feedback

When the agent runs verification and finds diagnostics, include them in the feedback:

```
Diagnostics from tick 1:
  [WARNING] Entity 1 has physics body but no size component — invisible in visual mode
  [WARNING] Entity 1 position (530, 1471) outside game bounds (0-800, 0-600)
```

### Step 6: Write tests

- Rust: test that diagnostics are populated when conditions are met
- Rust: test that diagnostics are empty when everything is fine
- Python: test that diagnostics parse from manifest dict
- Integration: test that an entity without size triggers a diagnostic

### Step 7: Commit

```
feat(manifest): add engine self-diagnosis system
```

---

## Task 3: Multi-Iteration Agent Loop

Replace the single-shot agent eval with an iterative loop: launch → verify → feed failures back → re-launch → verify → repeat until pass or budget exhausted.

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py`
- Modify: `run_agent_eval.py` (add --max-iterations flag)
- Test: `python/nomai-sdk/tests/test_eval_agent_harness.py`

### Step 1: Add iteration support to AgentConfig

```python
@dataclass(frozen=True)
class AgentConfig:
    task: str
    model: str = "sonnet"
    judge_model: str | None = None
    max_budget_usd: float = 5.0
    timeout_s: int = 600
    max_iterations: int = 3
```

### Step 2: Add feedback prompt builder

Create a function that converts verification failures into an actionable prompt:

```python
def _build_feedback_prompt(
    iteration: int,
    verification_results: dict,
    diagnostics: list[dict],
    snapshot_summary: str,
) -> str:
    """Build a feedback prompt from verification failures and diagnostics."""
    lines = [
        f"## Iteration {iteration} — Verification Failed\n",
        "Your game was tested against the verification suite. These checks FAILED:\n",
    ]

    for name, result in verification_results.items():
        if not result["passed"]:
            lines.append(f"- **{name}**: {result['failure_reason']}")
            if result.get("suggestion"):
                lines.append(f"  Suggestion: {result['suggestion']}")

    if diagnostics:
        lines.append("\n## Engine Diagnostics\n")
        for d in diagnostics:
            lines.append(f"- [{d['severity'].upper()}] {d['message']}")

    lines.append(f"\n## Current Game State\n\n```\n{snapshot_summary}\n```\n")
    lines.append("\nFix the issues above and re-run. Do NOT start from scratch — "
                 "iterate on your existing code.")

    return "\n".join(lines)
```

### Step 3: Implement iterative run_agent_eval

Replace the single-shot flow with a loop:

```python
def run_agent_eval(task, model, max_budget_usd, *, judge_model=None,
                   max_iterations=3, project_root=None):
    # ... setup ...

    for iteration in range(1, max_iterations + 1):
        print(f"\n--- Iteration {iteration}/{max_iterations} ---")

        if iteration == 1:
            agent_run = launch_agent(config, project_root=root)
        else:
            # Re-launch with feedback from previous iteration
            agent_run = launch_agent(
                config,
                project_root=root,
                feedback_prompt=feedback,
            )

        score = score_game(agent_run, project_root=root, judge_model=judge_model)
        task_result = score["task_result"]

        if task_result.succeeded:
            print(f"  >>> PASS on iteration {iteration} <<<")
            break

        # Build feedback for next iteration
        feedback = _build_feedback_prompt(
            iteration=iteration,
            verification_results=score["ground_truth"].get("verification_results", {}),
            diagnostics=score["ground_truth"].get("diagnostics", []),
            snapshot_summary=score["ground_truth"].get("snapshot_summary", ""),
        )
        print(f"  FAIL — feeding back {len(feedback)} chars of feedback")

    # Build final report with iteration history
    report["iterations"] = iteration
    report["converged"] = task_result.succeeded
```

### Step 4: Modify launch_agent to accept feedback

Add an optional `feedback_prompt` parameter to `launch_agent()`. When provided, append it to the user prompt:

```python
def launch_agent(config, *, project_root=None, feedback_prompt=None):
    # ... existing setup ...

    if feedback_prompt:
        prompt += f"\n\n{feedback_prompt}"

    # ... existing subprocess call ...
```

The key insight: the agent's workdir persists between iterations. The agent sees its previous code and can iterate on it rather than starting from scratch.

### Step 5: Add --max-iterations CLI flag

In `run_agent_eval.py`:

```python
parser.add_argument(
    "--max-iterations",
    type=int,
    default=3,
    help="Maximum write-verify-fix iterations (default: 3)",
)
```

### Step 6: Update TaskResult with iteration tracking

```python
task_result = TaskResult(
    task_id=run.config.task,
    succeeded=succeeded,
    iterations=iteration,
    human_interventions=0,
    replay_deterministic=replay_deterministic,
)
```

### Step 7: Write tests

- Test single iteration (backwards compatible)
- Test multi-iteration with mocked failing then passing verification
- Test max_iterations limit reached
- Test feedback prompt generation from verification failures
- Test that workdir persists between iterations

### Step 8: Commit

```
feat(eval): multi-iteration agent loop with verification feedback
```

---

## Task 4: Integration Test — End-to-End Autonomous Run

Verify all three subsystems work together by running a full eval.

**Files:**
- No new code — just run the eval and verify

### Step 1: Run full eval with iterations

```bash
python run_agent_eval.py \
  --task breakout \
  --model haiku \
  --budget 5.0 \
  --max-iterations 3
```

### Step 2: Verify report

Check that the report contains:
- `iterations` count showing how many loops the agent needed
- `verification_results` with per-intent pass/fail
- `diagnostics` from the engine (if any)
- `converged: true` if all intents passed

### Step 3: Commit final report

```
docs: add autonomous verification loop eval results
```

---

## Dependency Graph

```
Task 1 (Verification Suite)  ──┐
                                ├──► Task 4 (Integration Test)
Task 2 (Engine Diagnostics)  ──┤
                                │
Task 3 (Multi-Iteration Loop) ─┘
```

Tasks 1, 2, and 3 are independent and can be developed in parallel.
Task 4 requires all three.

## Estimated Scope

| Task | Files | New Lines (est.) | Parallel? |
|------|-------|-------------------|-----------|
| Task 1: Verification Suite | 3 modified, 1 test | ~150 | Yes |
| Task 2: Engine Diagnostics | 4 modified, 2 tests | ~200 (Rust+Python) | Yes |
| Task 3: Multi-Iteration Loop | 3 modified, 1 test | ~200 | Yes |
| Task 4: Integration Test | 0 | 0 (just run) | After 1-3 |
