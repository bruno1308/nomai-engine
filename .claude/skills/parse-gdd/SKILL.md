---
name: parse-gdd
description: Parse a prose Game Design Document into a structured GameDesignSpec, check completeness, and generate a VerificationSuite.
---

# Parse GDD

Parse a prose Game Design Document into a structured `GameDesignSpec`, iteratively check for completeness, and generate a `VerificationSuite` for AI-driven verification.

## Arguments

The skill takes one argument: the path to the GDD file (markdown or plain text).

Example: `/parse-gdd docs/match3-gdd.md`

## Steps

1. **Read the GDD file** at the path provided as argument. If no path is given, ask the user for the file path.

2. **Extract a `GameDesignSpec`** from the prose. Analyze the GDD and construct a JSON object matching the schema below. Write the extracted JSON to `python/nomai-sdk/specs/{game-name}/spec.json`.

   ### Extraction Guidelines
   - Map game entities to `EntitySpec` entries. Infer `body_type` from context:
     - "moves freely" / "physics-driven" -> `"dynamic"`
     - "player controls directly" -> `"kinematic"`
     - "stationary" / "doesn't move" -> `"static"`
   - Map collision/interaction descriptions to `InteractionSpec` entries using allowed behaviors:
     - Ball bounces off paddle -> `"bounce"`
     - Object reflects/ricochets -> `"reflect"`
     - Object is destroyed/removed -> `"destroy"`
     - Ball bounces off brick AND brick is destroyed -> `"reflect_and_destroy"`
     - Objects pass through / no interaction -> `"none"`
   - Map "must always be true" rules to `InvariantSpec`
   - Map "should never happen" / "bad states" to `DegenerateStateSpec`
   - Do NOT invent numeric values (speeds, dimensions, bounds). Leave them unset and let the CompletenessChecker surface them as questions.
   - Default `required_components`: movable entities -> `["position", "velocity"]`, sized entities -> `["position", "size"]`, both -> `["position", "velocity", "size"]`

3. **Run the pipeline**:
   ```python
   import sys
   sys.path.insert(0, "python/nomai-sdk")
   from nomai.gdd_pipeline import run_pipeline
   result = run_pipeline("python/nomai-sdk/specs/{game-name}/spec.json")
   ```

4. **Handle completeness gaps**: If `result.questions` is non-empty:
   - Present each question to the user, grouped by category and severity (high first)
   - Use the AskUserQuestion tool or direct questions to get answers
   - Update `spec.json` with the new information (add bounds, interactions, speeds, etc.)
   - Re-run step 3
   - Repeat until `result.questions` is empty

5. **Report results** when the spec is complete and the suite is generated.

## Schema Reference

### GameDesignSpec
| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| title | string | yes | -- | Game title |
| description | string | no | "" | Brief description |
| play_area | PlayAreaSpec | no | null | Game world dimensions |
| entities | EntitySpec[] | no | [] | Game entities |
| interactions | InteractionSpec[] | no | [] | Entity interaction rules |
| invariants | InvariantSpec[] | no | [] | Rules that must always hold |
| degenerate_states | DegenerateStateSpec[] | no | [] | Bad states to detect |
| win_condition | string | no | "" | How the player wins |
| lose_condition | string | no | "" | How the player loses |

### PlayAreaSpec
| Field | Type | Required |
|-------|------|----------|
| width | float | yes |
| height | float | yes |

### EntitySpec
| Field | Type | Required | Default |
|-------|------|----------|---------|
| name | string | yes | -- |
| entity_type | string | yes | -- |
| role | string | yes | -- |
| body_type | string | no | null |
| bounds | BoundsSpec | no | null |
| speed_max | float | no | null |
| required_components | string[] | no | [] |

### BoundsSpec
| Field | Type | Required |
|-------|------|----------|
| x_min | float | no |
| x_max | float | no |
| y_min | float | no |
| y_max | float | no |

### InteractionSpec
| Field | Type | Required | Default |
|-------|------|----------|---------|
| entity_a | string | yes | -- |
| entity_b | string | yes | -- |
| behavior | string | yes | -- |
| description | string | no | "" |

**Allowed `behavior` values:** `"bounce"`, `"reflect"`, `"destroy"`, `"reflect_and_destroy"`, `"none"`

### InvariantSpec
| Field | Type | Required | Default |
|-------|------|----------|---------|
| name | string | yes | -- |
| entity | string | yes | -- |
| component | string | yes | -- |
| field | string | yes | -- |
| condition | string | yes | -- |
| description | string | no | "" |

### DegenerateStateSpec
| Field | Type | Required | Default |
|-------|------|----------|---------|
| name | string | yes | -- |
| entity | string | yes | -- |
| component | string | yes | -- |
| field | string | yes | -- |
| condition | string | yes | -- |
| description | string | no | "" |

**Allowed `body_type` values:** `"static"`, `"dynamic"`, `"kinematic"`

## On Failure

- **File not found**: Report the error and ask for the correct path.
- **Malformed spec JSON**: Show the parse error, fix the JSON, and retry.
- **Pipeline exception**: Show the traceback and ask the user for guidance.

## Report

After completion, report:
```
GDD Parse: COMPLETE
  Game: {title}
  Entities: {count}
  Interactions: {count}
  Invariants: {count}
  Degenerate states: {count}
  Completeness: PASS (0 questions)
  Suite: {count} intents ({entity} entity, {behavior} behavior, {metric} metric, {invariant} invariant)
  Output: python/nomai-sdk/specs/{game-name}/
```
