# Agent Eval Harness Design

> **Purpose:** Evaluate how well an AI agent can autonomously build games using the Nomai engine,
> scored by the existing eval framework (Tier 1/2/3 metrics + CW-ZTVCR).

**Core question:** How good is AI at creating games, validating what it does, and finding
both code and visual errors by itself — effectively being a never-ending machine of auto
game creation?

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     run_agent_eval.py                           │
│                     (the harness)                               │
│                                                                 │
│  ┌───────────┐     ┌──────────────────────────────────────┐     │
│  │  GDD      │     │         AGENT SESSION                │     │
│  │  (task    │     │                                      │     │
│  │  spec)    │     │  ┌─────────┐    ┌───────────────┐    │     │
│  │           │────▶│  │ Launch  │───▶│ Claude Code   │    │     │
│  │ breakout  │     │  │ claude  │    │ (full tools)  │    │     │
│  │ .md       │     │  │ -p      │    │               │    │     │
│  └───────────┘     │  └─────────┘    │ • Reads GDD   │    │     │
│                    │                 │ • Reads docs   │    │     │
│  ┌───────────┐     │                 │ • Writes .py   │    │     │
│  │  SDK Docs │     │                 │ • Runs script  │    │     │
│  │  (AI      │     │                 │ • Reads snap   │    │     │
│  │  pointer  │     │                 │ • Self-checks  │    │     │
│  │  files)   │─────│────────────────▶│ • Fixes & re-  │    │     │
│  └───────────┘     │                 │   runs if bad  │    │     │
│                    │                 └───────┬───────┘    │     │
│                    │                         │            │     │
│                    │              agent says "done"       │     │
│                    │              or max budget hit       │     │
│                    │                         │            │     │
│                    └─────────────────────────┼────────────┘     │
│                                              │                  │
│                                              ▼                  │
│                    ┌──────────────────────────────────────┐     │
│                    │         SCORING PHASE                 │     │
│                    │                                      │     │
│                    │  1. Run agent's game.py (twice)       │     │
│                    │  2. Capture manifests + snapshot      │     │
│                    │  3. Compare against GDD ground truth  │     │
│                    │  4. Run Tier 1/2/3 eval metrics       │     │
│                    │  5. Produce TaskResult + EvalReport   │     │
│                    └──────────────────────────────────────┘     │
│                                              │                  │
│                                              ▼                  │
│                    ┌──────────────────────────────────────┐     │
│                    │  eval_agent_report.json               │     │
│                    │  • agent_meta (model, time, cost)     │     │
│                    │  • task_result (autonomy dimension)   │     │
│                    │  • eval_report (all dimensions)       │     │
│                    │  • ground_truth_comparison            │     │
│                    └──────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────┘
```

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Agent interaction | Claude Code writes & runs Python scripts | Simplest; uses SDK directly like baseline does |
| Self-verification | Snapshot only | Purest test of thesis: manifest is co-equal with framebuffer |
| First task | Reproduce baseline breakout | Known ground truth, existing eval metrics can score it |
| Documentation | SDK reference doc with file/line pointers | Agent can Read source files; needs a map, not a dump |
| Harness approach | Single `claude -p` call per attempt | Agent iterates internally; harness observes artifacts |
| Scoring | Harness re-runs game.py independently | Clean separation: agent builds, harness scores |

## Two Distinct LLM Roles

```
Judge (Tier 2/3 metrics):   scene text → LLM → answer/score
                            ClaudeCodeLLMClient, no tools, fast

Agent (game developer):     GDD → Claude Code → builds game → artifacts
                            Full claude -p with tools, slow, expensive
```

The eval framework scores the agent's output. The agent does NOT use the eval framework.

## Component Details

### 1. GDD Task Spec Format

Lives at `eval_tasks/<task_name>.md`. Specifies **what**, not **how**.

```markdown
# Task: Breakout Game

## Game Description
A classic breakout game. A paddle at the bottom, a ball that bounces,
and a grid of bricks at the top. The ball destroys bricks on contact.

## Required Entities
- 1 paddle (bottom center, width 100, height 10)
- 1 ball (above paddle, size 8x8, initial velocity: dx=200, dy=-150)
- 20 bricks (4 rows × 5 columns, each 80x20, evenly spaced)
- 3 boundary walls (top, left, right)

## Required Behaviors
- Ball bounces off paddle, walls, and bricks
- Ball destroys brick on contact (despawn)
- Ball velocity is preserved through bounces

## Success Criteria
After 300 ticks:
- All entities exist with correct types and roles
- Ball has moved from starting position
- Physics collisions are working (ball bounces, doesn't pass through)
- At least some bricks have been destroyed

## Output
Your script must be a single Python file at `eval_workdir/<run_id>/game.py` that:
1. Creates the engine and all entities
2. Runs for 300 ticks
3. Prints the final SceneSnapshot summary to stdout
```

### 2. SDK Reference for AI Agent

Lives at `docs/ai/nomai-sdk-reference.md`. Provides a map with file/line pointers
so the agent can Read the exact source it needs. Points to `run_eval_baseline.py`
as the canonical working example.

### 3. Harness Flow

```python
# run_agent_eval.py (pseudocode)

def run_agent_eval(task: str, model: str = "sonnet", budget: float = 5.0):
    # 1. Setup
    run_id = datetime.now().strftime("%Y%m%d_%H%M%S")
    workdir = f"eval_workdir/{run_id}"
    os.makedirs(workdir)
    gdd = read_file(f"eval_tasks/{task}.md")
    sdk_ref = read_file("docs/ai/nomai-sdk-reference.md")

    # 2. Build prompt
    system = AGENT_SYSTEM_PROMPT  # "You are a game developer..."
    prompt = f"{gdd}\n\n{sdk_ref}\n\nOutput to: {workdir}/game.py"

    # 3. Launch agent
    t0 = time.time()
    result = subprocess.run([
        "claude", "-p", prompt,
        "--system-prompt", system,
        "--model", model,
        "--dangerously-skip-permissions",
        "--max-budget-usd", str(budget),
        "--add-dir", workdir,
    ], capture_output=True, text=True, env=strip_claudecode(os.environ))
    wall_time = time.time() - t0

    # 4. Check artifacts
    game_exists = os.path.exists(f"{workdir}/game.py")
    done = os.path.exists(f"{workdir}/DONE.txt")
    stuck = os.path.exists(f"{workdir}/STUCK.txt")

    # 5. Score
    if game_exists:
        task_result, eval_report = score_game(workdir, task)
    else:
        task_result = TaskResult(task_id=task, succeeded=False, ...)

    # 6. Report
    save_report(run_id, agent_meta, task_result, eval_report)
```

### 4. Scoring Phase

```python
def score_game(workdir: str, task: str) -> tuple[TaskResult, EvalReport]:
    # Run game.py twice for reproducibility
    run1 = subprocess.run(["python", f"{workdir}/game.py"], ...)
    run2 = subprocess.run(["python", f"{workdir}/game.py"], ...)
    replay_deterministic = hash(run1.stdout) == hash(run2.stdout)

    # Parse snapshot from stdout, build ground truth from GDD
    snapshot = parse_snapshot(run1.stdout)
    ground_truth = build_ground_truth_from_gdd(task)

    # Run eval framework
    runner = EvalRunner()
    report = runner.run_all(
        scene_snapshot=snapshot,
        snapshot_ground_truth_entities=ground_truth,
        # ... other inputs derived from game output
    )

    # Produce TaskResult
    tier1_pass = all(m.passed for m in report.metrics
                     if m.name.startswith("snapshot_"))
    task_result = TaskResult(
        task_id=task,
        succeeded=tier1_pass,
        iterations=1,
        human_interventions=0,
        replay_deterministic=replay_deterministic,
        perf_gates_met=True,
    )
    return task_result, report
```

### 5. Report Format

```json
{
  "agent_meta": {
    "model": "sonnet",
    "task": "breakout",
    "wall_time_s": 47.3,
    "budget_used_usd": 1.82,
    "exit_code": 0,
    "signal": "DONE",
    "game_script": "eval_workdir/20260314_143022/game.py"
  },
  "task_result": {
    "task_id": "breakout_v1",
    "succeeded": true,
    "complexity_weight": 1.0,
    "iterations": 1,
    "human_interventions": 0,
    "replay_deterministic": true,
    "perf_gates_met": true
  },
  "eval_report": { "..." },
  "ground_truth_comparison": {
    "expected_entities": 24,
    "actual_entities": 22,
    "entity_recall": 0.917,
    "bricks_destroyed": 3,
    "ball_moving": true,
    "physics_working": true
  }
}
```

## Future Extensions (not v1)

- **Outer retry loop**: Re-launch agent with eval feedback if first attempt fails
- **Multiple GDDs**: Pong, space invaders, puzzle games for generalization testing
- **Model comparison**: Run same task across sonnet/opus/haiku, compare CW-ZTVCR
- **Intent specs (v2 verification)**: Agent writes verification rules, not just code
- **Cost tracking**: Parse Claude API usage from CLI output for precise budget tracking
