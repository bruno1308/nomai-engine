# Nomai Engine v8 -- MVP Specification

**Version 8.0 -- February 2026**
*Reframed around the verification thesis after 5-iteration dual-model review + adversarial red-team pass*

---

## Table of Contents

1. [The Problem](#1-the-problem)
2. [The Thesis](#2-the-thesis)
3. [The Verification Loop](#3-the-verification-loop)
4. [Architecture Overview](#4-architecture-overview)
5. [The Manifest Pipeline](#5-the-manifest-pipeline)
6. [Intent Specifications](#6-intent-specifications)
7. [Verification Engine](#7-verification-engine)
8. [Rust Engine Core](#8-rust-engine-core)
9. [WASM Gameplay Sandbox](#9-wasm-gameplay-sandbox)
10. [Deterministic Replay](#10-deterministic-replay)
11. [Python AI Interface](#11-python-ai-interface)
12. [Tech Stack](#12-tech-stack)
13. [MVP Scope and Deliverable](#13-mvp-scope-and-deliverable)
14. [Development Roadmap](#14-development-roadmap)
15. [Success Criteria](#15-success-criteria)
16. [Risks and Mitigations](#16-risks-and-mitigations)
17. [Post-MVP Path](#17-post-mvp-path)
18. [What Changed from v7](#18-what-changed-from-v7)

---

## 1. The Problem

AI can write game code. That part is trivial and getting more trivial every month. LLMs generate working gameplay logic, state machines, entity behaviors, and physics interactions reliably. Code generation is not the bottleneck.

**The bottleneck is verification.** When a human developer writes game code, they press play, watch the screen, and instantly see if something is wrong -- a misplaced sprite, a physics glitch, a missing animation, a broken interaction. The human brain processes visual output and compares it to intent in milliseconds. This feedback loop is what makes iterative game development work.

AI has no equivalent capability. Today's approaches all fail:

| Approach | Why It Fails |
|----------|-------------|
| **Screenshot + vision model** | Lossy, expensive, slow, no causality. "The sprite is in the wrong place" tells you nothing about which system moved it or why. No temporal awareness across frames. Cannot detect subtle behavioral bugs (wrong state transition, off-by-one in scoring). |
| **Unit tests on game logic** | Tests what code *computes*, not what the player *sees*. A function can return the correct value while the entity is invisible, offscreen, obscured, or spawned in the wrong layer. Tests cover code paths, not player experience. |
| **Integration tests** | Verify subsystem contracts, not emergent behavior. "Physics works" and "rendering works" does not mean "the ball bounces off the paddle and the player can see it." |
| **Human in the loop** | The developer plays the game, identifies what's wrong, and tells the AI. This is exactly the bottleneck we must eliminate. Every human verification step is a synchronous block on the AI development loop. |

The gap is **semantic awareness of rendered output**. AI needs to understand *what is on screen, why it's there, what state it's in, and what caused it to change* -- all without looking at pixels.

---

## 2. The Thesis

> **Structured semantic access to rendered game state enables AI to verify its own game development work, closing the development loop without human engineering intervention.**

This is a testable hypothesis with a concrete experiment (Section 13).

### The Four Pillars of AI Verification

Verification is the goal. Four capabilities are required to achieve it -- all are essential, none is optional:

| Pillar | What It Enables | Without It |
|--------|----------------|------------|
| **Observability** | AI can see what the engine is doing at a semantic level -- not pixels, but meaning | AI is blind. It generates code into a void and cannot evaluate the result. |
| **Controllability** | AI can manipulate engine state, spawn entities, modify behaviors, configure systems through structured APIs | AI can watch but not act. It sees problems but cannot fix them. The development loop is open. |
| **Reproducibility** | AI can reliably reproduce conditions -- same inputs produce same outputs, snapshots can be restored, experiments can be re-run | AI cannot regression-test, cannot A/B compare, cannot diagnose intermittent bugs. Every verification is one-shot. |
| **Autonomy** | AI can iterate independently -- write, verify, fix, deploy without human gatekeeping each step | AI has the capability but not the permission. Every iteration waits for human approval. |

These four pillars are not independent -- they form a dependency chain: **Observability enables Verification. Controllability enables Fixing. Reproducibility enables Regression Testing. Autonomy enables Iteration Speed.** Remove any one and the loop breaks.

The **manifest pipeline** is the architectural expression of Observability -- and it is the hardest to get right, because it requires the engine to maintain a semantic model of the game world that AI can reason about. Controllability, reproducibility, and autonomy are primarily engineering challenges with well-understood solutions. Observability at the level of *semantic awareness of rendered output* is the research problem this engine solves.

### Corollaries

1. **If AI can verify, it can iterate.** Verification failures produce structured diagnostics with causality chains. The AI doesn't need a human to explain what went wrong -- it can read the manifest, trace the cause, and generate a fix.

2. **If AI can iterate, it can develop autonomously.** The human provides direction ("make breakout"), not engineering ("fix the collision detection in line 47"). The AI handles the full write-verify-fix loop.

3. **The manifest is the product.** The engine exists to produce a structured, queryable, causal representation of the game world that AI can reason about. Everything else -- ECS, physics, rendering, replay -- serves this purpose.

### What This Means Architecturally

The manifest is not a debug view. It is not an afterthought. It is the **primary output of the engine**, co-equal with the framebuffer. Every architectural decision is evaluated against one question: *does this make the manifest more useful for AI verification?*

Controllability is baked into every API surface -- the Python interface, the WASM host API, the command buffer. Reproducibility is baked into the tick loop, snapshot system, and deterministic replay. Autonomy is baked into the hot-swap model and the verification-driven fix loop. But observability -- semantic awareness -- is the hard problem, and the manifest pipeline is its solution.

---

## 3. The Verification Loop

This is the core loop the MVP must prove works:

```
Human Directive
  "Make a breakout clone"
       │
       ▼
Intent Specification (structured, machine-readable)
  - Ball exists, moves, bounces off walls and paddle
  - Bricks exist, are destroyed on ball collision
  - Score increases when brick destroyed
  - Game ends when all bricks gone
       │
       ▼
Code Generation (AI writes gameplay logic)
  - Entity definitions, behaviors, rules
  - This step is TRIVIAL -- not the bottleneck
       │
       ▼
Headless Simulation (engine runs N ticks)
  - Fixed-timestep, deterministic
  - No GPU needed for verification
       │
       ▼
Manifest (structured semantic output, every tick)
  - What entities exist, their types, states, positions
  - What events occurred (collisions, state changes, spawns, despawns)
  - What caused each change (which system, which command)
       │
       ▼
Verification (AI compares manifest to intent spec)
  - For each intent assertion: did the expected behavior occur?
  - If yes: save as regression test
  - If no: trace causality chain from manifest → diagnose → fix → re-run
       │
       ▼
  ┌─ PASS ──► Regression Test Suite (deterministic replay + manifest assertions)
  │
  └─ FAIL ──► Diagnosis from manifest causality ──► Code fix ──► Re-run ──► (loop)
```

**The critical insight:** Steps 1-3 are solved or trivial. Step 4 is commodity infrastructure. **Steps 5-6 are the entire thesis.** The engine exists to make manifests that enable verification.

---

## 4. Architecture Overview

### Hybrid Native Architecture

```
Layer 3: Python orchestration       (AI agent commands, verification loop, intent specs)
Layer 2: WASM gameplay sandbox      (AI-generated game logic ONLY)
Layer 1: Rust engine core           (ECS, tick loop, physics, rendering, manifest pipeline)
```

All performance-critical and stateful subsystems are native Rust. WASM sandboxes only AI-generated gameplay scripts, which run at low frequency (~10-50 host calls per tick).

### Why Not WASM for Everything

v7 proposed all subsystems (physics, audio, rendering, networking) as WASM plugins. The red-team pass identified three fatal flaws:

| Issue | Detail | Impact |
|-------|--------|--------|
| **Boundary overhead** | Realistic physics needs 2,400-10,000 host calls/tick. v7's spike criterion tested 1,000. At 60Hz, the overhead budget is ~1.6ms -- insufficient for high-frequency subsystems. | Physics becomes the bottleneck, not the game logic. |
| **Stateful plugins** | rapier, kira, and wgpu maintain massive internal state (contact graphs, audio mix state, GPU resources). The "stateless plugin" model requires serializing this across the WASM boundary every tick. | Hot-swap becomes a state migration nightmare, not a clean restart. |
| **Cross-language determinism** | f32/f64 promotion rules differ between Rust native and WASM. HashMap iteration order differs. FMA instruction availability differs. | Deterministic replay breaks silently on edge cases. |

**v8 resolution:** Keep subsystems native. Sandbox only gameplay logic. All three issues disappear:
- Gameplay logic makes ~10-50 host calls/tick (well within budget)
- Gameplay state lives in ECS components (kernel-owned, not plugin-owned)
- Determinism is pure Rust (no cross-language boundary for authoritative state)

### Why Not Bevy

Bevy was considered as a foundation (build Nomai as a Bevy plugin). Rejected because:

- **Bevy is pre-1.0 with frequent breaking changes.** AI agents maintaining code against a moving API target is strictly harder than maintaining code against a stable custom API. Every Bevy release means relearning.
- **Tiered entity identity requires ECS integration.** Bevy's ECS has no concept of semantic vs. pooled vs. ephemeral entities. Grafting this on means constant friction with Bevy's spawn/despawn model.
- **Deterministic replay requires owning the tick loop.** Bevy's schedule system is flexible but not designed for frame-perfect deterministic execution. Fighting it adds complexity instead of removing it.
- **The manifest pipeline needs deep ECS access.** The change journal requires intercepting every component mutation. This is trivial when you own the ECS, non-trivial as a Bevy plugin.

The custom engine is larger upfront investment but smaller total surface area because every system is designed for the manifest.

---

## 5. The Manifest Pipeline

This is the core of the engine. Everything else serves it.

### What the Manifest Must Answer

The manifest must enable an AI agent to answer these question categories about any given tick:

| Category | Example Questions |
|----------|-------------------|
| **Existence** | Does the ball exist? Is it the right type? When was it spawned? |
| **State** | What state is the enemy in (patrolling, chasing, attacking)? When did it transition? |
| **Spatial** | Where is entity X? Is it within the camera frustum? How far from entity Y? |
| **Behavioral** | Did entity X respond to event E? What did it do? Was the response correct? |
| **Causal** | Why did entity X move? Which system issued the command? What input caused it? |
| **Temporal** | When did event E happen? What was the order relative to event F? How many ticks between them? |
| **Aggregate** | How many bricks remain? What's the total score? How many enemies are alive? |
| **Relational** | Is entity X colliding with entity Y? Is X a child of Y? Which entities are in group Z? |
| **Visual** | Was entity X submitted to the render pipeline? Is it within the camera frustum? Is it occluded? |

If the manifest can answer all of these, AI can verify any game behavior without seeing a single pixel.

### Three-Layer Manifest Structure

```
┌─────────────────────────────────────────────────────┐
│  Layer 1: Entity Index (stable, sparse updates)     │
│  - All Semantic entities with full identity          │
│  - Type, role, parent system, requirement ID         │
│  - Updated on spawn/despawn/identity change only     │
├─────────────────────────────────────────────────────┤
│  Layer 2: Change Journal (per-tick, proportional     │
│           to change volume)                          │
│  - Which components changed on which entities        │
│  - Old value → new value                             │
│  - Which system/command caused the change            │
│  - Causality chain back to input or game rule        │
├─────────────────────────────────────────────────────┤
│  Layer 3: Event Log (per-tick, append-only)          │
│  - Collisions, state transitions, spawns, despawns   │
│  - Each event tagged with causing system             │
│  - Cross-referenced with entity IDs                  │
│  - Queryable by type, entity, tick range             │
└─────────────────────────────────────────────────────┘
```

### Manifest Data Model

```rust
/// Emitted every tick. The AI's window into the game world.
struct TickManifest {
    tick: u64,
    sim_time_secs: f64,
    dt: f64,

    // Layer 1: Entity index (full on first tick, deltas thereafter)
    entity_spawns: Vec<EntitySpawn>,
    entity_despawns: Vec<EntityDespawn>,

    // Layer 2: Change journal
    component_changes: Vec<ComponentChange>,

    // Layer 3: Event log
    events: Vec<GameEvent>,

    // Aggregate summaries (computed per-tick, cheap)
    aggregates: HashMap<String, AggregateValue>,

    // Tick-level metadata
    systems_executed: Vec<SystemExecution>,
    commands_applied: u32,
    manifest_generation_us: u32, // self-measurement
}

struct EntitySpawn {
    entity_id: EntityId,
    identity: EntityIdentity,
    tier: IdentityTier, // Semantic | Pooled | Ephemeral
    initial_components: Vec<ComponentSnapshot>,
    spawned_by: SystemId,
    tick: u64,
}

struct ComponentChange {
    entity_id: EntityId,
    component_type: ComponentTypeId,
    component_name: String,
    old_value: Option<Value>, // None if component was just added
    new_value: Option<Value>, // None if component was removed
    changed_by: SystemId,
    command_index: u32, // position in command buffer (deterministic ordering)
}

struct GameEvent {
    event_id: EventId,
    event_type: String,   // "collision", "state_transition", "trigger", etc.
    tick: u64,
    involved_entities: Vec<EntityId>,
    data: HashMap<String, Value>, // event-specific payload
    caused_by: SystemId,
    causal_chain: Vec<CausalLink>, // trace back to root cause
}

struct CausalLink {
    system: SystemId,
    reason: String,    // human/AI-readable: "ball velocity reversed by collision response"
    input_ref: Option<InputRef>, // if caused by player input, which one
}

struct SystemExecution {
    system_id: SystemId,
    system_name: String,
    execution_order: u32,
    duration_us: u32,
    commands_emitted: u32,
    entities_read: u32,
    entities_modified: u32,
}
```

### Manifest Output Formats

| Format | Use Case | Serialization |
|--------|----------|---------------|
| **In-memory** | Real-time verification during simulation | Native Rust structs |
| **JSON** | AI agent queries, debugging, human inspection | serde_json |
| **MessagePack** | On-disk storage, replay archives | rmp-serde |
| **Delta** | Tick-to-tick streaming (only changes) | Custom binary (bincode) |

### Manifest Performance Budget

| Metric | Target | Rationale |
|--------|--------|-----------|
| Generation time | <5% of 16.67ms frame budget | Must not starve simulation |
| Memory per tick | <1MB at 10K Semantic entities | Rolling window, not unbounded |
| Query latency (single entity) | <10us | AI verification is latency-sensitive |
| Query latency (full-tick scan) | <1ms at 10K entities | Batch verification passes |

### Causality Tracking

Every component change and every event includes a **causal chain** -- a trace from the observable change back to its root cause. This is what separates the manifest from a state dump.

Example causal chain for "brick destroyed":

```
Event: brick_7 despawned
  ← caused by: scoring_system (command: despawn brick_7)
    ← caused by: collision_event(ball_1, brick_7) at tick 1234
      ← caused by: physics_system (ball_1 moved into brick_7 AABB)
        ← caused by: ball_1 velocity = [1.0, -2.0] set at tick 1230
          ← caused by: physics_system (bounce off paddle_1 at tick 1230)
            ← caused by: player input (paddle_1 moved to intercept)
```

The AI can read this chain and understand not just *what* happened but *why*, all the way back to player input or game rules.

### Visibility Model

Three levels of visibility information in the manifest:

| Level | What It Tells You | Cost | Default |
|-------|-------------------|------|---------|
| **Render-submitted** | Entity was sent to the GPU pipeline | Free (comes from scene resolution) | Always on |
| **Frustum-visible** | Entity's bounding volume intersects camera frustum | Cheap (CPU AABB test) | Always on for Semantic entities |
| **Pixel-visible** | Entity contributes to final framebuffer pixels | Expensive (GPU readback/occlusion query) | Off (opt-in per entity) |

For verification, levels 1-2 answer 95% of questions. Level 3 is needed only for occlusion-sensitive verification ("can the player actually SEE the health bar?").

### Semantic Art Annotation

The manifest gives AI semantic awareness of game *state* -- positions, velocities, events, causal chains. But games are also visual. When the manifest says "entity `player_1` is at position (200, 300) and is render-submitted," the AI knows *where* the entity is and that it's being drawn. What the AI does NOT know from state alone is: **what does it look like?**

Without semantic art annotation, the AI's understanding has a gap:

```
AI knows:  "player_1 is at (200, 300), state=idle, visible=true"
AI doesn't know: "player_1 is drawn as a 32x48 pixel character sprite
                   facing right, with blue armor, in idle animation frame 3"
```

This gap matters for verification. If the AI changes the player's direction from left to right, the manifest will show the state change -- but is the *correct sprite* being shown? Is the idle animation playing instead of the walk animation? Is the character visually distinguishable from enemies?

**Solution: every art asset carries semantic metadata that the manifest can reference.**

#### Asset Annotation Schema

```rust
/// Every art asset in the engine carries this metadata.
struct AssetAnnotation {
    asset_id: AssetId,
    asset_path: String,              // "sprites/player/idle_right.png"

    // Semantic identity
    semantic_type: String,           // "character_sprite", "tile", "ui_element", "vfx"
    represents: String,              // "player.idle.facing_right"
    tags: Vec<String>,               // ["player", "idle", "right", "armor_blue"]

    // Visual properties (machine-readable, not pixel-dependent)
    dominant_colors: Vec<Color>,     // for distinguishability checks
    dimensions: (u32, u32),          // pixel dimensions
    anchor_point: Vec2,              // where the sprite "is" relative to entity position
    bounding_box: Rect,              // tight AABB for overlap/occlusion reasoning

    // Animation metadata (if part of an animation set)
    animation_set: Option<String>,   // "player_idle"
    frame_index: Option<u32>,        // 3
    total_frames: Option<u32>,       // 8
    frame_duration_ms: Option<u32>,  // 100

    // Relationships
    belongs_to_entity_type: String,  // "character.player"
    visual_group: Option<String>,    // "player_visuals" (group all player-related art)
    contrast_with: Vec<String>,      // ["enemy_sprite"] (must be visually distinguishable from)
}
```

#### How Annotations Feed the Manifest

When the manifest reports an entity's visual state, it includes the semantic annotation of the currently-displayed asset:

```json
{
  "entity": "player_1",
  "position": [200, 300],
  "state": "idle",
  "visible": true,
  "visual": {
    "current_asset": "sprites/player/idle_right.png",
    "represents": "player.idle.facing_right",
    "animation": "player_idle",
    "frame": 3,
    "dimensions": [32, 48],
    "tags": ["player", "idle", "right", "armor_blue"],
    "facing": "right"
  }
}
```

Now the AI can verify visual correctness through the manifest:

```python
# Verify the player sprite matches the game state
player = manifest.entity("player_1")
assert player.visual.represents == "player.idle.facing_right", \
    f"Player is in state 'idle' facing right but showing '{player.visual.represents}'"

# Verify animation is playing
assert player.visual.animation == "player_idle", \
    f"Expected idle animation but got '{player.visual.animation}'"

# Verify player is visually distinguishable from enemies
for enemy in manifest.entities(type="enemy"):
    assert player.visual.dominant_colors != enemy.visual.dominant_colors, \
        f"Player and {enemy.identity.role} have identical colors -- indistinguishable"
```

#### Annotation Sources

Art assets can be annotated at three levels:

| Source | When | Example |
|--------|------|---------|
| **Manual (artist-provided)** | Asset import time | Artist tags sprite as "player.idle.facing_right" in asset metadata |
| **Convention-based (path parsing)** | Build time | `sprites/player/idle_right.png` → auto-derive semantic_type, represents, tags from directory structure and filename |
| **AI-generated** | Runtime or build time | AI annotates an unlabeled asset by analyzing filename, context of use, and (optionally) visual content via vision model |

For MVP, convention-based annotation is sufficient. The directory structure IS the semantic annotation:

```
assets/
  sprites/
    player/
      idle_right.png    → represents: "player.idle.facing_right"
      idle_left.png     → represents: "player.idle.facing_left"
      walk_right_01.png → represents: "player.walk.facing_right", frame: 1
    enemy/
      melee/
        idle.png        → represents: "enemy.melee.idle"
    ui/
      health_bar.png    → represents: "ui.health_bar"
  tiles/
    brick.png           → represents: "tile.brick"
    wall.png            → represents: "tile.wall"
```

A simple build-time tool walks the asset directory, parses paths against a naming convention, and generates `AssetAnnotation` structs. This is ~200 lines of code and gives the manifest full visual awareness.

#### Why This Matters for the Thesis

Without semantic art annotation, the manifest is a state model that happens to drive rendering. With it, the manifest is a **complete semantic description of the player's visual experience**. The AI can reason about:

- **Correctness:** "Is the right sprite showing for this state?"
- **Consistency:** "Do all enemies of the same type look similar?"
- **Distinguishability:** "Can the player tell enemies apart from friendly NPCs?"
- **Animation:** "Is the walk animation playing when the character moves?"
- **Visual feedback:** "When the player takes damage, does the sprite change?"

This bridges the last gap between "what the engine knows" and "what the player sees."

---

## 6. Intent Specifications

Intent specs are how the AI expresses *what should be true* about the game. They are structured, machine-readable assertions that map directly to manifest queries.

### Intent Spec Structure

```python
class IntentSpec:
    """Machine-readable description of what a game should do."""

    name: str                          # "breakout"
    description: str                   # Human-readable summary
    entity_intents: list[EntityIntent]
    behavior_intents: list[BehaviorIntent]
    metric_intents: list[MetricIntent]
    invariant_intents: list[InvariantIntent]

class EntityIntent:
    """An entity that must exist with certain properties."""
    name: str                      # "paddle"
    entity_type: str               # "controller"
    must_exist: bool = True
    must_be_visible: bool = True   # render-submitted + frustum-visible
    required_components: list[str] = []

class BehaviorIntent:
    """A behavior that must be observable through the manifest."""
    name: str                      # "ball_bounces_off_paddle"
    description: str               # Human-readable
    trigger: TriggerExpr           # When to check (e.g., collision between ball and paddle)
    expected: ExpectedOutcome      # What should happen (e.g., ball velocity.y sign flips)
    timeout_ticks: int             # How long to wait for trigger before failing

class MetricIntent:
    """A numeric property that must stay within bounds."""
    name: str                      # "ball_speed"
    entity: str                    # "ball"
    component: str                 # "velocity"
    measurement: str               # "magnitude"
    range: tuple[float, float]     # (1.0, 10.0)

class InvariantIntent:
    """A property that must ALWAYS hold, every tick."""
    name: str                      # "ball_stays_in_bounds"
    description: str
    condition: str                 # Manifest query expression
    # e.g., "entity('ball').position.x >= 0 AND entity('ball').position.x <= SCREEN_WIDTH"
```

### Trigger Expressions

Triggers are conditions evaluated against the manifest each tick:

```python
# Collision between two entities
trigger = Collision("ball", "paddle")

# Entity state transition
trigger = StateTransition("enemy_1", from_state="patrolling", to_state="chasing")

# Aggregate condition
trigger = AggregateCondition("brick_count", "==", 0)

# Component value condition
trigger = ComponentCondition("ball", "position.y", "<", 0)

# Event occurrence
trigger = EventOccurred("game_over")

# Compound
trigger = And(
    Collision("ball", "brick_*"),       # ball hits any brick
    After(ticks=1),                     # one tick later...
    AggregateChange("score", ">", 0),   # score increased
)
```

### Expected Outcomes

What should be true after a trigger fires:

```python
# Entity property changed
expected = ComponentChanged("ball", "velocity.y", sign_flipped=True)

# Entity despawned
expected = EntityDespawned("brick_7")

# Aggregate changed
expected = AggregateChanged("score", increased_by=10)

# State transition occurred
expected = InState("game", "won")

# Compound
expected = All(
    EntityDespawned(matching="brick_*"),   # the hit brick is gone
    AggregateChanged("score", increased_by=10),  # score went up
    EventEmitted("brick_destroyed"),       # event was fired
)
```

### Intent Spec as Regression Test

Once an intent spec passes verification, it becomes a **deterministic regression test**:

```python
regression_test = RegressionTest(
    intent=intent_spec,
    initial_snapshot=snapshot_id,       # deterministic starting state
    input_recording=input_log_id,       # recorded inputs that trigger the behavior
    expected_manifest_at_tick={
        1234: ManifestAssertions([...]),  # specific tick-level assertions
    },
    passing_since_tick=current_tick,
    created_by="ai_agent",
)
```

Future changes that break this test produce a **structured failure report** with:
- Which assertion failed
- The manifest state at the failing tick
- The causal chain explaining why the state differs from expectation
- The git diff that introduced the regression

---

## 7. Verification Engine

The verification engine is the system that closes the loop. It runs intent specs against manifest output and produces structured pass/fail reports.

### Verification Execution

```python
class VerificationEngine:
    def verify(self, engine: NomaiEngine, intent: IntentSpec,
               max_ticks: int = 6000) -> VerificationReport:
        """Run all intent assertions against a headless simulation."""

        results = []

        # Check entity existence and properties
        for entity_intent in intent.entity_intents:
            results.append(self._verify_entity(engine, entity_intent, max_ticks))

        # Check behaviors (trigger → expected outcome)
        for behavior_intent in intent.behavior_intents:
            results.append(self._verify_behavior(engine, behavior_intent, max_ticks))

        # Check metrics (must be within range at all times)
        for metric_intent in intent.metric_intents:
            results.append(self._verify_metric(engine, metric_intent, max_ticks))

        # Check invariants (must hold every tick)
        for invariant_intent in intent.invariant_intents:
            results.append(self._verify_invariant(engine, invariant_intent, max_ticks))

        return VerificationReport(
            intent=intent,
            results=results,
            passed=all(r.passed for r in results),
            ticks_simulated=max_ticks,
        )

    def _verify_behavior(self, engine, intent, max_ticks):
        """Run simulation looking for trigger, then check expected outcome."""
        engine.restore_snapshot("initial")

        for tick in range(max_ticks):
            engine.tick()
            manifest = engine.get_tick_manifest()

            if intent.trigger.matches(manifest):
                # Trigger fired -- check expected outcome
                outcome = intent.expected.check(manifest)
                return BehaviorResult(
                    intent=intent,
                    passed=outcome.passed,
                    trigger_tick=tick,
                    manifest_evidence=manifest,
                    failure_reason=outcome.reason if not outcome.passed else None,
                    causal_chain=manifest.trace_causality(intent.involved_entities()),
                )

        # Trigger never fired
        return BehaviorResult(
            intent=intent,
            passed=False,
            failure_reason=f"Trigger '{intent.trigger}' never fired in {max_ticks} ticks",
            suggestion="Check that the entities involved exist and interact correctly",
        )
```

### Verification Report

```python
class VerificationReport:
    intent: IntentSpec
    results: list[VerificationResult]
    passed: bool
    ticks_simulated: int
    wall_time_ms: float

    def failures(self) -> list[VerificationResult]:
        return [r for r in self.results if not r.passed]

    def diagnosis(self) -> str:
        """AI-readable summary of what failed and why."""
        # Structured text with causal chains for each failure
        ...

    def suggested_fixes(self) -> list[SuggestedFix]:
        """Heuristic fix suggestions based on failure patterns."""
        # e.g., "entity not found" → "add entity spawn"
        # e.g., "trigger never fired" → "check collision layers"
        # e.g., "wrong value" → "check the system that modifies this component"
        ...
```

### The Fix Loop

When verification fails, the AI uses the structured report to fix the issue:

```python
def develop_game(engine, directive: str, model: LLM):
    # Step 1: Decompose directive into intent spec
    intent = model.generate_intent_spec(directive)

    # Step 2: Generate initial game logic
    code = model.generate_game_code(intent)
    engine.load_gameplay_wasm(code.compile())

    # Step 3: Verify
    max_fix_attempts = 5
    for attempt in range(max_fix_attempts):
        report = VerificationEngine().verify(engine, intent)

        if report.passed:
            # Save all passing behaviors as regression tests
            save_regression_tests(engine, intent, report)
            return Success(report)

        # Step 4: Fix from structured diagnosis
        diagnosis = report.diagnosis()
        causal_chains = [f.causal_chain for f in report.failures()]

        fix = model.generate_fix(
            current_code=code,
            diagnosis=diagnosis,
            causal_chains=causal_chains,
            attempt=attempt,
        )

        code = fix.apply(code)
        engine.hot_swap_gameplay_wasm(code.compile())

    return Failure(report, attempts=max_fix_attempts)
```

**Key property:** At no point does the AI need to see pixels, ask a human, or guess. The manifest provides complete semantic awareness. The causal chains tell it *why* something went wrong. The fix loop is fully autonomous.

---

## 8. Rust Engine Core

### Design Principle

The engine core is native Rust. It is not modular or plugin-based -- it is a single binary that contains ECS, physics, rendering, and the manifest pipeline. This is deliberately less flexible than v7's microkernel design and deliberately more practical for an MVP.

The core must be small enough that AI can maintain it. Target: **~8,000 lines of Rust** for MVP.

### Components

| Component | Responsibility | Approximate LOC |
|-----------|---------------|-----------------|
| **ECS** | Archetype storage, entity allocation, tiered identity, component registry | ~2,000 |
| **Tick Loop** | Fixed-timestep execution, deterministic system ordering, command buffer | ~500 |
| **Command Buffer** | Deferred mutations, deterministic application order, causality tagging | ~500 |
| **Manifest Pipeline** | Change journal, event log, entity index, manifest generation, query API | ~2,000 |
| **Snapshot/Restore** | Full world serialization, deterministic replay entry point | ~600 |
| **Physics Integration** | rapier2d wrapper, collision events, spatial queries | ~500 |
| **Render Integration** | wgpu debug renderer (2D wireframe/sprites for MVP) | ~800 |
| **WASM Host** | Wasmtime integration, host function dispatch, fuel metering | ~1,000 |
| **Glue** | Startup, config, CLI, error handling | ~500 |

**Total: ~8,400 LOC**

### Tiered Entity Identity

Every entity declares an identity tier at spawn. This is the foundation for manifest granularity:

```rust
enum IdentityTier {
    /// Full identity, full manifest presence, full causality tracking.
    /// Used for game-meaningful entities: player, enemies, items, UI elements.
    Semantic,

    /// Type-level identity, instance aggregated in manifest.
    /// Used for repeated entities: bullets, coins, tiles.
    Pooled,

    /// Count only in manifest. No individual tracking.
    /// Used for transient effects: particles, debris, trails.
    Ephemeral,
}

struct EntityIdentity {
    entity_type: String,       // "character", "projectile", "ui"
    role: String,              // "player", "enemy.melee", "health_bar"
    spawned_by: SystemId,      // which system created this entity
    requirement_id: Option<String>, // traces back to game design intent
}

// Spawn API makes tier explicit -- impossible to forget
let player = world.spawn_semantic(
    EntityIdentity {
        entity_type: "character",
        role: "player",
        spawned_by: SystemId::PLAYER_SPAWNER,
        requirement_id: Some("PLAYER-01".into()),
    },
    (Position::new(400.0, 550.0), Velocity::ZERO, Paddle { width: 80.0 }),
);

let ball = world.spawn_semantic(
    EntityIdentity {
        entity_type: "projectile",
        role: "ball",
        spawned_by: SystemId::BALL_SPAWNER,
        requirement_id: Some("BALL-01".into()),
    },
    (Position::new(400.0, 300.0), Velocity::new(2.0, -3.0), Ball { radius: 5.0 }),
);

let brick = world.spawn_pooled(
    PoolIdentity { pool_type: "destructible", variant: "brick" },
    (Position::new(x, y), Brick { health: 1, points: 10 }),
);

let particle = world.spawn_ephemeral(
    EphemeralTag::VFX,
    (Position::new(x, y), Lifetime(0.3)),
);
```

**Manifest impact by tier:**

| Tier | Entity Index | Change Journal | Event Log | Causality |
|------|-------------|----------------|-----------|-----------|
| Semantic | Full entry | Every field change | All events | Full chain |
| Pooled | Type-level summary | Aggregate stats | Aggregate events | Type-level |
| Ephemeral | Count only | None | None | None |

### Command Buffer with Causality

All state mutations flow through the command buffer. Every command is tagged with its origin:

```rust
struct Command {
    kind: CommandKind,         // Spawn, Despawn, SetComponent, InsertComponent, RemoveComponent
    target: EntityId,
    data: CommandData,

    // Causality metadata -- this is what makes the manifest useful
    issued_by: SystemId,       // which system emitted this command
    reason: CausalReason,      // why (collision response, game rule, player input, etc.)
    source_event: Option<EventId>, // which event triggered this command, if any
}

enum CausalReason {
    PlayerInput(InputRef),
    CollisionResponse(EntityId, EntityId),
    GameRule(String),          // "brick_destroyed_on_hit"
    StateTransition(String, String), // from_state, to_state
    Timer(String),
    SystemInternal(String),
}
```

When the command buffer is applied, the change journal records both the state change and the causality. This is how the manifest gets its causal chains.

### Tick Execution Model

```
1. Input Capture
   - Record raw inputs into frame input log (for replay)
   - Tag with tick number

2. Gameplay Logic Execution (WASM sandbox)
   - WASM gameplay module reads entity state via host functions
   - Emits commands to command buffer
   - Each command tagged with system ID + causal reason

3. Physics Step (native rapier2d)
   - Step rapier with fixed dt
   - Collision events → game events with entity pairs
   - Position/velocity updates → commands with CausalReason::CollisionResponse

4. Command Buffer Application
   - All commands applied in deterministic order
   - Change journal updated with old/new values + causality
   - Entity index updated for spawns/despawns

5. Manifest Generation
   - Change journal → manifest Layer 2
   - Events → manifest Layer 3
   - Entity spawns/despawns → manifest Layer 1 delta
   - Aggregates computed
   - Causal chains assembled

6. Render (optional, skipped in headless)
   - Scene resolution from ECS state
   - Frustum visibility computed, written to manifest
   - wgpu draw calls emitted
```

---

## 9. WASM Gameplay Sandbox

### What Goes in WASM

Only AI-generated gameplay logic. Specifically:

- Entity behavior scripts (state machines, AI, game rules)
- Scoring and progression logic
- Entity spawn/despawn rules
- Game state management (menu, playing, paused, game over)

### What Stays Native

Everything with high-frequency state or host calls:

- Physics (rapier2d -- thousands of internal operations per step)
- Rendering (wgpu -- GPU state management)
- Audio (kira -- audio mix state, deferred to post-MVP)
- ECS core (archetype storage, entity allocation)
- Manifest pipeline (needs deep ECS access)
- Snapshot/restore (needs full world access)

### Host API

```rust
/// Functions the WASM gameplay module can call
trait GameplayHost {
    // Read state
    fn get_position(&self, entity: EntityId) -> Option<Vec2>;
    fn get_component(&self, entity: EntityId, component: &str) -> Option<Value>;
    fn query_entities(&self, filter: &EntityFilter) -> Vec<EntityId>;
    fn get_aggregate(&self, name: &str) -> Option<f64>;

    // Emit commands (applied at end of phase, not immediately)
    fn spawn(&mut self, tier: IdentityTier, identity: &str, components: &[ComponentData]) -> EntityId;
    fn despawn(&mut self, entity: EntityId, reason: &str);
    fn set_component(&mut self, entity: EntityId, component: &str, value: Value, reason: &str);
    fn emit_event(&mut self, event_type: &str, data: &HashMap<String, Value>, reason: &str);

    // Utilities
    fn random_f32(&self) -> f32;  // deterministic, seeded
    fn sim_time(&self) -> f64;
    fn tick_number(&self) -> u64;
    fn log(&self, level: LogLevel, message: &str);
}
```

**Key design choices:**

1. **Every mutation takes a `reason` parameter.** This feeds the manifest's causality tracking. The AI *must* explain why it's making each change. This is not a burden -- it's the information that makes verification possible.

2. **Read operations are immediate, writes are deferred.** Gameplay logic sees a consistent snapshot of the world. All commands are applied after all gameplay scripts finish, in deterministic order.

3. **No direct physics access.** Gameplay logic reads position/velocity from components. Physics integration is handled by the native engine. This keeps the WASM boundary clean.

### WASM Runtime: Wasmtime

| Property | Value |
|----------|-------|
| Runtime | Wasmtime (Bytecode Alliance) |
| Fuel metering | Enabled (deterministic execution budgets) |
| Memory limit | 16MB per module (configurable) |
| Compilation | Cranelift (AOT, ~10-50ms compile time) |
| Sandbox | No filesystem, no network, no threads, no wall-clock time |

### AI Code Generation Target

**Primary: AssemblyScript (TypeScript syntax → WASM)**

- Highest LLM accuracy for WASM target (TypeScript-family syntax)
- Fast compilation (~100-500ms)
- Clear error messages
- The AI development loop: write AS → compile to WASM (500ms) → hot-swap (50ms) → verify → iterate

**Secondary: Rust (for performance-critical logic)**

- ~2x runtime performance over AssemblyScript
- Slower compilation (2-15s)
- Used when a gameplay script is stabilized and performance matters

### Hot-Swap

Gameplay WASM modules can be swapped at tick boundaries:

```
1. AI compiles new gameplay module
2. Engine validates WASM binary (structure, imports, exports)
3. At next tick boundary: unload old module, load new module
4. New module executes starting next tick
5. No state migration needed -- all state is in ECS components (kernel-owned)
```

Because gameplay logic is stateless (all state lives in ECS), hot-swap is trivial. No state migration, no version compatibility, no serialization. This is the payoff of keeping authoritative state in the kernel.

---

## 10. Deterministic Replay

Deterministic replay is the foundation of both verification and regression testing. If you can replay, you can verify. If you can verify deterministically, you can regression-test.

### Simulation Contract

The engine guarantees: given the same initial snapshot, the same input sequence, and the same WASM gameplay module, the simulation produces identical world state at every tick.

**What is deterministic:**
- ECS state (all component values on all entities)
- Command buffer application order
- System execution order
- Entity ID allocation
- RNG output (seeded, per-system sub-streams)
- Physics (rapier2d `enhanced-determinism` feature)

**What is NOT deterministic (and doesn't need to be):**
- Rendering output (GPU-dependent, not part of simulation)
- Wall-clock timing
- Manifest generation performance (content is deterministic, timing is not)

### Replay Log Format

```rust
struct ReplayLog {
    initial_snapshot: WorldSnapshot,
    gameplay_module_hash: [u8; 32],  // BLAKE3 hash of WASM binary
    seed: u64,
    entries: Vec<ReplayEntry>,
}

enum ReplayEntry {
    Input { tick: u64, input: InputFrame },
    Checkpoint { tick: u64, state_hash: [u8; 32] },
}
```

### Verification via Replay

```python
# Record a passing verification run
recording = engine.record_replay(intent_spec, max_ticks=6000)

# Later, after code changes, replay and verify
replay_result = engine.replay(recording)
for checkpoint in replay_result.checkpoints:
    assert checkpoint.hash == recording.expected_hash_at(checkpoint.tick), \
        f"Determinism broken at tick {checkpoint.tick}"

# Also re-run verification against manifest from replay
report = VerificationEngine().verify_replay(engine, intent_spec, replay_result)
assert report.passed, f"Regression: {report.diagnosis()}"
```

### Snapshot Branching

Fork a simulation at any tick, try two approaches, compare manifests:

```python
snapshot = engine.capture_snapshot()

# Branch A: ball speed = 3.0
engine.set_component("ball", "velocity", Vec2(3.0, -3.0))
engine.run_ticks(300)
manifest_a = engine.get_manifest_range(0, 300)

# Branch B: ball speed = 5.0
engine.restore_snapshot(snapshot)
engine.set_component("ball", "velocity", Vec2(5.0, -5.0))
engine.run_ticks(300)
manifest_b = engine.get_manifest_range(0, 300)

# Compare: which branch produces better gameplay?
diff = manifest_diff(manifest_a, manifest_b)
```

This enables AI to experiment with game design hypotheses. Instead of guessing, it runs controlled experiments.

---

## 11. Python AI Interface

Python is the AI agent's interface to the engine. It runs out-of-band (not inside the simulation tick) and communicates through a structured API.

### Engine Control API

```python
class NomaiEngine:
    # Lifecycle
    def start(self, config: EngineConfig) -> None: ...
    def shutdown(self) -> None: ...

    # Simulation control
    def tick(self) -> TickManifest: ...
    def run_ticks(self, n: int) -> list[TickManifest]: ...
    def run_until(self, condition: Callable[[TickManifest], bool],
                  max_ticks: int = 10000) -> list[TickManifest]: ...

    # Manifest queries
    def get_tick_manifest(self, tick: int = -1) -> TickManifest: ...
    def get_manifest_range(self, start: int, end: int) -> ManifestRange: ...
    def query_entities(self, filter: EntityFilter) -> list[EntityView]: ...
    def get_aggregate(self, name: str) -> AggregateValue: ...
    def trace_causality(self, entity: str, tick: int) -> CausalChain: ...

    # Snapshot / replay
    def capture_snapshot(self) -> SnapshotId: ...
    def restore_snapshot(self, snapshot: SnapshotId) -> None: ...
    def record_replay(self) -> ReplayRecording: ...
    def replay(self, recording: ReplayRecording) -> ReplayResult: ...

    # Gameplay module management
    def load_gameplay_wasm(self, wasm_bytes: bytes) -> None: ...
    def hot_swap_gameplay_wasm(self, wasm_bytes: bytes) -> None: ...

    # Component manipulation (for testing / setup)
    def set_component(self, entity: str, component: str, value: Any) -> None: ...
    def spawn_entity(self, tier: str, identity: dict, components: dict) -> EntityId: ...
    def despawn_entity(self, entity: EntityId) -> None: ...
```

### Manifest Query API

```python
class TickManifest:
    tick: int
    sim_time: float

    def entity(self, name_or_id: str) -> EntityView: ...
    def entities(self, type: str = None, role: str = None) -> list[EntityView]: ...
    def entity_exists(self, name_or_id: str) -> bool: ...

    def events(self, type: str = None, involving: str = None) -> list[GameEvent]: ...
    def changes(self, entity: str = None, component: str = None) -> list[ComponentChange]: ...

    def aggregate(self, name: str) -> float: ...
    def aggregates(self) -> dict[str, float]: ...

class EntityView:
    entity_id: EntityId
    identity: EntityIdentity
    tier: IdentityTier

    position: Vec2  # shorthand for get_component("position")

    def get_component(self, name: str) -> Any: ...
    def has_component(self, name: str) -> bool: ...
    def changed_this_tick(self, component: str = None) -> bool: ...
    def changed_by(self) -> list[SystemId]: ...

    @property
    def visible(self) -> bool: ...      # render-submitted AND frustum-visible
    @property
    def render_submitted(self) -> bool: ...
    @property
    def frustum_visible(self) -> bool: ...

class ManifestRange:
    """Query across multiple ticks."""
    def component_over_time(self, entity: str, component: str) -> list[tuple[int, Any]]: ...
    def events_in_range(self, type: str = None) -> list[GameEvent]: ...
    def first_tick_where(self, condition: Callable[[TickManifest], bool]) -> Optional[int]: ...
```

### AI Development Session (End-to-End Example)

```python
from nomai import NomaiEngine, IntentSpec, VerificationEngine
from nomai.intents import *

# Human directive: "Make breakout"
engine = NomaiEngine.start(headless=True)

# Step 1: AI generates intent spec from directive
intent = IntentSpec("breakout",
    entity_intents=[
        EntityIntent("paddle", entity_type="controller", must_be_visible=True,
                     required_components=["position", "paddle"]),
        EntityIntent("ball", entity_type="projectile", must_be_visible=True,
                     required_components=["position", "velocity", "ball"]),
        EntityIntent("brick", entity_type="destructible", must_be_visible=True,
                     min_count=20),
    ],
    behavior_intents=[
        BehaviorIntent(
            name="ball_bounces_off_walls",
            trigger=ComponentCondition("ball", "position.x", "<=", 0),
            expected=ComponentChanged("ball", "velocity.x", sign_flipped=True),
            timeout_ticks=600,
        ),
        BehaviorIntent(
            name="ball_bounces_off_paddle",
            trigger=Collision("ball", "paddle"),
            expected=ComponentChanged("ball", "velocity.y", sign_flipped=True),
            timeout_ticks=600,
        ),
        BehaviorIntent(
            name="brick_destroyed_on_hit",
            trigger=Collision("ball", "brick_*"),
            expected=All(
                EntityDespawned(matching="brick_*"),
                AggregateChanged("score", increased=True),
            ),
            timeout_ticks=600,
        ),
        BehaviorIntent(
            name="game_won_when_no_bricks",
            trigger=AggregateCondition("brick_count", "==", 0),
            expected=InState("game", "won"),
            timeout_ticks=10000,
        ),
    ],
    metric_intents=[
        MetricIntent("ball_speed", entity="ball", component="velocity",
                     measurement="magnitude", range=(1.0, 10.0)),
    ],
    invariant_intents=[
        InvariantIntent(
            name="ball_in_bounds",
            condition="entity('ball').position.x >= 0 AND entity('ball').position.x <= 800"
                      " AND entity('ball').position.y >= 0 AND entity('ball').position.y <= 600",
        ),
        InvariantIntent(
            name="paddle_in_bounds",
            condition="entity('paddle').position.x >= 0 AND entity('paddle').position.x <= 800",
        ),
    ],
)

# Step 2: AI generates gameplay code (trivial)
gameplay_code = ai_model.generate_gameplay(intent)
wasm_binary = compile_assemblyscript(gameplay_code)
engine.load_gameplay_wasm(wasm_binary)

# Step 3: Verify (this is the hard part -- and the manifest makes it work)
verifier = VerificationEngine()
report = verifier.verify(engine, intent, max_ticks=6000)

if report.passed:
    print("All behaviors verified!")
    # Save regression tests
    for result in report.results:
        save_regression_test(engine, result)
else:
    print(f"Failures: {len(report.failures())}")
    for failure in report.failures():
        print(f"  {failure.intent.name}: {failure.failure_reason}")
        print(f"  Causal chain: {failure.causal_chain}")

    # AI reads the structured diagnosis and fixes
    # (see Section 7: The Fix Loop)
```

---

## 12. Tech Stack

Pinned versions. No "latest".

### Core

| Component | Choice | Version | Rationale |
|-----------|--------|---------|-----------|
| Language | Rust | 1.83.0 stable | Safety, performance, determinism |
| Build | cargo | Bundled with rustc | Standard |
| Task runner | just | 1.38.0 | Command runner, CI orchestration |
| ECS | Custom | N/A | Tiered identity + manifest integration requires custom |
| Physics | rapier2d | 0.22.0 (`enhanced-determinism`) | Cross-platform determinism, Rust-native |
| Rendering | wgpu | 23.0.0 | Cross-platform GPU abstraction |
| Windowing | winit | 0.30.8 | Cross-platform windowing |
| Serialization (binary) | bincode | 2.0.0-rc.3 | Fast, compact, serde-compatible |
| Serialization (manifest) | serde_json | 1.0.134 | AI-readable manifest output |
| Serialization (storage) | rmp-serde | 1.3.0 | Compact on-disk manifest storage |
| Hashing | blake3 | 1.5.5 | Content addressing, replay checksums |
| RNG | rand + rand_pcg | rand 0.8.5, rand_pcg 0.3.1 | Deterministic, per-system seeding |
| Logging | tracing | 0.1.41 | Structured, async-aware |

### WASM

| Component | Choice | Version | Rationale |
|-----------|--------|---------|-----------|
| Runtime | wasmtime | 27.0.0 | Fuel-based determinism, Cranelift, Component Model |
| AI code target | AssemblyScript | 0.28.2 | Highest LLM accuracy for WASM |
| WASM tools | wasm-tools | 1.222.0 | Validate, inspect WASM binaries |

### Python

| Component | Choice | Version | Rationale |
|-----------|--------|---------|-----------|
| Python | CPython | 3.12.8 | AI interface, verification engine |
| Type checker | pyright | 1.1.391 | Strict mode |
| Test runner | pytest | 8.3.4 | Standard |
| Engine binding | PyO3 | 0.23.3 | Rust ↔ Python FFI |

### Testing

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Rust tests | cargo-nextest | Parallel execution, better output |
| Rust property tests | proptest | Snapshot round-trip, command sequences |
| Rust benchmarks | criterion | Statistical benchmarking |
| Python tests | pytest | Verification engine tests |

### AI Integration

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Primary LLM | Model-agnostic (Claude Opus 4.6 default) | Highest reasoning for code gen + verification |
| Fast LLM | Model-agnostic (Claude Sonnet 4.5 default) | Lower cost for simple tasks |
| API access | Direct HTTP | No framework abstraction |
| Prompt management | Version-controlled .prompt files | Prompts are code |

### Infrastructure

| Component | Choice | Rationale |
|-----------|--------|-----------|
| CI | GitHub Actions | Free tier sufficient for MVP |
| Source control | Git + GitHub | Standard |
| Dev environment | Nix flake | Reproducible toolchain |

---

## 13. MVP Scope and Deliverable

### The Experiment

**Hypothesis:** Structured semantic access to rendered game state (via manifest) enables AI to verify its own game development work without human engineering intervention.

**Experiment:** An AI agent, given the text directive "make a breakout clone", must:

1. Generate an intent specification from the directive
2. Generate gameplay code (entity definitions, behaviors, game rules)
3. Load the code into the engine
4. Run headless simulation
5. Read the manifest and verify each intent assertion
6. When verification fails: read the causal chain, diagnose the issue, generate a fix, re-run
7. Iterate until all assertions pass (max 5 attempts per assertion)
8. Save passing assertions as deterministic regression tests
9. A human can then play the result and confirm it is, in fact, breakout

All of steps 1-8 must happen without human engineering intervention. Step 9 is the subjective validation.

### What's IN the MVP

| Component | What It Includes | Why It's Needed |
|-----------|-----------------|-----------------|
| **Rust ECS** | Archetype storage, tiered identity (Semantic + Pooled), entity allocation | Foundation for manifest |
| **Tick loop** | Fixed-timestep, deterministic system ordering, single-threaded | Determinism guarantee |
| **Command buffer** | Deferred mutations with causality tagging | Feeds manifest causal chains |
| **Manifest pipeline** | Entity index, change journal, event log, aggregate computation, causality assembly | THE core deliverable |
| **Manifest query API** | Python API for entity queries, event queries, aggregate queries, causality tracing | How AI reads the manifest |
| **Physics** | rapier2d native integration, collision events with entity pairs | Breakout needs ball-paddle-brick collisions |
| **Debug renderer** | wgpu 2D renderer: colored rectangles, basic text, camera | Human validation of the result |
| **WASM sandbox** | Wasmtime host, gameplay module loading, hot-swap at tick boundary | AI-generated code runs here |
| **Snapshot/restore** | Full world serialization, deterministic replay | Regression testing foundation |
| **Intent specification** | Python DSL for entity/behavior/metric/invariant assertions | How AI expresses expectations |
| **Verification engine** | Runs intents against manifest, produces structured reports | Closes the verification loop |
| **Python engine bindings** | PyO3-based API for engine control, manifest queries | AI agent interface |

### What's NOT in the MVP

| Component | When | Why Not Now |
|-----------|------|-------------|
| Audio | Post-MVP | Not needed to prove verification thesis |
| Networking | Post-MVP | Not needed for single-player breakout |
| Full Python DSL (Feel/Structure/Behavior blocks) | Post-MVP | MVP uses direct code gen, not DSL pipeline |
| DSL compiler pipeline | Post-MVP | Adds complexity without proving thesis |
| Graduated autonomy model | Post-MVP | MVP has a single developer (the AI), no merge pipeline |
| Game Autonomy Plane | Post-MVP | Playtesting agents, self-healing, live evolution come after core is proven |
| CI/CD pipeline | Post-MVP | Manual builds are fine for proving the thesis |
| Cross-platform determinism | Post-MVP | Same-platform determinism is sufficient for MVP |
| Luau scripting | Post-MVP | WASM gameplay sandbox is the MVP scripting layer |
| Content-addressed artifact store | Post-MVP | Not needed until there are multiple artifacts |
| Ring-based deployment | Post-MVP | No deployment in MVP |
| Ephemeral entity tier | Post-MVP | Not needed for breakout (no particle effects) |
| Pixel-visible occlusion queries | Post-MVP | Frustum visibility is sufficient |

### MVP Deliverable Artifact

A single Rust binary (`nomai-engine`) + Python package (`nomai-sdk`) that:

1. Launches a headless simulation
2. Accepts WASM gameplay modules
3. Produces tick manifests with full causality
4. Exposes a Python API for manifest queries
5. Supports snapshot/restore and deterministic replay
6. Includes a debug 2D renderer for human validation
7. Ships with the verification engine and intent spec DSL

Plus a demo script (`demo_breakout.py`) that runs the full verification loop end-to-end.

---

## 14. Development Roadmap

### Phase 0: Feasibility Spikes (4-5 weeks)

Two spikes, not five. We cut the v7 spikes that are no longer relevant to the hybrid architecture.

| Spike | Duration | Focus | Pass Criteria | Kill Criteria |
|-------|----------|-------|---------------|---------------|
| **A: ECS + Manifest** | 2-3 weeks | Custom ECS with tiered identity, command buffer with causality tagging, manifest generation from change journal | Manifest correctly reflects all component changes with causal chains for 1K entities at 60Hz. Manifest generation <5% of frame budget. | Manifest generation >10% of frame budget, or causality tracking adds >30% command overhead. |
| **B: WASM Gameplay + Verification** | 2-3 weeks | Wasmtime integration, gameplay host API, intent spec evaluation against manifest output | WASM gameplay module reads entities, emits commands with reasons, manifest captures causality, verification engine correctly detects intentional failures. | WASM host call overhead >1ms for 50 calls/tick, or causality chain breaks across WASM boundary. |

**Phase 0 exit gate:** Both spikes pass. Manifest provides correct, causal, queryable representation of game state. Verification engine can distinguish correct from incorrect behavior from manifest alone.

### Phase 1: MVP Build (6-8 weeks)

| Week | Focus | Deliverable |
|------|-------|-------------|
| 1-2 | ECS core + tick loop + command buffer | Entity spawning with tiered identity, fixed-step tick, deterministic command application with causality tags |
| 3-4 | Manifest pipeline + query API | Change journal, event log, entity index, Python query API via PyO3 |
| 4-5 | Physics + WASM sandbox | rapier2d integration with collision events, wasmtime gameplay host, hot-swap |
| 5-6 | Verification engine + intent specs | Intent spec DSL, trigger/expected evaluation, structured reports with causal chains |
| 6-7 | Snapshot/restore + debug renderer | Deterministic replay, 2D wireframe renderer, demo breakout game |
| 7-8 | End-to-end integration + demo | `demo_breakout.py` runs full verification loop, regression test saving, polish |

**Phase 1 exit gate:** `demo_breakout.py` completes without human intervention. AI generates breakout, verifies it, fixes failures, and saves regression tests. A human plays the result and confirms it's breakout.

### Total Timeline: 10-13 weeks

Phase 0 (4-5 weeks) + Phase 1 (6-8 weeks) = **10-13 weeks to thesis validation**.

This is 2-8 weeks shorter than v7's M0 (14-21 weeks) because:
- No WASM plugin system for physics/audio/rendering (native instead)
- No Luau integration (WASM only for MVP)
- No DSL compiler pipeline (direct code gen)
- No graduated autonomy model (single developer)
- No CI/CD pipeline (manual builds)
- Focused on one thing: proving the verification loop works

---

## 15. Success Criteria

### Primary: The Verification Thesis

**The experiment succeeds if:**

1. AI generates a working breakout clone from a text directive, verifying its own work through manifest queries, without human engineering intervention.

2. When the AI introduces a deliberate bug (e.g., ball doesn't bounce off paddle), the verification engine detects the failure and the AI fixes it using only the manifest diagnosis, not human feedback.

3. Regression tests created from the verification run catch future behavioral regressions with zero false negatives on the tested behaviors.

### Secondary: Performance

| Metric | Target |
|--------|--------|
| Manifest generation overhead | <5% of 16.67ms frame budget at 1K Semantic entities |
| Manifest query latency (single entity) | <10us |
| Verification run (breakout, 6K ticks) | <10 seconds wall-clock |
| WASM hot-swap time | <100ms |
| Snapshot/restore round-trip | <50ms for 1K entities |
| Debug renderer | 60fps at 1K entities (not a priority but shouldn't be broken) |

### Tertiary: Code Quality

| Metric | Target |
|--------|--------|
| Total Rust LOC | <10,000 |
| Total Python LOC | <3,000 |
| Test coverage (Rust, line) | >80% |
| Deterministic replay accuracy | 100% (zero hash drift on same platform) |

---

## 16. Risks and Mitigations

| Risk | Severity | Probability | Mitigation |
|------|----------|-------------|------------|
| **Manifest doesn't capture enough for verification** | Critical | 15% | Phase 0 Spike A tests this directly. If entity queries + events + causality aren't enough, add more manifest layers before committing to full build. |
| **Causality tracking overhead too high** | High | 20% | Causality is metadata on commands, not computation. If overhead is excessive, degrade to system-level causality (which system changed what) instead of full causal chains. |
| **AI can't generate useful intent specs from directives** | High | 15% | This is an LLM capability question, not an engine question. If current models can't decompose "make breakout" into structured assertions, provide example intent specs and have AI adapt them. |
| **Custom ECS takes too long** | Medium | 15% | 2K LOC target. If exceeding 3 weeks on ECS alone, evaluate using hecs as foundation and layering tiered identity on top. |
| **Vision models improve faster than manifest advantage** | Medium | 10% | Run comparison quarterly. Even with perfect vision, structured queries are faster, cheaper, provide causality, and enable regression testing. Vision cannot snapshot-branch-compare. |
| **rapier2d determinism issues** | Medium | 10% | `enhanced-determinism` feature + fixed timestep + same-platform-only for MVP. Known to work in practice for most use cases. |
| **PyO3 ergonomics for manifest API** | Low | 10% | Fallback: JSON serialization across FFI boundary. Slower but guaranteed to work. |

### Honest Probability Assessment

**Success defined as:** Verification thesis validated -- AI completes the breakout experiment end-to-end.

**Probability: 65%.** Significantly higher than v7's 35% because:
- Scope is 60% smaller (no WASM plugins for subsystems, no Luau, no DSL compiler, no GAP)
- Three fatal v7 risks are eliminated (WASM overhead, stateless plugins, cross-language determinism)
- Timeline is 40% shorter (10-13 weeks vs 15-21 weeks)
- Focus is on one provable hypothesis, not a full engine spec
- Failure modes are earlier and cheaper (Phase 0 kills in 4-5 weeks, not 10-16 weeks)

**Remaining failure modes:**
- 15%: Manifest doesn't provide enough semantic information (discovered in Phase 0)
- 10%: AI can't generate useful intent specs (LLM limitation, not engine limitation)
- 5%: Custom ECS takes too long (scope creep risk)
- 5%: Other (dependency issues, hardware failure, life events)

---

## 17. Post-MVP Path

If the verification thesis holds, the path forward is clear. Each item below is only pursued **after MVP validation**.

### Near-Term (MVP + 3-6 months)

| Feature | Why | Depends On |
|---------|-----|------------|
| **Luau scripting layer** | Higher-performance in-tick scripting for gameplay logic | Proven WASM sandbox limitations |
| **Python DSL (Feel/Behavior/Structure blocks)** | Richer authoring abstraction for AI | Proven that directive → code → verify loop works |
| **Full graduated autonomy** | Auto-merge for safe changes, human review for risky changes | Multiple AI agents making concurrent changes |
| **Playtesting agents** | Automated exploration, flow, regression, adversarial testing | Manifest query API is mature |
| **Audio (kira)** | Needed for games beyond simple prototypes | Manifest supports audio events |
| **Cross-platform determinism** | Required for distributed development, networked replay | Same-platform determinism is stable |

### Medium-Term (MVP + 6-12 months)

| Feature | Why | Depends On |
|---------|-----|------------|
| **Networking (quinn)** | Multiplayer games, server-authoritative with client prediction | Change journal replication is proven |
| **Full rendering pipeline** | Beyond debug wireframes: sprites, animation, basic VFX | Manifest visibility model is stable |
| **Self-healing runtime** | Graceful degradation, automatic recovery | Playtesting agents provide failure signals |
| **Live game evolution** | Telemetry-driven balance, A/B testing | Deployed games with player data |
| **DSL compiler pipeline** | IR, validation passes, code generation | DSL surface is stabilized |
| **CI/CD automation** | 8-level test hierarchy, ring-based deployment | Multiple artifacts need automated testing |

### Long-Term (MVP + 12+ months)

| Feature | Why | Depends On |
|---------|-----|------------|
| **Game Autonomy Plane** | Full directive decomposition, autonomous bug fixing, content generation | All near-term and medium-term features |
| **3D support** | Expand beyond 2D game prototypes | 2D rendering pipeline is mature |
| **WASM plugin system for subsystems** | Hot-swappable physics/audio/rendering (v7's original vision) | Performance profiles justify the complexity |
| **Public SDK and documentation** | External developers and AI agents | Engine is stable and documented |

---

## 18. What Changed from v7

### Architectural Changes

| v7 | v8 | Rationale |
|----|----|----|
| WASM plugins for all subsystems (physics, audio, rendering, networking) | Native Rust subsystems, WASM only for gameplay logic | Red team: WASM boundary overhead kills physics, stateful plugins can't be stateless, cross-language determinism breaks |
| Microkernel (~10,750 LOC) + WASM host | Monolithic engine core (~8,400 LOC) | Simpler, faster to build, no boundary overhead |
| Four-layer stack (Rust/WASM/Luau/Python) | Three-layer stack (Rust/WASM-gameplay/Python) | Luau deferred to post-MVP |
| Full DSL compiler pipeline (IR, 6 validation passes, 4 code generators) | Direct code generation, no DSL compiler | Proving the verification thesis doesn't require a compiler |
| Graduated autonomy model (Tier 0/1/2) | Single developer model (no merge pipeline) | MVP has one AI agent, not a team |
| Game Autonomy Plane (5 systems) | Deferred entirely to post-MVP | Prove the core verification loop first |
| 8-level test hierarchy | Minimal test suite (cargo test + pytest) | CI/CD automation is not the thesis |

### Thesis Reframing

| v7 Thesis | v8 Thesis |
|-----------|-----------|
| "AI game development is bottlenecked by closed-loop control quality -- observability, controllability, reproducibility, autonomy" | "AI can write game code trivially. The bottleneck is VERIFICATION. Four pillars enable it: observability (manifest), controllability (APIs), reproducibility (deterministic replay), and autonomy (self-directed iteration). The manifest pipeline is the hardest and most novel." |

v7 identified four co-equal bottlenecks but treated them as independent problems. v8 reframes them as **four pillars of a single goal -- verification** -- with a clear hierarchy of difficulty:

- **Observability** (v7 bottleneck #1) → **The hardest pillar and the core research problem.** The manifest pipeline -- semantic awareness of rendered output -- is what makes verification possible. Without it, the other three pillars are useless.
- **Controllability** (v7 bottleneck #2) → **Essential but solvable with known techniques.** Structured APIs (Python interface, WASM host API, command buffer) give AI full control over the engine. The design challenge is making these APIs AI-friendly, not inventing new paradigms.
- **Reproducibility** (v7 bottleneck #3) → **Essential and well-understood in principle, but requires careful engineering.** Deterministic replay, snapshot/restore, and branching are the tools that turn single-shot verification into regression testing and controlled experimentation.
- **Autonomy** (v7 bottleneck #4) → **The compound result of the other three.** If AI can observe (manifest), control (APIs), and reproduce (replay), then autonomy is a matter of giving it permission and iteration speed. Graduated autonomy is a refinement for post-MVP.

### New in v8: Semantic Art Annotation

v7 had no concept of semantic art metadata. v8 introduces asset annotations -- every art asset (sprite, texture, sound) carries structured metadata describing what it represents, its visual properties, and its relationships. This bridges the gap between "game state" and "player visual experience" in the manifest, enabling AI to verify visual correctness without looking at pixels.

### Scope Changes

| Metric | v7 | v8 |
|--------|----|----|
| MVP timeline | 14-21 weeks | 10-13 weeks |
| Estimated LOC (Rust) | ~10,750 (kernel) + plugins | ~8,400 (everything) |
| Success probability | 35% | 65% |
| Phase 0 spikes | 5 (10-16 weeks) | 2 (4-5 weeks) |
| Team required | 4-5 engineers | 1 developer + AI |
| Languages in MVP | Rust, WASM, Luau, Python | Rust, WASM, Python |

### What's Preserved from v7

Everything that serves the verification thesis is preserved:

- **Tiered entity identity** (Semantic/Pooled/Ephemeral)
- **Dual-output pipeline** concept (framebuffer + manifest)
- **Deterministic tick loop** with command buffer
- **Change journal** as the foundation for manifest deltas
- **Snapshot/restore** for deterministic replay
- **Python as AI interface** language
- **WASM as AI code generation target** (AssemblyScript primary)
- **Wasmtime** as WASM runtime
- **rapier2d** for physics
- **wgpu** for rendering
- **BLAKE3** for content hashing

The engine's DNA is the same. The scope and framing are different.

---

*This document supersedes NOMAI_ENGINE_FINAL_v7.md. It is the authoritative specification for the Nomai Engine MVP. The experiment described in Section 13 is the singular goal. Everything in this document exists to make that experiment succeed.*
