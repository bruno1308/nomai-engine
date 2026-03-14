# Agent Eval Harness — Next Steps

## What Was Built

The `feat/eval-framework` branch adds an autonomous agent eval harness to the Nomai engine. An AI agent (Claude Code) receives a Game Design Document (GDD), builds a game using the Nomai SDK, and the harness scores the result.

### Components

| Component | File | Purpose |
|-----------|------|---------|
| Agent harness | `python/nomai-sdk/nomai/eval/agent_harness.py` | Launches Claude Code, scores output, produces report |
| CLI entry point | `run_agent_eval.py` | `python run_agent_eval.py --model haiku --budget 1.0` |
| Breakout GDD | `eval_tasks/breakout.md` | First task spec (paddle, ball, 20 bricks, 3 walls) |
| SDK reference | `docs/ai/nomai-sdk-reference.md` | API docs the agent reads to learn the engine |
| ClaudeCodeLLMClient | `python/nomai-sdk/nomai/eval/llm_client.py` | Subprocess wrapper for `claude -p` (judge role) |
| Eval workdir | `eval_workdir/` | Isolated per-run directories (gitignored) |

### How It Works

```
run_agent_eval.py
  → AgentConfig (task, model, judge_model, budget, timeout)
  → launch_agent: spawns `claude -p` with GDD + SDK ref
     → agent writes game.py, snapshot.json, DONE.txt in eval_workdir/<timestamp>/
  → score_game: runs game.py twice (determinism check), optionally runs LLM judge
  → report saved to eval_agent_report.json
```

### Key Design Decisions

- `cwd=workdir` so agent writes files to isolated timestamped directory
- `--add-dir=project_root` so agent can read reference files
- `env.pop("CLAUDECODE")` to allow nested Claude Code sessions
- Deep scoring (Scene QA, G-Eval, Multi-hop) is **opt-in** via `--judge-model`
- Agent system prompt instructs saving `snapshot.json` for LLM-judged scoring

## Known Issues

### 1. Agent CWD Reliability (Priority: High)

The agent sometimes writes `game.py` to the parent `eval_workdir/` directory instead of its timestamped subdirectory. This happens non-deterministically — the agent navigates away from its `cwd` despite being launched with `cwd=workdir`.

**Possible fixes:**
- Add explicit instruction to the system prompt: "Write ALL files to your current working directory. Do NOT navigate to other directories."
- Add a fallback in `score_game` that searches parent directories for `game.py` if not found in workdir
- Use `--allowed-dirs` (if Claude Code supports it) to restrict the agent's file access

### 2. Deep Scoring Performance (Priority: Medium)

The LLM-judged scoring makes 74-144 sequential `claude -p` subprocess calls (one per question). Each call takes 5-30 seconds, making deep scoring take 30+ minutes.

**Possible fixes:**
- **Batch questions:** Modify `scene_qa_accuracy` and `multihop_spatial_accuracy` to send all questions in a single prompt and parse answers from one response
- **Add a `BatchLLMClient`** that accumulates questions and sends them in one `claude -p` call
- **Parallelize:** Use `concurrent.futures.ThreadPoolExecutor` to run multiple `claude -p` calls concurrently (check if Claude CLI supports concurrent sessions)

### 3. Score Validation (Priority: Low)

Current `score_game` only checks:
- Did game.py run without crashing?
- Did it produce entities? (ENTITY_COUNT > 0)
- Is it deterministic? (two runs produce same stdout hash)

It does NOT check:
- Are the entities correct types/roles? (paddle, ball, bricks, walls)
- Are positions reasonable?
- Does the ball actually move?
- Were bricks destroyed?

These checks exist in `run_eval_baseline.py` but aren't wired into the agent harness. Could be added as Tier 1 (no LLM needed) structural validation.

## What To Build Next

### Tier 1: Structural Validation (no LLM, fast)
- Parse `snapshot.json` and verify entity types match GDD requirements
- Check ball moved from starting position
- Check at least some bricks were destroyed
- Check positions are within game bounds (0-800, 0-600)

### More GDD Tasks
- `eval_tasks/pong.md` — two paddles, simpler than breakout
- `eval_tasks/space_invaders.md` — grid of enemies, player ship, bullets
- Increase `complexity_weight` for harder tasks to test CW-ZTVCR scaling

### Observability/Controllability Dimensions
- Requires the agent to also save tick manifests (`manifests.json`)
- Add to system prompt: save manifests collected during simulation
- Wire into `EvalRunner.run_observability()` and `run_controllability()`

## Test Coverage

- 33 tests in `tests/test_eval_agent_harness.py`
- 7 tests in `tests/test_eval_llm.py` (ClaudeCodeLLMClient)
- All 508+ project tests pass

## Running the Eval

```bash
# Basic eval (fast, ~1-4 min)
python run_agent_eval.py --model haiku --budget 1.0

# With deep LLM scoring (slow, ~30+ min)
python run_agent_eval.py --model haiku --budget 1.0 --judge-model haiku

# Full power
python run_agent_eval.py --model sonnet --budget 5.0 --judge-model sonnet
```
