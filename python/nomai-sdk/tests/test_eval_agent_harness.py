"""Tests for the agent eval harness.

Verifies AgentConfig defaults/custom values, AgentRun property logic,
launch_agent subprocess orchestration with mocked subprocess.run,
score_game scoring logic, and run_agent_eval integration.
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch

from nomai.eval.agent_harness import (
    AGENT_SYSTEM_PROMPT,
    PROJECT_ROOT,
    AgentConfig,
    AgentRun,
    SnapshotValidation,
    _build_feedback_prompt,
    launch_agent,
    run_agent_eval,
    score_game,
    validate_breakout_snapshot,
)
from nomai.eval.autonomy import TaskResult
from nomai.scene import SceneEntity, SceneSnapshot


# ---------------------------------------------------------------------------
# AgentConfig
# ---------------------------------------------------------------------------

class TestAgentConfig:
    def test_defaults(self) -> None:
        cfg = AgentConfig(task="breakout")
        assert cfg.task == "breakout"
        assert cfg.model == "sonnet"
        assert cfg.max_budget_usd == 5.0
        assert cfg.timeout_s == 600

    def test_custom_values(self) -> None:
        cfg = AgentConfig(
            task="pong",
            model="opus",
            max_budget_usd=10.0,
            timeout_s=1200,
        )
        assert cfg.task == "pong"
        assert cfg.model == "opus"
        assert cfg.max_budget_usd == 10.0
        assert cfg.timeout_s == 1200

    def test_judge_model_default(self) -> None:
        config = AgentConfig(task="breakout")
        assert config.judge_model is None

    def test_judge_model_custom(self) -> None:
        config = AgentConfig(task="breakout", judge_model="opus")
        assert config.judge_model == "opus"

    def test_frozen(self) -> None:
        cfg = AgentConfig(task="breakout")
        try:
            cfg.task = "pong"  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass


# ---------------------------------------------------------------------------
# AgentRun
# ---------------------------------------------------------------------------

class TestAgentRun:
    def _make_run(self, workdir: Path, exit_code: int = 0) -> AgentRun:
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir,
            exit_code=exit_code,
            wall_time_s=42.0,
            stdout="some output",
            stderr="",
        )

    def test_game_script_path(self, tmp_path: Path) -> None:
        run = self._make_run(tmp_path)
        assert run.game_script == tmp_path / "game.py"

    def test_game_exists_false(self, tmp_path: Path) -> None:
        run = self._make_run(tmp_path)
        assert run.game_exists is False

    def test_game_exists_true(self, tmp_path: Path) -> None:
        (tmp_path / "game.py").write_text("print('hello')")
        run = self._make_run(tmp_path)
        assert run.game_exists is True

    def test_game_exists_fallback_parent_dir(self, tmp_path: Path) -> None:
        """game.py in parent dir should be found via fallback."""
        subdir = tmp_path / "timestamp_subdir"
        subdir.mkdir()
        (tmp_path / "game.py").write_text("print('hello')")
        run = self._make_run(subdir)
        assert run.game_exists is True
        assert run.game_script == tmp_path / "game.py"

    def test_game_script_prefers_workdir_over_parent(self, tmp_path: Path) -> None:
        """If game.py exists in both workdir and parent, prefer workdir."""
        subdir = tmp_path / "timestamp_subdir"
        subdir.mkdir()
        (subdir / "game.py").write_text("print('workdir')")
        (tmp_path / "game.py").write_text("print('parent')")
        run = self._make_run(subdir)
        assert run.game_script == subdir / "game.py"

    def test_signal_done(self, tmp_path: Path) -> None:
        (tmp_path / "DONE.txt").write_text("done")
        run = self._make_run(tmp_path, exit_code=0)
        assert run.signal == "DONE"

    def test_signal_done_fallback_parent(self, tmp_path: Path) -> None:
        """DONE.txt in parent dir should be found via fallback."""
        subdir = tmp_path / "timestamp_subdir"
        subdir.mkdir()
        (tmp_path / "DONE.txt").write_text("done")
        run = self._make_run(subdir, exit_code=0)
        assert run.signal == "DONE"

    def test_signal_stuck(self, tmp_path: Path) -> None:
        (tmp_path / "STUCK.txt").write_text("stuck")
        run = self._make_run(tmp_path, exit_code=0)
        assert run.signal == "STUCK"

    def test_signal_done_takes_priority_over_stuck(self, tmp_path: Path) -> None:
        """If both DONE.txt and STUCK.txt exist, DONE wins."""
        (tmp_path / "DONE.txt").write_text("done")
        (tmp_path / "STUCK.txt").write_text("stuck")
        run = self._make_run(tmp_path, exit_code=0)
        assert run.signal == "DONE"

    def test_signal_timeout(self, tmp_path: Path) -> None:
        run = self._make_run(tmp_path, exit_code=1)
        assert run.signal == "TIMEOUT"

    def test_signal_unknown(self, tmp_path: Path) -> None:
        run = self._make_run(tmp_path, exit_code=0)
        assert run.signal == "UNKNOWN"


# ---------------------------------------------------------------------------
# launch_agent
# ---------------------------------------------------------------------------

class TestLaunchAgent:
    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_creates_workdir(self, mock_run: MagicMock, tmp_path: Path) -> None:
        """launch_agent should create the eval_workdir/<timestamp>/ directory."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="agent output", stderr=""
        )
        cfg = AgentConfig(task="breakout")
        result = launch_agent(cfg, project_root=root)

        assert result.workdir.exists()
        # workdir is root/eval_workdir/<timestamp>
        assert result.workdir.parent == root / "eval_workdir"
        assert str(result.workdir).startswith(str(root / "eval_workdir"))

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_calls_claude_with_correct_args(
        self, mock_run: MagicMock, tmp_path: Path
    ) -> None:
        """Verify the claude CLI is invoked with the right flags."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="built game.py", stderr=""
        )
        cfg = AgentConfig(task="breakout", model="opus", max_budget_usd=8.0)
        launch_agent(cfg, project_root=root)

        args = mock_run.call_args
        cmd = args[0][0]

        assert cmd[0] == "claude"
        assert "-p" in cmd
        assert "--system-prompt" in cmd
        assert "--model" in cmd
        idx = cmd.index("--model")
        assert cmd[idx + 1] == "opus"
        assert "--dangerously-skip-permissions" in cmd
        assert "--max-budget-usd" in cmd
        idx = cmd.index("--max-budget-usd")
        assert cmd[idx + 1] == "8.0"
        assert "--add-dir" in cmd

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_strips_claudecode_env_var(
        self, mock_run: MagicMock, tmp_path: Path
    ) -> None:
        """CLAUDECODE env var must be removed before spawning the agent."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="ok", stderr=""
        )
        # Ensure CLAUDECODE is in env before call
        with patch.dict(os.environ, {"CLAUDECODE": "1"}):
            launch_agent(AgentConfig(task="breakout"), project_root=root)

        env = mock_run.call_args[1]["env"]
        assert "CLAUDECODE" not in env

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_timeout_handled_gracefully(
        self, mock_run: MagicMock, tmp_path: Path
    ) -> None:
        """TimeoutExpired should not raise — it returns an AgentRun with nonzero exit_code."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# Ref")

        mock_run.side_effect = subprocess.TimeoutExpired(
            "claude", 600, output=b"partial", stderr=b"timed out"
        )
        result = launch_agent(
            AgentConfig(task="breakout", timeout_s=600), project_root=root
        )

        assert result.exit_code != 0
        assert result.signal == "TIMEOUT"

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_prompt_contains_gdd_and_sdk_ref(
        self, mock_run: MagicMock, tmp_path: Path
    ) -> None:
        """The user prompt should contain GDD content and SDK reference."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD\nBuild breakout.")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK\nNomaiEngine API")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr=""
        )
        launch_agent(AgentConfig(task="breakout"), project_root=root)

        cmd = mock_run.call_args[0][0]
        # The prompt is the argument after -p
        idx = cmd.index("-p")
        prompt = cmd[idx + 1]
        assert "Breakout GDD" in prompt
        assert "NomaiEngine API" in prompt

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_returns_agent_run_with_results(
        self, mock_run: MagicMock, tmp_path: Path
    ) -> None:
        """launch_agent should return a properly populated AgentRun."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="game built", stderr="warnings"
        )
        result = launch_agent(AgentConfig(task="breakout"), project_root=root)

        assert isinstance(result, AgentRun)
        assert result.stdout == "game built"
        assert result.stderr == "warnings"
        assert result.exit_code == 0
        assert result.wall_time_s >= 0.0


# ---------------------------------------------------------------------------
# Module-level constants
# ---------------------------------------------------------------------------

class TestModuleConstants:
    def test_project_root_is_path(self) -> None:
        assert isinstance(PROJECT_ROOT, Path)

    def test_agent_system_prompt_is_nonempty_string(self) -> None:
        assert isinstance(AGENT_SYSTEM_PROMPT, str)
        assert len(AGENT_SYSTEM_PROMPT) > 100

    def test_agent_system_prompt_mentions_key_concepts(self) -> None:
        assert "game developer" in AGENT_SYSTEM_PROMPT.lower() or "game" in AGENT_SYSTEM_PROMPT
        assert "DONE.txt" in AGENT_SYSTEM_PROMPT
        assert "STUCK.txt" in AGENT_SYSTEM_PROMPT

    def test_agent_system_prompt_mentions_snapshot_json(self) -> None:
        assert "snapshot.json" in AGENT_SYSTEM_PROMPT


# ---------------------------------------------------------------------------
# score_game
# ---------------------------------------------------------------------------

class TestScoreGame:
    def _make_run(self, workdir: Path, exit_code: int = 0) -> AgentRun:
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir,
            exit_code=exit_code,
            wall_time_s=42.0,
            stdout="some output",
            stderr="",
        )

    def test_missing_game_script_returns_failed(self, tmp_path: Path) -> None:
        """No game.py → TaskResult.succeeded is False."""
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)
        tr = result["task_result"]
        assert isinstance(tr, TaskResult)
        assert tr.succeeded is False
        assert "not found" in result["ground_truth"]["error"]

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_broken_script_returns_failed(
        self, mock_run_script: MagicMock, tmp_path: Path
    ) -> None:
        """game.py that crashes → TaskResult.succeeded is False."""
        (tmp_path / "game.py").write_text("raise RuntimeError('boom')")
        mock_run_script.return_value = MagicMock(
            returncode=1, stdout="", stderr="RuntimeError: boom"
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)
        tr = result["task_result"]
        assert isinstance(tr, TaskResult)
        assert tr.succeeded is False
        assert result["ground_truth"]["error"] == "game.py crashed"

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_successful_script(
        self, mock_run_script: MagicMock, tmp_path: Path
    ) -> None:
        """Successful game.py with ENTITY_COUNT → TaskResult.succeeded is True."""
        (tmp_path / "game.py").write_text("print('ok')")
        stdout = "Running game...\nENTITY_COUNT: 24\nDone.\n"
        mock_run_script.return_value = MagicMock(
            returncode=0, stdout=stdout, stderr=""
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)
        tr = result["task_result"]
        assert isinstance(tr, TaskResult)
        assert tr.succeeded is True
        assert result["ground_truth"]["entity_count"] == 24
        # Both runs return identical stdout → deterministic
        assert tr.replay_deterministic is True

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_determinism_check(
        self, mock_run_script: MagicMock, tmp_path: Path
    ) -> None:
        """Two runs with different stdout → replay_deterministic is False."""
        (tmp_path / "game.py").write_text("print('ok')")
        run1 = MagicMock(returncode=0, stdout="ENTITY_COUNT: 10\nrun1", stderr="")
        run2 = MagicMock(returncode=0, stdout="ENTITY_COUNT: 10\nrun2", stderr="")
        mock_run_script.side_effect = [run1, run2]
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)
        tr = result["task_result"]
        assert tr.replay_deterministic is False


# ---------------------------------------------------------------------------
# run_agent_eval
# ---------------------------------------------------------------------------

class TestRunAgentEval:
    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_produces_report_dict(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """run_agent_eval returns a dict with expected top-level keys."""
        # Set up mock AgentRun
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

        # Set up mock score_game result
        mock_score.return_value = {
            "task_result": TaskResult(
                task_id="breakout",
                succeeded=True,
                replay_deterministic=True,
            ),
            "eval_report": None,
            "ground_truth": {"entity_count": 24, "replay_deterministic": True},
            "llm_scores": None,
        }

        report = run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            project_root=tmp_path,
        )

        assert "agent_meta" in report
        assert "task_result" in report
        assert "ground_truth" in report
        assert report["agent_meta"]["task"] == "breakout"
        assert report["agent_meta"]["model"] == "sonnet"
        assert report["task_result"]["succeeded"] is True
        # Verify report was saved
        assert (tmp_path / "eval_agent_report.json").exists()

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
            judge_model="sonnet",
            project_root=tmp_path,
        )

        assert "llm_scores" in report
        assert report["llm_scores"]["scene_qa_accuracy"] == 0.85
        # Verify score_game was called with judge_model
        mock_score.assert_called_once()
        call_kwargs = mock_score.call_args[1]
        assert call_kwargs["judge_model"] == "sonnet"


# ---------------------------------------------------------------------------
# score_game — LLM-judged deep scoring
# ---------------------------------------------------------------------------

class TestScoreGameDeepScoring:
    """Tests for LLM-judged deep scoring in score_game."""

    def _make_snapshot_dict(self) -> dict:
        return {
            "schema_version": 1, "tick": 300, "sim_time": 5.0,
            "entities": [
                {"entity_id": 1, "entity_type": "character", "role": "paddle",
                 "tier": "Foreground", "position": [400.0, 560.0],
                 "size": [100.0, 15.0], "velocity": None, "visible": True,
                 "z_index": 0.0, "components": {}},
                {"entity_id": 2, "entity_type": "projectile", "role": "ball",
                 "tier": "Foreground", "position": [350.0, 200.0],
                 "size": [8.0, 8.0], "velocity": [200.0, -300.0], "visible": True,
                 "z_index": 0.0, "components": {}},
            ],
            "bounds": {"min_x": 0, "min_y": 0, "max_x": 800, "max_y": 600},
            "entity_count": 2,
        }

    def _make_run(self, workdir, exit_code=0):
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir, exit_code=exit_code,
            wall_time_s=10.0, stdout="ENTITY_COUNT: 2", stderr="",
        )

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_no_snapshot_returns_null_llm_scores(self, mock_run_script, tmp_path):
        """When snapshot.json is missing, llm_scores should be None."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 2')")
        mock_run_script.return_value = MagicMock(returncode=0, stdout="ENTITY_COUNT: 2", stderr="")
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path, judge_model="sonnet")
        assert result["llm_scores"] is None

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_no_judge_model_returns_null_llm_scores(self, mock_run_script, tmp_path):
        """When judge_model is None, llm_scores should be None even if snapshot exists."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 2')")
        (tmp_path / "snapshot.json").write_text(json.dumps(self._make_snapshot_dict()))
        mock_run_script.return_value = MagicMock(returncode=0, stdout="ENTITY_COUNT: 2", stderr="")
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
        self, mock_run_script, mock_llm_cls, mock_gen_scene_q,
        mock_gen_spatial_q, mock_scene_qa, mock_geval, mock_multihop, tmp_path,
    ):
        """When snapshot.json exists and judge_model is set, run all three scorers."""
        from nomai.eval.metrics import EvalDimension, MetricResult

        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 2')")
        (tmp_path / "snapshot.json").write_text(json.dumps(self._make_snapshot_dict()))
        mock_run_script.return_value = MagicMock(returncode=0, stdout="ENTITY_COUNT: 2", stderr="")

        mock_llm = MagicMock()
        mock_llm_cls.return_value = mock_llm

        mock_gen_scene_q.return_value = []
        mock_scene_qa.return_value = MetricResult(
            name="scene_qa_accuracy", dimension=EvalDimension.OBSERVABILITY,
            value=0.85, target=0.8, passed=True, detail="17/20 correct",
        )

        mock_geval.return_value = [
            MetricResult(name=f"geval_{c}", dimension=EvalDimension.VERIFICATION,
                         value=v, target=None, passed=True, detail=f"{c} score")
            for c, v in [("completeness", 0.75), ("clarity", 0.80),
                         ("spatial_accuracy", 0.70), ("actionability", 0.65)]
        ]

        mock_gen_spatial_q.return_value = []
        mock_multihop.return_value = MetricResult(
            name="multihop_spatial_accuracy", dimension=EvalDimension.OBSERVABILITY,
            value=0.60, target=None, passed=True, detail="3/5 correct",
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
        mock_llm_cls.assert_called_once_with(model="sonnet")

    def test_early_return_no_game_has_llm_scores_none(self, tmp_path):
        """Early return when game.py missing should include llm_scores=None."""
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path, judge_model="sonnet")
        assert "llm_scores" in result
        assert result["llm_scores"] is None


# ---------------------------------------------------------------------------
# Structural validation (validate_breakout_snapshot)
# ---------------------------------------------------------------------------

def _make_entity(
    entity_id: int,
    entity_type: str,
    role: str,
    position: tuple[float, float] | None = None,
    size: tuple[float, float] | None = None,
    velocity: tuple[float, float] | None = None,
) -> SceneEntity:
    """Helper to create a SceneEntity for tests."""
    return SceneEntity(
        entity_id=entity_id,
        entity_type=entity_type,
        role=role,
        tier="Semantic",
        position=position,
        size=size,
        velocity=velocity,
        visible=True,
        z_index=0.0,
    )


def _make_good_breakout_snapshot() -> SceneSnapshot:
    """A valid breakout snapshot where all checks should pass."""
    entities = [
        _make_entity(0, "character", "paddle", (400, 560), (100, 15)),
        _make_entity(1, "projectile", "ball", (350, 200), (8, 8), (200, -300)),
    ]
    # 18 bricks (2 destroyed from initial 20)
    for i in range(18):
        entities.append(
            _make_entity(2 + i, "destructible", "brick", (260 + (i % 5) * 70, 60 + (i // 5) * 30), (60, 20))
        )
    # 3 walls
    entities.append(_make_entity(20, "boundary", "wall_top", (400, -10)))
    entities.append(_make_entity(21, "boundary", "wall_left", (-10, 300)))
    entities.append(_make_entity(22, "boundary", "wall_right", (810, 300)))

    from nomai.scene import SceneBounds
    return SceneSnapshot(
        schema_version=1,
        tick=301,
        sim_time=5.0,
        entities=entities,
        bounds=SceneBounds(min_x=-10, min_y=-10, max_x=810, max_y=600),
        entity_count=len(entities),
    )


class TestValidateBreakoutSnapshot:
    """Tests for structural validation of breakout game snapshots."""

    def test_good_snapshot_passes_all_checks(self) -> None:
        """A correct breakout game snapshot should pass all validation checks."""
        snap = _make_good_breakout_snapshot()
        result = validate_breakout_snapshot(snap)
        assert result.passed, f"Expected all checks to pass, failures: {result.failures}"
        assert len(result.failures) == 0

    def test_ball_out_of_bounds_fails(self) -> None:
        """Ball at y=1471 (fell through bottom) should fail ball_in_bounds."""
        snap = _make_good_breakout_snapshot()
        # Replace ball with one that's way off screen
        entities = [e for e in snap.entities if e.role != "ball"]
        entities.append(_make_entity(1, "projectile", "ball", (530, 1471), (8, 8), (8, 291)))
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "ball_in_bounds" in result.failures

    def test_no_bricks_destroyed_fails(self) -> None:
        """If all 20 bricks remain, bricks_destroyed check should fail."""
        entities = [
            _make_entity(0, "character", "paddle", (400, 560), (100, 15)),
            _make_entity(1, "projectile", "ball", (350, 200), (8, 8), (200, -300)),
        ]
        for i in range(20):  # all 20 bricks still alive
            entities.append(
                _make_entity(2 + i, "destructible", "brick", (260 + (i % 5) * 70, 60 + (i // 5) * 30), (60, 20))
            )
        entities.append(_make_entity(22, "boundary", "wall_top", (400, -10)))
        entities.append(_make_entity(23, "boundary", "wall_left", (-10, 300)))
        entities.append(_make_entity(24, "boundary", "wall_right", (810, 300)))

        from nomai.scene import SceneBounds
        snap = SceneSnapshot(
            schema_version=1, tick=301, sim_time=5.0, entities=entities,
            bounds=SceneBounds(min_x=-10, min_y=-10, max_x=810, max_y=600),
            entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "bricks_destroyed" in result.failures

    def test_missing_paddle_fails(self) -> None:
        """No paddle entity should fail has_paddle check."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "paddle"]
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "has_paddle" in result.failures

    def test_missing_ball_fails(self) -> None:
        """No ball entity should fail has_ball and ball_moved checks."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "ball"]
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "has_ball" in result.failures

    def test_ball_at_starting_position_fails(self) -> None:
        """Ball still at (400, 300) should fail ball_moved check."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "ball"]
        entities.append(_make_entity(1, "projectile", "ball", (400, 300), (8, 8)))
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "ball_moved" in result.failures

    def test_missing_walls_fails(self) -> None:
        """Fewer than 3 walls should fail has_walls check."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if not e.role.startswith("wall_")]
        # Only add 1 wall
        entities.append(_make_entity(20, "boundary", "wall_top", (400, -10)))
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "has_walls" in result.failures

    def test_haiku_broken_game_fails(self) -> None:
        """Reproduce exact Haiku game state — ball at (530, 1471), only 1 brick destroyed.

        This is the real scenario that exposed the eval gap: the game looked
        completely broken visually but the old eval said PASS.
        """
        entities = [
            _make_entity(0, "character", "paddle", (400, 560), (100, 15)),
            _make_entity(1, "projectile", "ball", (530.3, 1471.1), (8, 8), (8.0, 291.4)),
        ]
        # 19 bricks remaining (1 destroyed from 20)
        for i in range(19):
            entities.append(
                _make_entity(2 + i, "destructible", "brick", (260 + (i % 5) * 70, 60 + (i // 5) * 30), (60, 20))
            )
        entities.append(_make_entity(22, "boundary", "wall_top", (400, -10)))
        entities.append(_make_entity(23, "boundary", "wall_left", (-10, 300)))
        entities.append(_make_entity(24, "boundary", "wall_right", (810, 300)))

        from nomai.scene import SceneBounds
        snap = SceneSnapshot(
            schema_version=1, tick=301, sim_time=5.0, entities=entities,
            bounds=SceneBounds(min_x=-10, min_y=-10, max_x=810, max_y=1475),
            entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert not result.passed
        assert "ball_in_bounds" in result.failures
        # Ball moved (it's far from 400,300) so that should pass
        assert result.checks["ball_moved"] is True
        # Has entities — those pass
        assert result.checks["has_paddle"] is True
        assert result.checks["has_ball"] is True
        assert result.checks["has_walls"] is True


class TestScoreGameWithValidation:
    """Tests that score_game uses structural validation when snapshot exists."""

    def _make_run(self, workdir: Path, exit_code: int = 0) -> AgentRun:
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir, exit_code=exit_code,
            wall_time_s=10.0, stdout="ENTITY_COUNT: 24", stderr="",
        )

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_broken_snapshot_fails_score(self, mock_run_script: MagicMock, tmp_path: Path) -> None:
        """score_game should FAIL when snapshot shows ball out of bounds."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 24')")

        # Write the broken Haiku snapshot
        broken_snapshot = {
            "schema_version": 1, "tick": 301, "sim_time": 5.0,
            "entities": [
                {"entity_id": 0, "entity_type": "character", "role": "paddle",
                 "tier": "Semantic", "position": [400, 560], "size": [100, 15],
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 1, "entity_type": "projectile", "role": "ball",
                 "tier": "Semantic", "position": [530, 1471], "size": [8, 8],
                 "velocity": [8, 291], "visible": True, "z_index": 0, "components": {}},
            ] + [
                {"entity_id": 2 + i, "entity_type": "destructible", "role": "brick",
                 "tier": "Semantic", "position": [260 + (i % 5) * 70, 60 + (i // 5) * 30],
                 "size": [60, 20], "velocity": None, "visible": True, "z_index": 0, "components": {}}
                for i in range(19)
            ] + [
                {"entity_id": 22, "entity_type": "boundary", "role": "wall_top",
                 "tier": "Semantic", "position": [400, -10], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 23, "entity_type": "boundary", "role": "wall_left",
                 "tier": "Semantic", "position": [-10, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 24, "entity_type": "boundary", "role": "wall_right",
                 "tier": "Semantic", "position": [810, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
            ],
            "bounds": {"min_x": -10, "min_y": -10, "max_x": 810, "max_y": 1475},
            "entity_count": 24,
        }
        (tmp_path / "snapshot.json").write_text(json.dumps(broken_snapshot))

        mock_run_script.return_value = MagicMock(returncode=0, stdout="ENTITY_COUNT: 24", stderr="")
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)

        assert result["task_result"].succeeded is False
        assert result["ground_truth"]["validation_passed"] is False
        assert result["ground_truth"]["validation_checks"]["ball_in_bounds"] is False

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_good_snapshot_passes_score(self, mock_run_script: MagicMock, tmp_path: Path) -> None:
        """score_game should PASS when snapshot shows a healthy game state."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 22')")

        good_snapshot = {
            "schema_version": 1, "tick": 301, "sim_time": 5.0,
            "entities": [
                {"entity_id": 0, "entity_type": "character", "role": "paddle",
                 "tier": "Semantic", "position": [400, 560], "size": [100, 15],
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 1, "entity_type": "projectile", "role": "ball",
                 "tier": "Semantic", "position": [350, 200], "size": [8, 8],
                 "velocity": [200, -300], "visible": True, "z_index": 0, "components": {}},
            ] + [
                {"entity_id": 2 + i, "entity_type": "destructible", "role": "brick",
                 "tier": "Semantic", "position": [260 + (i % 5) * 70, 60 + (i // 5) * 30],
                 "size": [60, 20], "velocity": None, "visible": True, "z_index": 0, "components": {}}
                for i in range(16)  # 4 destroyed
            ] + [
                {"entity_id": 20, "entity_type": "boundary", "role": "wall_top",
                 "tier": "Semantic", "position": [400, -10], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 21, "entity_type": "boundary", "role": "wall_left",
                 "tier": "Semantic", "position": [-10, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 22, "entity_type": "boundary", "role": "wall_right",
                 "tier": "Semantic", "position": [810, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
            ],
            "bounds": {"min_x": -10, "min_y": -10, "max_x": 810, "max_y": 600},
            "entity_count": 21,
        }
        (tmp_path / "snapshot.json").write_text(json.dumps(good_snapshot))

        mock_run_script.return_value = MagicMock(returncode=0, stdout="ENTITY_COUNT: 22", stderr="")
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)

        assert result["task_result"].succeeded is True
        assert result["ground_truth"]["validation_passed"] is True


# ---------------------------------------------------------------------------
# Intent-style validation checks (new checks from breakout_intents)
# ---------------------------------------------------------------------------

class TestValidateBreakoutSnapshotIntentChecks:
    """Tests for the intent-aligned validation checks added to
    validate_breakout_snapshot: component presence, velocity bounds,
    and paddle position bounds."""

    def test_ball_velocity_within_bounds_passes(self) -> None:
        """Ball with velocity in [-500, 500] should pass ball_speed_bounded."""
        snap = _make_good_breakout_snapshot()
        result = validate_breakout_snapshot(snap)
        assert result.checks["ball_speed_bounded"] is True

    def test_ball_velocity_exceeds_bounds_fails(self) -> None:
        """Ball with dx=600 should fail ball_speed_bounded."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "ball"]
        entities.append(
            _make_entity(1, "projectile", "ball", (350, 200), (8, 8), (600, -300))
        )
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert result.checks["ball_speed_bounded"] is False
        assert "ball_speed_bounded" in result.failures

    def test_ball_velocity_negative_exceeds_bounds_fails(self) -> None:
        """Ball with dy=-550 should fail ball_speed_bounded."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "ball"]
        entities.append(
            _make_entity(1, "projectile", "ball", (350, 200), (8, 8), (100, -550))
        )
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert result.checks["ball_speed_bounded"] is False

    def test_paddle_within_bounds_passes(self) -> None:
        """Paddle at x=400 should pass paddle_in_bounds."""
        snap = _make_good_breakout_snapshot()
        result = validate_breakout_snapshot(snap)
        assert result.checks["paddle_in_bounds"] is True

    def test_paddle_out_of_bounds_fails(self) -> None:
        """Paddle at x=-50 should fail paddle_in_bounds."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "paddle"]
        entities.append(
            _make_entity(0, "character", "paddle", (-50, 560), (100, 15))
        )
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert result.checks["paddle_in_bounds"] is False
        assert "paddle_in_bounds" in result.failures

    def test_paddle_has_position_and_size(self) -> None:
        """Paddle must have position and size components (entity intent check)."""
        snap = _make_good_breakout_snapshot()
        result = validate_breakout_snapshot(snap)
        assert result.checks["paddle_has_components"] is True

    def test_paddle_missing_position_fails(self) -> None:
        """Paddle without position should fail paddle_has_components."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "paddle"]
        entities.append(
            _make_entity(0, "character", "paddle", None, (100, 15))
        )
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert result.checks["paddle_has_components"] is False

    def test_ball_has_position_and_velocity(self) -> None:
        """Ball must have position and velocity components."""
        snap = _make_good_breakout_snapshot()
        result = validate_breakout_snapshot(snap)
        assert result.checks["ball_has_components"] is True

    def test_ball_missing_velocity_fails(self) -> None:
        """Ball without velocity should fail ball_has_components."""
        snap = _make_good_breakout_snapshot()
        entities = [e for e in snap.entities if e.role != "ball"]
        entities.append(
            _make_entity(1, "projectile", "ball", (350, 200), (8, 8), None)
        )
        snap = SceneSnapshot(
            schema_version=snap.schema_version, tick=snap.tick,
            sim_time=snap.sim_time, entities=entities,
            bounds=snap.bounds, entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert result.checks["ball_has_components"] is False

    def test_bricks_have_position_and_size(self) -> None:
        """Bricks must have position and size components."""
        snap = _make_good_breakout_snapshot()
        result = validate_breakout_snapshot(snap)
        assert result.checks["bricks_have_components"] is True

    def test_bricks_missing_position_fails(self) -> None:
        """At least one brick without position should fail bricks_have_components."""
        entities = [
            _make_entity(0, "character", "paddle", (400, 560), (100, 15)),
            _make_entity(1, "projectile", "ball", (350, 200), (8, 8), (200, -300)),
        ]
        # 17 good bricks + 1 bad brick with no position
        for i in range(17):
            entities.append(
                _make_entity(2 + i, "destructible", "brick",
                             (260 + (i % 5) * 70, 60 + (i // 5) * 30), (60, 20))
            )
        entities.append(
            _make_entity(19, "destructible", "brick", None, (60, 20))
        )
        entities.append(_make_entity(20, "boundary", "wall_top", (400, -10)))
        entities.append(_make_entity(21, "boundary", "wall_left", (-10, 300)))
        entities.append(_make_entity(22, "boundary", "wall_right", (810, 300)))

        from nomai.scene import SceneBounds
        snap = SceneSnapshot(
            schema_version=1, tick=301, sim_time=5.0, entities=entities,
            bounds=SceneBounds(min_x=-10, min_y=-10, max_x=810, max_y=600),
            entity_count=len(entities),
        )
        result = validate_breakout_snapshot(snap)
        assert result.checks["bricks_have_components"] is False


class TestValidateBreakoutSnapshotIntentResults:
    """Tests that intent_results are included in ground_truth from score_game."""

    def _make_run(self, workdir: Path, exit_code: int = 0) -> AgentRun:
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir, exit_code=exit_code,
            wall_time_s=10.0, stdout="ENTITY_COUNT: 24", stderr="",
        )

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_ground_truth_has_intent_results(
        self, mock_run_script: MagicMock, tmp_path: Path,
    ) -> None:
        """score_game ground_truth should include intent_results dict."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 22')")

        good_snapshot = {
            "schema_version": 1, "tick": 301, "sim_time": 5.0,
            "entities": [
                {"entity_id": 0, "entity_type": "character", "role": "paddle",
                 "tier": "Semantic", "position": [400, 560], "size": [100, 15],
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 1, "entity_type": "projectile", "role": "ball",
                 "tier": "Semantic", "position": [350, 200], "size": [8, 8],
                 "velocity": [200, -300], "visible": True, "z_index": 0, "components": {}},
            ] + [
                {"entity_id": 2 + i, "entity_type": "destructible", "role": "brick",
                 "tier": "Semantic", "position": [260 + (i % 5) * 70, 60 + (i // 5) * 30],
                 "size": [60, 20], "velocity": None, "visible": True, "z_index": 0, "components": {}}
                for i in range(16)
            ] + [
                {"entity_id": 20, "entity_type": "boundary", "role": "wall_top",
                 "tier": "Semantic", "position": [400, -10], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 21, "entity_type": "boundary", "role": "wall_left",
                 "tier": "Semantic", "position": [-10, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 22, "entity_type": "boundary", "role": "wall_right",
                 "tier": "Semantic", "position": [810, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
            ],
            "bounds": {"min_x": -10, "min_y": -10, "max_x": 810, "max_y": 600},
            "entity_count": 21,
        }
        (tmp_path / "snapshot.json").write_text(json.dumps(good_snapshot))

        mock_run_script.return_value = MagicMock(
            returncode=0, stdout="ENTITY_COUNT: 22", stderr="",
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)

        gt = result["ground_truth"]
        assert "intent_results" in gt
        intent_results = gt["intent_results"]
        # Should contain intent-style check names
        assert "paddle_exists" in intent_results
        assert "ball_exists" in intent_results
        assert "bricks_exist" in intent_results
        assert "ball_speed_bounded" in intent_results
        assert "paddle_in_bounds" in intent_results
        # Each result should have passed and detail keys
        for name, res in intent_results.items():
            assert "passed" in res, f"intent_results[{name!r}] missing 'passed'"
            assert "detail" in res, f"intent_results[{name!r}] missing 'detail'"


class TestPromptContainsVerificationChecklist:
    """Tests that the agent prompt includes a verification checklist."""

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_prompt_has_verification_checklist(
        self, mock_run: MagicMock, tmp_path: Path,
    ) -> None:
        """The user prompt sent to the agent should include verification checklist."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr="",
        )
        launch_agent(AgentConfig(task="breakout"), project_root=root)

        cmd = mock_run.call_args[0][0]
        idx = cmd.index("-p")
        prompt = cmd[idx + 1]
        assert "Verification Checklist" in prompt
        # Should include at least some of the breakout intents
        assert "paddle_exists" in prompt
        assert "ball_exists" in prompt
        assert "bricks_exist" in prompt
        assert "ball_in_bounds" in prompt

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_prompt_checklist_has_descriptions(
        self, mock_run: MagicMock, tmp_path: Path,
    ) -> None:
        """Each checklist item should include the intent description."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr="",
        )
        launch_agent(AgentConfig(task="breakout"), project_root=root)

        cmd = mock_run.call_args[0][0]
        idx = cmd.index("-p")
        prompt = cmd[idx + 1]
        # The descriptions from breakout_intents should appear
        assert "paddle entity must exist" in prompt.lower() or "paddle" in prompt.lower()
        assert "ball entity must exist" in prompt.lower() or "ball" in prompt.lower()


# ---------------------------------------------------------------------------
# AgentConfig — max_iterations field
# ---------------------------------------------------------------------------

class TestAgentConfigMaxIterations:
    def test_default_max_iterations(self) -> None:
        cfg = AgentConfig(task="breakout")
        assert cfg.max_iterations == 3

    def test_custom_max_iterations(self) -> None:
        cfg = AgentConfig(task="breakout", max_iterations=5)
        assert cfg.max_iterations == 5


# ---------------------------------------------------------------------------
# _build_feedback_prompt
# ---------------------------------------------------------------------------

class TestBuildFeedbackPrompt:
    def test_feedback_prompt_contains_failures(self) -> None:
        """Feedback prompt should mention each failed check by name and detail."""
        validation = SnapshotValidation(
            checks={
                "has_paddle": True,
                "ball_in_bounds": False,
                "bricks_destroyed": False,
            },
            details={
                "has_paddle": "paddle count: 1",
                "ball_in_bounds": "ball at (530.0, 1471.0), bounds: (0-800, 0-600), margin: 50",
                "bricks_destroyed": "0/20 bricks destroyed",
            },
        )
        prompt = _build_feedback_prompt(1, validation, "Scene @ tick 300")

        assert "ball_in_bounds" in prompt
        assert "bricks_destroyed" in prompt
        assert "1471" in prompt
        assert "0/20" in prompt
        # Passing check should NOT be listed as a failure
        assert "- **has_paddle**" not in prompt

    def test_feedback_prompt_contains_snapshot_summary(self) -> None:
        """Feedback prompt should include the snapshot summary text."""
        validation = SnapshotValidation(
            checks={"ball_in_bounds": False},
            details={"ball_in_bounds": "out of bounds"},
        )
        summary = "Scene @ tick 300 (t=5.000s)\n  Entities: 24"
        prompt = _build_feedback_prompt(1, validation, summary)

        assert "Scene @ tick 300" in prompt
        assert "Entities: 24" in prompt

    def test_feedback_prompt_contains_iteration_number(self) -> None:
        """Feedback prompt should mention which iteration just completed."""
        prompt = _build_feedback_prompt(2, None, "no snapshot")
        assert "Iteration 2" in prompt

    def test_feedback_prompt_with_none_validation(self) -> None:
        """When validation is None, prompt should still be valid (no crash)."""
        prompt = _build_feedback_prompt(1, None, "not available")
        assert "Verification FAILED" in prompt
        assert "not available" in prompt

    def test_feedback_prompt_instructs_iteration(self) -> None:
        """Prompt should tell agent to iterate, not start from scratch."""
        prompt = _build_feedback_prompt(1, None, "summary")
        assert "do NOT start from scratch" in prompt


# ---------------------------------------------------------------------------
# launch_agent — feedback_prompt and workdir params
# ---------------------------------------------------------------------------

class TestLaunchAgentFeedback:
    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_launch_agent_with_feedback_appends_to_prompt(
        self, mock_run: MagicMock, tmp_path: Path,
    ) -> None:
        """When feedback_prompt is provided, it should appear in the -p argument."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr="",
        )
        feedback = "## Iteration 1 Results -- Verification FAILED\nball_in_bounds failed"
        launch_agent(
            AgentConfig(task="breakout"),
            project_root=root,
            feedback_prompt=feedback,
        )

        cmd = mock_run.call_args[0][0]
        idx = cmd.index("-p")
        prompt = cmd[idx + 1]
        assert "Iteration 1 Results" in prompt
        assert "ball_in_bounds failed" in prompt

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_launch_agent_without_feedback_has_no_feedback_section(
        self, mock_run: MagicMock, tmp_path: Path,
    ) -> None:
        """When feedback_prompt is None, prompt should not have iteration results."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# Breakout GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# SDK Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr="",
        )
        launch_agent(AgentConfig(task="breakout"), project_root=root)

        cmd = mock_run.call_args[0][0]
        idx = cmd.index("-p")
        prompt = cmd[idx + 1]
        assert "Iteration" not in prompt or "Verification FAILED" not in prompt

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_launch_agent_reuses_workdir(
        self, mock_run: MagicMock, tmp_path: Path,
    ) -> None:
        """When workdir is provided, launch_agent should use it instead of creating new."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# Ref")

        existing_workdir = tmp_path / "my_custom_workdir"
        existing_workdir.mkdir()

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr="",
        )
        result = launch_agent(
            AgentConfig(task="breakout"),
            project_root=root,
            workdir=existing_workdir,
        )

        assert result.workdir == existing_workdir
        # subprocess.run should have been called with cwd=str(existing_workdir)
        call_kwargs = mock_run.call_args[1]
        assert call_kwargs["cwd"] == str(existing_workdir)

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_launch_agent_creates_new_workdir_when_none(
        self, mock_run: MagicMock, tmp_path: Path,
    ) -> None:
        """When workdir is None, launch_agent should create a timestamped dir."""
        root = tmp_path / "project"
        root.mkdir()
        (root / "eval_tasks").mkdir()
        (root / "eval_tasks" / "breakout.md").write_text("# GDD")
        (root / "docs").mkdir()
        (root / "docs" / "ai").mkdir()
        (root / "docs" / "ai" / "nomai-sdk-reference.md").write_text("# Ref")

        mock_run.return_value = MagicMock(
            returncode=0, stdout="done", stderr="",
        )
        result = launch_agent(
            AgentConfig(task="breakout"),
            project_root=root,
            workdir=None,
        )

        # Should be under eval_workdir/<timestamp>
        assert result.workdir.parent == root / "eval_workdir"


# ---------------------------------------------------------------------------
# run_agent_eval — iteration loop
# ---------------------------------------------------------------------------

class TestRunAgentEvalIterationLoop:
    def _make_mock_agent_run(
        self, tmp_path: Path, succeeded: bool = True
    ) -> MagicMock:
        """Create a mock AgentRun for tests."""
        mock_run = MagicMock(spec=AgentRun)
        mock_run.exit_code = 0
        mock_run.wall_time_s = 10.0
        mock_run.signal = "DONE"
        mock_run.game_exists = True
        mock_run.workdir = tmp_path / "eval_workdir" / "mock_run"
        mock_run.workdir.mkdir(parents=True, exist_ok=True)
        mock_run.stdout = "agent output"
        mock_run.stderr = ""
        return mock_run

    def _make_score_result(
        self,
        succeeded: bool,
        *,
        with_validation: bool = False,
        failures: list[str] | None = None,
        snapshot_summary: str | None = None,
    ) -> dict:
        """Create a mock score_game return value."""
        gt: dict[str, object] = {
            "entity_count": 24,
            "replay_deterministic": True,
        }
        if snapshot_summary is not None:
            gt["snapshot_summary"] = snapshot_summary
        if with_validation:
            all_checks = {
                "has_paddle": True,
                "has_ball": True,
                "ball_in_bounds": True,
                "bricks_destroyed": True,
            }
            all_details: dict[str, str] = {
                "has_paddle": "paddle count: 1",
                "has_ball": "ball count: 1",
                "ball_in_bounds": "ball ok",
                "bricks_destroyed": "4/20 bricks destroyed",
            }
            for f in (failures or []):
                all_checks[f] = False
                all_details[f] = f"{f} check failed"
            gt["validation_passed"] = succeeded
            gt["validation_checks"] = all_checks
            gt["validation_details"] = all_details
            gt["intent_results"] = {
                name: {"passed": v, "detail": all_details.get(name, "")}
                for name, v in all_checks.items()
            }
        return {
            "task_result": TaskResult(
                task_id="breakout",
                succeeded=succeeded,
                replay_deterministic=True,
            ),
            "eval_report": None,
            "ground_truth": gt,
            "llm_scores": None,
        }

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_single_iteration_pass_on_first_try(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """If the first iteration passes, only 1 iteration should run."""
        mock_launch.return_value = self._make_mock_agent_run(tmp_path)
        mock_score.return_value = self._make_score_result(True)

        report = run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=3,
            project_root=tmp_path,
        )

        assert mock_launch.call_count == 1
        assert mock_score.call_count == 1
        assert report["agent_meta"]["iterations"] == 1
        assert report["agent_meta"]["converged"] is True

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_iterates_on_failure_then_passes(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """Fails on iteration 1, passes on iteration 2."""
        mock_launch.return_value = self._make_mock_agent_run(tmp_path)
        mock_score.side_effect = [
            self._make_score_result(
                False,
                with_validation=True,
                failures=["ball_in_bounds"],
                snapshot_summary="Scene @ tick 300",
            ),
            self._make_score_result(True),
        ]

        report = run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=3,
            project_root=tmp_path,
        )

        assert mock_launch.call_count == 2
        assert mock_score.call_count == 2
        assert report["agent_meta"]["iterations"] == 2
        assert report["agent_meta"]["converged"] is True

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_exhausts_max_iterations(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """Fails all iterations -- should run exactly max_iterations times."""
        mock_launch.return_value = self._make_mock_agent_run(tmp_path)
        mock_score.return_value = self._make_score_result(
            False,
            with_validation=True,
            failures=["ball_in_bounds"],
            snapshot_summary="Scene @ tick 300",
        )

        report = run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=2,
            project_root=tmp_path,
        )

        assert mock_launch.call_count == 2
        assert mock_score.call_count == 2
        assert report["agent_meta"]["iterations"] == 2
        assert report["agent_meta"]["max_iterations"] == 2
        assert report["agent_meta"]["converged"] is False
        assert report["task_result"]["succeeded"] is False

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_feedback_prompt_passed_on_retry(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """On iteration 2, launch_agent should receive a feedback_prompt."""
        mock_launch.return_value = self._make_mock_agent_run(tmp_path)
        mock_score.side_effect = [
            self._make_score_result(
                False,
                with_validation=True,
                failures=["ball_in_bounds"],
                snapshot_summary="Scene @ tick 300",
            ),
            self._make_score_result(True),
        ]

        run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=3,
            project_root=tmp_path,
        )

        # First call should have feedback_prompt=None
        first_call_kwargs = mock_launch.call_args_list[0][1]
        assert first_call_kwargs.get("feedback_prompt") is None

        # Second call should have a feedback_prompt containing failure info
        second_call_kwargs = mock_launch.call_args_list[1][1]
        feedback = second_call_kwargs.get("feedback_prompt")
        assert feedback is not None
        assert "ball_in_bounds" in feedback
        assert "Scene @ tick 300" in feedback

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_workdir_reused_on_retry(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """On iteration 2+, launch_agent should reuse the same workdir."""
        mock_run = self._make_mock_agent_run(tmp_path)
        mock_launch.return_value = mock_run
        mock_score.side_effect = [
            self._make_score_result(
                False,
                with_validation=True,
                failures=["ball_in_bounds"],
                snapshot_summary="Scene @ tick 300",
            ),
            self._make_score_result(True),
        ]

        run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=3,
            project_root=tmp_path,
        )

        # First call should have workdir=None (create new)
        first_call_kwargs = mock_launch.call_args_list[0][1]
        assert first_call_kwargs.get("workdir") is None

        # Second call should reuse the workdir from the first run
        second_call_kwargs = mock_launch.call_args_list[1][1]
        assert second_call_kwargs.get("workdir") == mock_run.workdir

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_backwards_compatible_single_iteration(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """Default max_iterations=3, but passes on first try means 1 iteration.

        This verifies backwards compatibility: existing callers that do not
        pass max_iterations should behave identically to the old single-shot
        flow.
        """
        mock_launch.return_value = self._make_mock_agent_run(tmp_path)
        mock_score.return_value = self._make_score_result(True)

        report = run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            project_root=tmp_path,
        )

        # Should run exactly once
        assert mock_launch.call_count == 1
        assert mock_score.call_count == 1
        assert report["agent_meta"]["iterations"] == 1
        assert report["task_result"]["succeeded"] is True

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_report_has_iteration_metadata(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """Report agent_meta should include iterations, max_iterations, converged."""
        mock_launch.return_value = self._make_mock_agent_run(tmp_path)
        mock_score.return_value = self._make_score_result(True)

        report = run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=5,
            project_root=tmp_path,
        )

        meta = report["agent_meta"]
        assert "iterations" in meta
        assert "max_iterations" in meta
        assert "converged" in meta
        assert meta["iterations"] == 1
        assert meta["max_iterations"] == 5
        assert meta["converged"] is True

    @patch("nomai.eval.agent_harness.score_game")
    @patch("nomai.eval.agent_harness.launch_agent")
    def test_per_iteration_logs_saved(
        self,
        mock_launch: MagicMock,
        mock_score: MagicMock,
        tmp_path: Path,
    ) -> None:
        """Each iteration should save agent_stdout_iterN.log and agent_stderr_iterN.log."""
        mock_run = self._make_mock_agent_run(tmp_path)
        mock_run.stdout = "iter output"
        mock_run.stderr = "iter errors"
        mock_launch.return_value = mock_run
        mock_score.side_effect = [
            self._make_score_result(
                False,
                with_validation=True,
                failures=["ball_in_bounds"],
                snapshot_summary="Scene @ tick 300",
            ),
            self._make_score_result(True),
        ]

        run_agent_eval(
            task="breakout",
            model="sonnet",
            max_budget_usd=5.0,
            max_iterations=3,
            project_root=tmp_path,
        )

        workdir = mock_run.workdir
        assert (workdir / "agent_stdout_iter1.log").exists()
        assert (workdir / "agent_stderr_iter1.log").exists()
        assert (workdir / "agent_stdout_iter2.log").exists()
        assert (workdir / "agent_stderr_iter2.log").exists()


# ---------------------------------------------------------------------------
# score_game — snapshot_summary in ground_truth
# ---------------------------------------------------------------------------

class TestScoreGameSnapshotSummary:
    """Tests that score_game includes snapshot_summary in ground_truth."""

    def _make_run(self, workdir: Path) -> AgentRun:
        return AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=workdir, exit_code=0,
            wall_time_s=10.0, stdout="ENTITY_COUNT: 22", stderr="",
        )

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_snapshot_summary_in_ground_truth(
        self, mock_run_script: MagicMock, tmp_path: Path,
    ) -> None:
        """When snapshot.json exists, ground_truth should have snapshot_summary."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 22')")

        good_snapshot = {
            "schema_version": 1, "tick": 301, "sim_time": 5.0,
            "entities": [
                {"entity_id": 0, "entity_type": "character", "role": "paddle",
                 "tier": "Semantic", "position": [400, 560], "size": [100, 15],
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 1, "entity_type": "projectile", "role": "ball",
                 "tier": "Semantic", "position": [350, 200], "size": [8, 8],
                 "velocity": [200, -300], "visible": True, "z_index": 0, "components": {}},
            ] + [
                {"entity_id": 2 + i, "entity_type": "destructible", "role": "brick",
                 "tier": "Semantic", "position": [260 + (i % 5) * 70, 60 + (i // 5) * 30],
                 "size": [60, 20], "velocity": None, "visible": True, "z_index": 0, "components": {}}
                for i in range(16)
            ] + [
                {"entity_id": 20, "entity_type": "boundary", "role": "wall_top",
                 "tier": "Semantic", "position": [400, -10], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 21, "entity_type": "boundary", "role": "wall_left",
                 "tier": "Semantic", "position": [-10, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
                {"entity_id": 22, "entity_type": "boundary", "role": "wall_right",
                 "tier": "Semantic", "position": [810, 300], "size": None,
                 "velocity": None, "visible": True, "z_index": 0, "components": {}},
            ],
            "bounds": {"min_x": -10, "min_y": -10, "max_x": 810, "max_y": 600},
            "entity_count": 21,
        }
        (tmp_path / "snapshot.json").write_text(json.dumps(good_snapshot))

        mock_run_script.return_value = MagicMock(
            returncode=0, stdout="ENTITY_COUNT: 22", stderr="",
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)

        gt = result["ground_truth"]
        assert "snapshot_summary" in gt
        assert "tick 301" in gt["snapshot_summary"]
        assert "paddle" in gt["snapshot_summary"]

    @patch("nomai.eval.agent_harness._run_game_script")
    def test_no_snapshot_no_summary(
        self, mock_run_script: MagicMock, tmp_path: Path,
    ) -> None:
        """When no snapshot.json, ground_truth should not have snapshot_summary."""
        (tmp_path / "game.py").write_text("print('ENTITY_COUNT: 5')")
        mock_run_script.return_value = MagicMock(
            returncode=0, stdout="ENTITY_COUNT: 5", stderr="",
        )
        run = self._make_run(tmp_path)
        result = score_game(run, project_root=tmp_path)
        assert "snapshot_summary" not in result["ground_truth"]
