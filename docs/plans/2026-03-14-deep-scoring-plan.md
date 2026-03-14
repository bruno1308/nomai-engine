# Deep Scoring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend the agent eval harness to run LLM-judged Tier 2/3 metrics (Scene QA, G-Eval, Multi-hop Spatial) when the agent saves a `snapshot.json`, producing richer eval scores.

**Architecture:** The agent's `game.py` saves `snapshot.json` via `SceneSnapshot.to_dict()`. After the existing determinism check, `score_game` deserializes the snapshot, creates a `ClaudeCodeLLMClient` with a configurable judge model, and runs three snapshot-based scorers. Results go into the report under `"llm_scores"`.

**Tech Stack:** Python, nomai-sdk eval framework, `ClaudeCodeLLMClient` (subprocess to `claude -p`)

**Design doc:** `docs/plans/2026-03-14-deep-scoring-design.md`

---

### Task 1: Add `judge_model` to AgentConfig

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py:61-75`
- Modify: `python/nomai-sdk/tests/test_eval_agent_harness.py`

**Step 1: Write the failing test**

Add to `TestAgentConfig` in `tests/test_eval_agent_harness.py`:

```python
def test_judge_model_default(self) -> None:
    """AgentConfig should default judge_model to 'sonnet'."""
    config = AgentConfig(task="breakout")
    assert config.judge_model == "sonnet"

def test_judge_model_custom(self) -> None:
    """AgentConfig should accept custom judge_model."""
    config = AgentConfig(task="breakout", judge_model="opus")
    assert config.judge_model == "opus"
```

**Step 2: Run test to verify it fails**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestAgentConfig::test_judge_model_default -v`
Expected: FAIL (TypeError: unexpected keyword argument 'judge_model')

**Step 3: Write minimal implementation**

In `agent_harness.py`, add `judge_model` field to `AgentConfig`:

```python
@dataclass(frozen=True)
class AgentConfig:
    task: str
    model: str = "sonnet"
    judge_model: str = "sonnet"
    max_budget_usd: float = 5.0
    timeout_s: int = 600
```

**Step 4: Run tests to verify they pass**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestAgentConfig -v`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py python/nomai-sdk/tests/test_eval_agent_harness.py
git commit -m "feat(eval): add judge_model to AgentConfig"
```

---

### Task 2: Update system prompt and GDD to require snapshot.json

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py:33-54` (AGENT_SYSTEM_PROMPT)
- Modify: `eval_tasks/breakout.md`
- Modify: `python/nomai-sdk/tests/test_eval_agent_harness.py`

**Step 1: Write the failing test**

Add to `TestModuleConstants` in `tests/test_eval_agent_harness.py`:

```python
def test_agent_system_prompt_mentions_snapshot_json(self) -> None:
    """System prompt must instruct agent to save snapshot.json."""
    assert "snapshot.json" in AGENT_SYSTEM_PROMPT
```

**Step 2: Run test to verify it fails**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestModuleConstants::test_agent_system_prompt_mentions_snapshot_json -v`
Expected: FAIL (AssertionError)

**Step 3: Update the system prompt**

In `agent_harness.py`, update `AGENT_SYSTEM_PROMPT` — add after item 6 (before item 7):

```
7. Save the final snapshot as JSON for scoring:
   import json
   with open("snapshot.json", "w") as f:
       json.dump(snapshot.to_dict(), f)
```

Renumber existing items 7→8, 8→9.

**Step 4: Update the GDD**

In `eval_tasks/breakout.md`, add to the `## Output` section:

```
8. Save the final snapshot as `snapshot.json` via `json.dump(snapshot.to_dict(), open("snapshot.json", "w"))`
```

**Step 5: Run tests to verify they pass**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestModuleConstants -v`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py eval_tasks/breakout.md
git commit -m "feat(eval): instruct agent to save snapshot.json for deep scoring"
```

---

### Task 3: Extend `score_game` with LLM-judged scoring

This is the core task. `score_game` gets an optional `judge_model` parameter. When `snapshot.json` exists and `judge_model` is provided, it runs Scene QA, G-Eval, and Multi-hop Spatial.

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py:286-371`
- Modify: `python/nomai-sdk/tests/test_eval_agent_harness.py`

**Step 1: Write the failing tests**

Add a new test class `TestScoreGameDeepScoring` to `tests/test_eval_agent_harness.py`:

```python
from unittest.mock import MagicMock, patch
from nomai.eval.agent_harness import AgentConfig, AgentRun, score_game
from nomai.scene import SceneSnapshot, SceneEntity, SceneBounds
import json

class TestScoreGameDeepScoring:
    """Tests for LLM-judged deep scoring in score_game."""

    def _make_snapshot_dict(self) -> dict:
        """Create a minimal snapshot dict for testing."""
        return {
            "schema_version": 1,
            "tick": 300,
            "sim_time": 5.0,
            "entities": [
                {
                    "entity_id": 1,
                    "entity_type": "character",
                    "role": "paddle",
                    "tier": "Foreground",
                    "position": [400.0, 560.0],
                    "size": [100.0, 15.0],
                    "velocity": None,
                    "visible": True,
                    "z_index": 0.0,
                    "components": {},
                },
                {
                    "entity_id": 2,
                    "entity_type": "projectile",
                    "role": "ball",
                    "tier": "Foreground",
                    "position": [350.0, 200.0],
                    "size": [8.0, 8.0],
                    "velocity": [200.0, -300.0],
                    "visible": True,
                    "z_index": 0.0,
                    "components": {},
                },
            ],
            "bounds": {"min_x": 0, "min_y": 0, "max_x": 800, "max_y": 600},
            "entity_count": 2,
        }

    def _make_run(self, workdir, exit_code=0) -> AgentRun:
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir,
            exit_code=exit_code,
            wall_time_s=10.0,
            stdout="ENTITY_COUNT: 2",
            stderr="",
        )

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_no_snapshot_returns_null_llm_scores(
        self, mock_run_script, tmp_path
    ):
        """When snapshot.json is missing, llm_scores should be None."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 2')")
        mock_run_script.return_value = MagicMock(
            returncode=0, stdout="ENTITY_COUNT: 2", stderr=""
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path, judge_model="sonnet")
        assert result["llm_scores"] is None

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_no_judge_model_returns_null_llm_scores(
        self, mock_run_script, tmp_path
    ):
        """When judge_model is None, llm_scores should be None even if snapshot exists."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 2')")
        (tmp_path / "snapshot.json").write_text(
            json.dumps(self._make_snapshot_dict())
        )
        mock_run_script.return_value = MagicMock(
            returncode=0, stdout="ENTITY_COUNT: 2", stderr=""
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)
        assert result["llm_scores"] is None

    @patch("nomai.eval.agent_harness.multihop_spatial_accuracy")
    @patch("nomai.eval.agent_harness.geval_all")
    @patch("nomai.eval.agent_harness.scene_qa_accuracy")
    @patch("nomai.eval.agent_harness.generate_spatial_questions")
    @patch("nomai.eval.agent_harness.generate_scene_questions")
    @patch("nomai.eval.agent_harness.ClaudeCodeLLMClient")
    @patch("nomai.eval.agent_harness._run_game_script")
    def test_deep_scoring_runs_all_three_dimensions(
        self,
        mock_run_script,
        mock_llm_cls,
        mock_gen_scene_q,
        mock_gen_spatial_q,
        mock_scene_qa,
        mock_geval,
        mock_multihop,
        tmp_path,
    ):
        """When snapshot.json exists and judge_model is set, run all three scorers."""
        from nomai.eval.metrics import EvalDimension, MetricResult

        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 2')")
        (tmp_path / "snapshot.json").write_text(
            json.dumps(self._make_snapshot_dict())
        )
        mock_run_script.return_value = MagicMock(
            returncode=0, stdout="ENTITY_COUNT: 2", stderr=""
        )

        # Mock LLM client
        mock_llm = MagicMock()
        mock_llm_cls.return_value = mock_llm

        # Mock scene QA
        mock_gen_scene_q.return_value = []
        mock_scene_qa.return_value = MetricResult(
            name="scene_qa_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=0.85,
            target=0.8,
            passed=True,
            detail="17/20 correct",
        )

        # Mock G-Eval (returns list of 4 MetricResults)
        mock_geval.return_value = [
            MetricResult(
                name=f"geval_{c}",
                dimension=EvalDimension.VERIFICATION,
                value=v,
                target=None,
                passed=True,
                detail=f"{c} score",
            )
            for c, v in [
                ("completeness", 0.75),
                ("clarity", 0.80),
                ("spatial_accuracy", 0.70),
                ("actionability", 0.65),
            ]
        ]

        # Mock multi-hop
        mock_gen_spatial_q.return_value = []
        mock_multihop.return_value = MetricResult(
            name="multihop_spatial_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=0.60,
            target=None,
            passed=True,
            detail="3/5 correct",
        )

        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path, judge_model="sonnet")

        assert result["llm_scores"] is not None
        scores = result["llm_scores"]
        assert scores["judge_model"] == "sonnet"
        assert scores["scene_qa_accuracy"] == 0.85
        assert scores["geval_completeness"] == 0.75
        assert scores["geval_clarity"] == 0.80
        assert scores["geval_spatial_accuracy"] == 0.70
        assert scores["geval_actionability"] == 0.65
        assert scores["multihop_spatial_accuracy"] == 0.60

        # Verify LLM client was created with correct model
        mock_llm_cls.assert_called_once_with(model="sonnet")
```

**Step 2: Run tests to verify they fail**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestScoreGameDeepScoring -v`
Expected: FAIL (score_game does not accept judge_model / no llm_scores key)

**Step 3: Write the implementation**

In `agent_harness.py`, update imports at the top:

```python
from nomai.eval.llm_client import ClaudeCodeLLMClient
from nomai.eval.scene_qa import generate_scene_questions, scene_qa_accuracy
from nomai.eval.reasoning import geval_all, generate_spatial_questions, multihop_spatial_accuracy
from nomai.scene import SceneSnapshot
```

Update `score_game` signature:

```python
def score_game(run: AgentRun, *, project_root: Path | None = None, judge_model: str | None = None) -> dict:
```

After the existing `return` block (line 363-371), replace it with:

```python
    # --- LLM-judged deep scoring ---
    llm_scores = None
    snapshot_path = run.workdir / "snapshot.json"
    if snapshot_path.exists() and judge_model:
        try:
            snapshot_data = json.loads(snapshot_path.read_text(encoding="utf-8"))
            snapshot = SceneSnapshot.from_dict(snapshot_data)
            llm = ClaudeCodeLLMClient(model=judge_model)

            # Scene QA (Tier 2)
            scene_questions = generate_scene_questions(snapshot)
            scene_qa_result = scene_qa_accuracy(snapshot, scene_questions, llm)

            # G-Eval (Tier 3)
            geval_results = geval_all(snapshot, llm)

            # Multi-hop spatial (Tier 3)
            spatial_questions = generate_spatial_questions(snapshot)
            multihop_result = multihop_spatial_accuracy(snapshot, spatial_questions, llm)

            llm_scores = {
                "judge_model": judge_model,
                "scene_qa_accuracy": scene_qa_result.value,
            }
            for gr in geval_results:
                llm_scores[gr.name] = gr.value
            llm_scores["multihop_spatial_accuracy"] = multihop_result.value

        except Exception:
            logger.exception("Deep scoring failed — continuing with basic score")

    return {
        "task_result": task_result,
        "eval_report": None,
        "ground_truth": {
            "entity_count": entity_count,
            "replay_deterministic": replay_deterministic,
            "stdout_hash": hashlib.sha256(result1.stdout.encode()).hexdigest(),
        },
        "llm_scores": llm_scores,
    }
```

Also update the early-return dicts (lines 305-312, 318-325, 327-339) to include `"llm_scores": None`.

**Step 4: Run tests to verify they pass**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestScoreGameDeepScoring -v`
Expected: ALL PASS

Also run existing tests to verify no regressions:
Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py -v`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py python/nomai-sdk/tests/test_eval_agent_harness.py
git commit -m "feat(eval): add LLM-judged deep scoring to score_game"
```

---

### Task 4: Update `run_agent_eval` to pass judge_model and print LLM scores

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py:378-464` (run_agent_eval)
- Modify: `python/nomai-sdk/tests/test_eval_agent_harness.py`

**Step 1: Write the failing test**

Update the existing `TestRunAgentEval.test_produces_report_dict` or add a new test:

```python
@patch("nomai.eval.agent_harness.score_game")
@patch("nomai.eval.agent_harness.launch_agent")
def test_report_includes_llm_scores(
    self,
    mock_launch: MagicMock,
    mock_score: MagicMock,
    tmp_path: Path,
) -> None:
    """run_agent_eval should include llm_scores in the report."""
    mock_run = MagicMock(spec=AgentRun)
    mock_run.exit_code = 0
    mock_run.wall_time_s = 10.0
    mock_run.signal = "DONE"
    mock_run.game_exists = True
    mock_run.workdir = tmp_path / "eval_workdir" / "mock_run"
    mock_run.workdir.mkdir(parents=True, exist_ok=True)
    mock_run.stdout = "agent output"
    mock_run.stderr = ""
    mock_launch.return_value = mock_run

    mock_score.return_value = {
        "task_result": TaskResult(
            task_id="breakout", succeeded=True, replay_deterministic=True,
        ),
        "eval_report": None,
        "ground_truth": {"entity_count": 24},
        "llm_scores": {
            "judge_model": "sonnet",
            "scene_qa_accuracy": 0.85,
            "geval_completeness": 0.75,
            "geval_clarity": 0.80,
            "geval_spatial_accuracy": 0.70,
            "geval_actionability": 0.65,
            "multihop_spatial_accuracy": 0.60,
        },
    }

    report = run_agent_eval(
        task="breakout", model="sonnet", max_budget_usd=5.0,
        project_root=tmp_path,
    )

    assert "llm_scores" in report
    assert report["llm_scores"]["scene_qa_accuracy"] == 0.85
    # Verify score_game was called with judge_model
    mock_score.assert_called_once()
    call_kwargs = mock_score.call_args[1]
    assert "judge_model" in call_kwargs
```

**Step 2: Run test to verify it fails**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestRunAgentEval::test_report_includes_llm_scores -v`
Expected: FAIL

**Step 3: Write the implementation**

Update `run_agent_eval` signature to accept `judge_model`:

```python
def run_agent_eval(
    task: str = "breakout",
    model: str = "sonnet",
    max_budget_usd: float = 5.0,
    *,
    judge_model: str = "sonnet",
    project_root: Path | None = None,
) -> dict:
```

Update `AgentConfig` creation to include `judge_model`:

```python
    config = AgentConfig(
        task=task,
        model=model,
        judge_model=judge_model,
        max_budget_usd=max_budget_usd,
    )
```

Update the header print to show judge model:

```python
    print(f"  judge={config.judge_model}")
```

Update `score_game` call to pass `judge_model`:

```python
    score = score_game(agent_run, project_root=root, judge_model=config.judge_model)
```

Add `llm_scores` to the report dict:

```python
    report = {
        "agent_meta": { ... },
        "task_result": asdict(task_result),
        "ground_truth": score["ground_truth"],
        "llm_scores": score.get("llm_scores"),
    }
```

Add LLM scores to the verdict print:

```python
    llm_scores = score.get("llm_scores")
    if llm_scores:
        print(f"\n  LLM Scores (judge={llm_scores['judge_model']}):")
        for key, val in llm_scores.items():
            if key != "judge_model":
                print(f"    {key}: {val:.2f}")
```

**Step 4: Run tests to verify they pass**

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestRunAgentEval -v`
Expected: ALL PASS

Run: `python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py -v`
Expected: ALL PASS (no regressions)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py python/nomai-sdk/tests/test_eval_agent_harness.py
git commit -m "feat(eval): wire judge_model through run_agent_eval and print LLM scores"
```

---

### Task 5: Update CLI entry point with --judge-model flag

**Files:**
- Modify: `run_agent_eval.py`

**Step 1: Add the argument**

Add after the `--budget` argument:

```python
    parser.add_argument(
        "--judge-model",
        default="sonnet",
        help="Model for LLM-judged scoring (default: sonnet)",
    )
```

Pass it to `run_agent_eval`:

```python
    report = run_agent_eval(
        task=args.task,
        model=args.model,
        max_budget_usd=args.budget,
        judge_model=args.judge_model,
    )
```

**Step 2: Verify help text**

Run: `python run_agent_eval.py --help`
Expected: Shows `--judge-model` option

**Step 3: Commit**

```bash
git add run_agent_eval.py
git commit -m "feat(eval): add --judge-model CLI flag"
```

---

### Task 6: Dry run — verify deep scoring end-to-end

**Step 1: Run the full pipeline**

```bash
cd B:/Projects/Nomai
unset CLAUDECODE
python run_agent_eval.py --model haiku --budget 1.0 --judge-model haiku
```

Expected output includes:
- `[1/3] Launching agent...` — agent builds game
- `[2/3] Scoring game...` — basic pass/fail + LLM scores
- LLM Scores section with scene_qa_accuracy, geval_*, multihop_spatial_accuracy
- `[3/3] Verdict` — PASS/PARTIAL/FAIL
- Report saved with `llm_scores` key populated

**Step 2: Verify the report**

```bash
cat eval_agent_report.json | python -m json.tool
```

Confirm `llm_scores` is populated (not null) with all 6 metric values.

**Step 3: Verify snapshot.json was saved**

```bash
ls eval_workdir/<latest>/snapshot.json
```

Confirm the file exists and contains valid JSON.
