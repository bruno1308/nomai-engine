"""Tests for nomai.gdd_pipeline -- GDD-to-verification pipeline runner.

Tests validate the public API (run_pipeline, load_spec, save_spec,
PipelineResult) and the private _slugify helper.  Filesystem tests
use pytest's tmp_path fixture to avoid polluting the working tree.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from nomai.gdd import (
    GameDesignSpec,
    PlayAreaSpec,
)
from nomai.gdd_pipeline import (
    PipelineResult,
    _slugify,
    load_spec,
    run_pipeline,
    save_spec,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _complete_breakout_dict() -> dict[str, object]:
    """Build a complete breakout spec as a plain dict.

    This spec passes CompletenessChecker with 0 questions because it
    has: play_area, bounds on all dynamic/kinematic entities, speed_max
    on all dynamic entities, interactions covering every movable pair,
    and at least one degenerate state.
    """
    return {
        "title": "Breakout",
        "description": "Classic breakout clone with paddle, ball, and bricks",
        "play_area": {"width": 800.0, "height": 600.0},
        "entities": [
            {
                "name": "paddle",
                "entity_type": "character",
                "role": "paddle",
                "body_type": "kinematic",
                "bounds": {"y_min": 550.0, "y_max": 550.0},
                "speed_max": 8.0,
                "required_components": ["position", "size", "velocity"],
            },
            {
                "name": "ball",
                "entity_type": "projectile",
                "role": "ball",
                "body_type": "dynamic",
                "bounds": {
                    "x_min": 0.0,
                    "x_max": 800.0,
                    "y_min": 0.0,
                    "y_max": 600.0,
                },
                "speed_max": 10.0,
                "required_components": ["position", "velocity"],
            },
            {
                "name": "brick",
                "entity_type": "obstacle",
                "role": "brick",
                "body_type": "static",
                "required_components": ["position", "size", "health"],
            },
        ],
        "interactions": [
            {
                "entity_a": "ball",
                "entity_b": "paddle",
                "behavior": "bounce",
                "description": "Ball bounces off the paddle, reversing y-velocity",
            },
            {
                "entity_a": "ball",
                "entity_b": "brick",
                "behavior": "destroy",
                "description": "Ball destroys brick on contact",
            },
            {
                "entity_a": "ball",
                "entity_b": "wall",
                "behavior": "bounce",
                "description": "Ball bounces off walls",
            },
            {
                "entity_a": "paddle",
                "entity_b": "brick",
                "behavior": "none",
                "description": "Paddle and brick do not interact directly",
            },
        ],
        "invariants": [
            {
                "name": "ball_in_bounds",
                "entity": "ball",
                "component": "position",
                "field": "x",
                "condition": ">= 0 and <= 800",
                "description": "Ball x-position must stay within the play area",
            },
        ],
        "degenerate_states": [
            {
                "name": "ball_stuck",
                "entity": "ball",
                "component": "velocity",
                "field": "dy",
                "condition": "== 0",
                "description": "Ball y-velocity should never be zero during play",
            },
        ],
        "win_condition": "All bricks destroyed",
        "lose_condition": "Ball falls below paddle",
    }


def _incomplete_spec_dict() -> dict[str, object]:
    """Build a minimal incomplete spec dict.

    Missing play_area, bounds, speed_max, interactions, and
    degenerate_states -- guaranteed to produce multiple questions.
    """
    return {
        "title": "Minimal",
        "description": "Bare minimum spec",
        "entities": [
            {
                "name": "ball",
                "entity_type": "projectile",
                "role": "ball",
                "body_type": "dynamic",
                "required_components": ["position"],
            },
            {
                "name": "paddle",
                "entity_type": "character",
                "role": "paddle",
                "body_type": "kinematic",
                "required_components": ["position"],
            },
        ],
    }


# ---------------------------------------------------------------------------
# TestSlugify
# ---------------------------------------------------------------------------

class TestSlugify:
    """Tests for the private _slugify helper."""

    def test_normal_title(self) -> None:
        """Spaces are replaced with underscores and text is lowered."""
        assert _slugify("My Cool Game") == "my_cool_game"

    def test_special_characters(self) -> None:
        """Non-alphanumeric characters are collapsed into underscores."""
        assert _slugify("Match-3 Puzzle!") == "match_3_puzzle"

    def test_empty_string(self) -> None:
        """Empty input returns the fallback 'untitled'."""
        assert _slugify("") == "untitled"

    def test_only_special_chars(self) -> None:
        """Input with only special chars returns 'untitled'."""
        assert _slugify("!!!@@@") == "untitled"

    def test_already_slugified(self) -> None:
        """A simple lowercase word passes through unchanged."""
        assert _slugify("breakout") == "breakout"


# ---------------------------------------------------------------------------
# TestLoadSpec
# ---------------------------------------------------------------------------

class TestLoadSpec:
    """Tests for the load_spec public function."""

    def test_load_from_json_file(self, tmp_path: Path) -> None:
        """Create a spec, save to a temp JSON file, then load it back."""
        # Arrange
        spec = GameDesignSpec(
            title="Round Trip",
            description="Testing load_spec",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
        )
        json_path = tmp_path / "spec.json"
        json_path.write_text(spec.to_json(), encoding="utf-8")

        # Act
        loaded = load_spec(json_path)

        # Assert
        assert loaded.title == "Round Trip"
        assert loaded == spec

    def test_load_missing_file_raises(self, tmp_path: Path) -> None:
        """load_spec raises FileNotFoundError for a nonexistent path."""
        with pytest.raises(FileNotFoundError):
            load_spec(tmp_path / "nonexistent.json")

    def test_load_prose_file_raises(self, tmp_path: Path) -> None:
        """Passing a .md file to run_pipeline raises ValueError with 'parse-gdd'."""
        # Arrange
        md_file = tmp_path / "design.md"
        md_file.write_text("# Game Design\nSome prose here.", encoding="utf-8")

        # Act / Assert
        with pytest.raises(ValueError, match="parse-gdd"):
            run_pipeline(str(md_file))


# ---------------------------------------------------------------------------
# TestSaveSpec
# ---------------------------------------------------------------------------

class TestSaveSpec:
    """Tests for the save_spec public function."""

    def test_save_creates_file(self, tmp_path: Path) -> None:
        """save_spec writes a JSON file and its content round-trips."""
        # Arrange
        spec = GameDesignSpec(
            title="Save Test",
            description="Testing save_spec",
        )
        target = tmp_path / "spec.json"

        # Act
        result_path = save_spec(spec, target)

        # Assert
        assert result_path.exists()
        loaded = GameDesignSpec.from_json(
            result_path.read_text(encoding="utf-8")
        )
        assert loaded.title == "Save Test"
        assert loaded == spec

    def test_save_creates_parent_dirs(self, tmp_path: Path) -> None:
        """save_spec creates intermediate parent directories."""
        # Arrange
        spec = GameDesignSpec(title="Nested Save")
        target = tmp_path / "deep" / "nested" / "spec.json"

        # Act
        result_path = save_spec(spec, target)

        # Assert
        assert result_path.exists()
        loaded = GameDesignSpec.from_json(
            result_path.read_text(encoding="utf-8")
        )
        assert loaded.title == "Nested Save"


# ---------------------------------------------------------------------------
# TestRunPipeline
# ---------------------------------------------------------------------------

class TestRunPipeline:
    """Tests for the run_pipeline orchestration function."""

    def test_complete_spec_returns_suite(self, tmp_path: Path) -> None:
        """A complete breakout spec dict produces a suite with no questions."""
        # Arrange
        spec_dict = _complete_breakout_dict()

        # Act
        result = run_pipeline(spec_dict, output_dir=tmp_path)

        # Assert
        assert isinstance(result, PipelineResult)
        assert result.suite is not None
        assert len(result.questions) == 0
        assert result.spec_path is not None
        assert result.spec_path.exists()
        assert result.suite_path is not None
        assert result.suite_path.exists()

    def test_incomplete_spec_returns_questions(self, tmp_path: Path) -> None:
        """A minimal incomplete spec dict produces questions and no suite."""
        # Arrange
        spec_dict = _incomplete_spec_dict()

        # Act
        result = run_pipeline(spec_dict, output_dir=tmp_path)

        # Assert
        assert len(result.questions) >= 4
        assert result.suite is None

    def test_save_false_skips_disk(self, tmp_path: Path) -> None:
        """save=False suppresses all file output."""
        # Arrange
        spec_dict = _complete_breakout_dict()

        # Act
        result = run_pipeline(spec_dict, output_dir=tmp_path, save=False)

        # Assert
        assert result.spec_path is None
        assert result.suite_path is None
        # No files should have been written under tmp_path
        assert list(tmp_path.iterdir()) == []

    def test_custom_output_dir(self, tmp_path: Path) -> None:
        """Files are saved into the explicit output_dir."""
        # Arrange
        spec_dict = _complete_breakout_dict()
        custom_dir = tmp_path / "custom"

        # Act
        result = run_pipeline(spec_dict, output_dir=custom_dir)

        # Assert
        assert result.spec_path is not None
        assert result.spec_path.parent == custom_dir
        assert result.spec_path.exists()
        assert result.suite_path is not None
        assert result.suite_path.exists()

    def test_saves_questions_json(self, tmp_path: Path) -> None:
        """Incomplete spec run saves a questions.json with non-empty list."""
        # Arrange
        spec_dict = _incomplete_spec_dict()

        # Act
        result = run_pipeline(spec_dict, output_dir=tmp_path)

        # Assert
        questions_file = tmp_path / "questions.json"
        assert questions_file.exists()
        questions_data = json.loads(
            questions_file.read_text(encoding="utf-8")
        )
        assert isinstance(questions_data, list)
        assert len(questions_data) > 0


# ---------------------------------------------------------------------------
# TestPipelineRoundTrip
# ---------------------------------------------------------------------------

class TestPipelineRoundTrip:
    """End-to-end: run pipeline, save, then reload and verify."""

    def test_save_and_reload(self, tmp_path: Path) -> None:
        """Pipeline output can be reloaded via load_spec."""
        # Arrange
        spec_dict = _complete_breakout_dict()

        # Act
        result = run_pipeline(spec_dict, output_dir=tmp_path)
        assert result.spec_path is not None
        reloaded = load_spec(result.spec_path)

        # Assert
        assert reloaded.title == "Breakout"
        assert reloaded == result.spec
