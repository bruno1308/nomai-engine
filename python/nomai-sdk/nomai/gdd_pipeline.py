"""GDD-to-verification pipeline runner for the Nomai engine.

Orchestrates the full pipeline from a Game Design Document specification
to a verification suite:

1. Load a ``GameDesignSpec`` from a JSON file or dict.
2. Run the ``CompletenessChecker`` to identify gaps.
3. If the spec is complete, generate a ``VerificationSuite`` via
   ``IntentGenerator``.
4. Optionally save all artifacts (spec, questions, suite) to disk.

Usage::

    from nomai.gdd_pipeline import run_pipeline

    result = run_pipeline("path/to/spec.json")
    if result.suite is not None:
        print(f"Generated {len(result.suite.intents)} intents")
    else:
        print(f"{len(result.questions)} questions need answers first")
"""

from __future__ import annotations

import json
import logging
import re
from dataclasses import dataclass
from pathlib import Path

from nomai.gdd import (
    ClarificationQuestion,
    CompletenessChecker,
    GameDesignSpec,
    IntentGenerator,
)
from nomai.intents import VerificationSuite

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# PipelineResult
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class PipelineResult:
    """Result of running the GDD-to-verification pipeline.

    Attributes:
        spec: The loaded game design specification.
        questions: Clarification questions raised by the completeness checker.
            An empty list means the spec is complete.
        suite: The generated verification suite, or ``None`` if the spec
            is incomplete (i.e. ``questions`` is non-empty).
        spec_path: Filesystem path where the spec was saved, or ``None``
            if saving was skipped.
        suite_path: Filesystem path where the suite was saved, or ``None``
            if saving was skipped or no suite was generated.
    """
    spec: GameDesignSpec
    questions: list[ClarificationQuestion]
    suite: VerificationSuite | None
    spec_path: Path | None
    suite_path: Path | None


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def run_pipeline(
    source: str | Path | dict[str, object],
    output_dir: str | Path | None = None,
    save: bool = True,
) -> PipelineResult:
    """Run the GDD pipeline: load, check, generate, save.

    Args:
        source: A ``GameDesignSpec`` dict, a path to a spec JSON file,
            or a string path to a spec JSON file.
        output_dir: Directory to save artifacts into.  Defaults to
            ``python/nomai-sdk/specs/{slug}/`` based on the spec title.
        save: Whether to save artifacts to disk.  Defaults to ``True``.

    Returns:
        A :class:`PipelineResult` with the spec, questions, and
        (optionally) the generated verification suite.

    Raises:
        ValueError: If ``source`` is a prose file (.md, .txt, .markdown).
        FileNotFoundError: If ``source`` is a path that does not exist.
    """
    # Step 1: Load spec
    spec = _load_source(source)
    logger.info("Loaded spec: %s", spec.title)

    # Step 2: Check completeness
    checker = CompletenessChecker()
    questions = checker.check(spec)
    if questions:
        logger.info(
            "Spec '%s' has %d clarification question(s); skipping intent generation",
            spec.title, len(questions),
        )
    else:
        logger.info("Spec '%s' is complete", spec.title)

    # Step 3: Generate suite if complete
    suite: VerificationSuite | None = None
    if not questions:
        generator = IntentGenerator()
        suite = generator.generate(spec)
        warnings = suite.validate()
        for warning in warnings:
            logger.warning("Suite validation: %s", warning)
        logger.info(
            "Generated suite '%s' with %d intent(s)",
            suite.name, len(suite.intents),
        )

    # Step 4: Save artifacts
    spec_path: Path | None = None
    suite_path: Path | None = None

    if save:
        out_dir = _resolve_output_dir(output_dir, spec.title)
        out_dir.mkdir(parents=True, exist_ok=True)
        logger.info("Saving artifacts to %s", out_dir)

        # Save spec
        spec_path = out_dir / "spec.json"
        spec_path.write_text(spec.to_json(), encoding="utf-8")
        logger.info("Saved spec to %s", spec_path)

        # Save questions
        questions_path = out_dir / "questions.json"
        questions_path.write_text(
            json.dumps([q.to_dict() for q in questions], indent=2),
            encoding="utf-8",
        )
        logger.info("Saved %d question(s) to %s", len(questions), questions_path)

        # Save suite
        if suite is not None:
            suite_path = out_dir / "suite.json"
            suite.save(suite_path)
            logger.info("Saved suite to %s", suite_path)

    # Step 5: Return result
    return PipelineResult(
        spec=spec,
        questions=questions,
        suite=suite,
        spec_path=spec_path,
        suite_path=suite_path,
    )


def load_spec(path: str | Path) -> GameDesignSpec:
    """Load a GameDesignSpec from a JSON file.

    Args:
        path: Filesystem path to a JSON file containing a serialized
            ``GameDesignSpec``.

    Returns:
        The deserialized ``GameDesignSpec``.

    Raises:
        FileNotFoundError: If the file does not exist.
    """
    p = Path(path)
    if not p.exists():
        msg = f"Spec file not found: {p}"
        raise FileNotFoundError(msg)
    text = p.read_text(encoding="utf-8")
    return GameDesignSpec.from_json(text)


def save_spec(spec: GameDesignSpec, path: str | Path) -> Path:
    """Save a GameDesignSpec to a JSON file, creating parent dirs.

    Args:
        spec: The game design specification to save.
        path: Filesystem path to write the JSON file to.

    Returns:
        The resolved :class:`Path` where the file was written.
    """
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(spec.to_json(), encoding="utf-8")
    return p.resolve()


# ---------------------------------------------------------------------------
# Private helpers
# ---------------------------------------------------------------------------

_PROSE_SUFFIXES: frozenset[str] = frozenset({".md", ".txt", ".markdown"})


def _slugify(title: str) -> str:
    """Convert a game title to a filesystem-safe slug.

    Lowercases the title, replaces non-alphanumeric characters with
    underscores, collapses consecutive underscores, and strips leading
    and trailing underscores.  Returns ``"untitled"`` for empty input.

    Args:
        title: The game title to slugify.

    Returns:
        A filesystem-safe slug string.
    """
    slug = title.lower()
    slug = re.sub(r"[^a-z0-9]+", "_", slug)
    slug = slug.strip("_")
    if not slug:
        return "untitled"
    return slug


def _load_source(source: str | Path | dict[str, object]) -> GameDesignSpec:
    """Load a GameDesignSpec from a dict or JSON file path.

    Args:
        source: Either a dict to pass to ``GameDesignSpec.from_dict()``,
            or a string/Path pointing to a JSON file.

    Returns:
        The loaded ``GameDesignSpec``.

    Raises:
        ValueError: If ``source`` is a prose file (.md, .txt, .markdown)
            that must be parsed by the /parse-gdd skill first.
        FileNotFoundError: If ``source`` is a path that does not exist.
    """
    if isinstance(source, dict):
        return GameDesignSpec.from_dict(source)

    p = Path(source)
    if p.suffix.lower() in _PROSE_SUFFIXES:
        msg = (
            f"Prose GDD files ({p.name}) must be parsed by the "
            f"/parse-gdd skill first. Pass a spec JSON file or dict "
            f"to run_pipeline()."
        )
        raise ValueError(msg)

    return load_spec(p)


def _resolve_output_dir(output_dir: str | Path | None, title: str) -> Path:
    """Resolve the output directory for pipeline artifacts.

    Args:
        output_dir: Explicit output directory, or ``None`` to use the
            default location.
        title: The game title, used to derive a slug for the default path.

    Returns:
        The resolved output directory as a :class:`Path`.
    """
    if output_dir is not None:
        return Path(output_dir)
    return Path("python/nomai-sdk/specs") / _slugify(title)
