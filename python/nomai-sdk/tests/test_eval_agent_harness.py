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
    launch_agent,
    run_agent_eval,
    score_game,
)
from nomai.eval.autonomy import TaskResult


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
