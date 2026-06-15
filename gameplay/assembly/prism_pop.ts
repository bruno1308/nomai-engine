// Prism Pop -- Match-3 gameplay logic for the Nomai engine.
//
// This module implements core match-3 mechanics on an 8x8 grid:
//   - Tile swap validation (adjacent only, must form match)
//   - Match detection (3+ in a row/column)
//   - Matched tile destruction with scoring
//   - Gravity fill (tiles fall down, new tiles spawn at top)
//   - Cascade detection (new matches formed after gravity)
//   - Win condition check (2500 points)
//
// The game is turn-based: state only changes in response to swap inputs
// injected from Python via set_input(). Each tick advances the internal
// state machine one step through the cascade resolution.
//
// State machine phases:
//   IDLE        -> waiting for swap input
//   SWAPPING    -> validating and executing a swap
//   MATCHING    -> scanning grid for matches of 3+
//   DESTROYING  -> removing matched tiles, awarding points
//   FALLING     -> applying gravity, spawning new tiles
//   CHECKING    -> re-scanning for cascade matches
//   WIN         -> player reached 2500 points
//
// Design:
// - Grid state is stored as a flat array of 64 tile types (0-4).
// - Entity IDs for each cell are tracked so we can set_component on them.
// - All state mutations go through the host API with causal reasons.
// - Every command carries a reason string for manifest causality.

import {
  get_entity_count,
  tick_number,
  set_component,
  spawn_semantic,
  despawn_entity,
  emit_event,
  log_msg,
  get_component,
} from "./host";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GRID_SIZE: i32 = 8;
const TILE_TYPES: i32 = 5; // Ruby=0, Sapphire=1, Emerald=2, Topaz=3, Amethyst=4
const WIN_SCORE: i32 = 2500;
const MAX_CASCADE: i32 = 20;

const POINTS_3: i32 = 300;
const POINTS_4: i32 = 500;
const POINTS_5: i32 = 1000;

// State machine phases
const PHASE_IDLE: i32 = 0;
const PHASE_SWAPPING: i32 = 1;
const PHASE_MATCHING: i32 = 2;
const PHASE_DESTROYING: i32 = 3;
const PHASE_FALLING: i32 = 4;
const PHASE_CHECKING: i32 = 5;
const PHASE_WIN: i32 = 6;

// Tile type names for events
const TILE_NAMES: string[] = ["ruby", "sapphire", "emerald", "topaz", "amethyst"];

// ---------------------------------------------------------------------------
// Game state (resets on module swap -- persistent state is in ECS)
// ---------------------------------------------------------------------------

// Grid: flat array [row * GRID_SIZE + col] -> tile_type (0-4), -1 = empty
let grid: i32[] = new Array<i32>(GRID_SIZE * GRID_SIZE);

// Entity IDs for each grid cell (assigned during init)
let entityIds: i64[] = new Array<i64>(GRID_SIZE * GRID_SIZE);

// Deterministic RNG seed (set from host on init)
let rngState: u32 = 42;

let score: i32 = 0;
let phase: i32 = PHASE_IDLE;
let cascadeDepth: i32 = 0;
let initialized: bool = false;

// Pending swap input (set by handleSwap, consumed in SWAPPING phase)
let swapRow1: i32 = -1;
let swapCol1: i32 = -1;
let swapRow2: i32 = -1;
let swapCol2: i32 = -1;
let hasPendingSwap: bool = false;

// Matched cells bitmap (64 bits would be ideal, use bool array)
let matched: bool[] = new Array<bool>(GRID_SIZE * GRID_SIZE);

// Score entity ID (entity 0 by convention, same as breakout)
let scoreEntityId: i64 = 0;
// Grid entity ID
let gridEntityId: i64 = 0;

// Track total matches for events
let matchesThisCascade: i32 = 0;
let tilesDestroyedThisCascade: i32 = 0;

// ---------------------------------------------------------------------------
// Deterministic RNG (xorshift32)
// ---------------------------------------------------------------------------

function nextRandom(): u32 {
  rngState ^= rngState << 13;
  rngState ^= rngState >> 17;
  rngState ^= rngState << 5;
  return rngState;
}

function randomTileType(): i32 {
  return i32(nextRandom() % u32(TILE_TYPES));
}

// ---------------------------------------------------------------------------
// Grid helpers
// ---------------------------------------------------------------------------

function idx(row: i32, col: i32): i32 {
  return row * GRID_SIZE + col;
}

function inBounds(row: i32, col: i32): bool {
  return row >= 0 && row < GRID_SIZE && col >= 0 && col < GRID_SIZE;
}

function isAdjacent(r1: i32, c1: i32, r2: i32, c2: i32): bool {
  const dr = abs(r1 - r2);
  const dc = abs(c1 - c2);
  return (dr == 1 && dc == 0) || (dr == 0 && dc == 1);
}

function abs(v: i32): i32 {
  return v < 0 ? -v : v;
}

// ---------------------------------------------------------------------------
// Grid initialization -- fill with random tiles, no initial matches
// ---------------------------------------------------------------------------

function initGrid(): void {
  for (let r: i32 = 0; r < GRID_SIZE; r++) {
    for (let c: i32 = 0; c < GRID_SIZE; c++) {
      let tile: i32 = randomTileType();
      // Avoid creating initial matches of 3
      while (createsMatch(r, c, tile)) {
        tile = (tile + 1) % TILE_TYPES;
      }
      grid[idx(r, c)] = tile;
    }
  }
}

function createsMatch(row: i32, col: i32, tile: i32): bool {
  // Check horizontal: 2 tiles to the left
  if (col >= 2 && grid[idx(row, col - 1)] == tile && grid[idx(row, col - 2)] == tile) {
    return true;
  }
  // Check vertical: 2 tiles above
  if (row >= 2 && grid[idx(row - 1, col)] == tile && grid[idx(row - 2, col)] == tile) {
    return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// Push grid state to ECS
// ---------------------------------------------------------------------------

function pushTileToECS(row: i32, col: i32): void {
  const i = idx(row, col);
  const eid = entityIds[i];
  const tile = grid[i];

  set_component(
    eid,
    "position",
    '{"x":' + col.toString() + ',"y":' + row.toString() + '}',
    "tile_position_set"
  );

  set_component(
    eid,
    "tile_type",
    '{"type_id":' + tile.toString() + ',"name":"' + TILE_NAMES[tile] + '"}',
    "tile_type_set"
  );

  set_component(
    eid,
    "size",
    '{"w":1,"h":1}',
    "tile_size_set"
  );
}

function pushScoreToECS(): void {
  set_component(
    scoreEntityId,
    "score_value",
    '{"points":' + score.toString() + '}',
    "score_updated"
  );
}

function pushGridStateToECS(): void {
  // Build grid state JSON
  let validMoves: i32 = countValidMoves();
  set_component(
    gridEntityId,
    "state",
    '{"valid_moves_count":' + validMoves.toString() +
    ',"cascade_depth":' + cascadeDepth.toString() +
    ',"phase":' + phase.toString() + '}',
    "grid_state_updated"
  );
}

function pushAllTilesToECS(): void {
  for (let r: i32 = 0; r < GRID_SIZE; r++) {
    for (let c: i32 = 0; c < GRID_SIZE; c++) {
      pushTileToECS(r, c);
    }
  }
}

// ---------------------------------------------------------------------------
// Match detection
// ---------------------------------------------------------------------------

function clearMatched(): void {
  for (let i: i32 = 0; i < GRID_SIZE * GRID_SIZE; i++) {
    matched[i] = false;
  }
}

function findMatches(): i32 {
  clearMatched();
  let matchCount: i32 = 0;

  // Horizontal matches
  for (let r: i32 = 0; r < GRID_SIZE; r++) {
    let runStart: i32 = 0;
    let runLen: i32 = 1;
    for (let c: i32 = 1; c < GRID_SIZE; c++) {
      if (grid[idx(r, c)] == grid[idx(r, runStart)] && grid[idx(r, c)] >= 0) {
        runLen++;
      } else {
        if (runLen >= 3) {
          matchCount++;
          for (let k: i32 = runStart; k < runStart + runLen; k++) {
            matched[idx(r, k)] = true;
          }
        }
        runStart = c;
        runLen = 1;
      }
    }
    if (runLen >= 3) {
      matchCount++;
      for (let k: i32 = runStart; k < runStart + runLen; k++) {
        matched[idx(r, k)] = true;
      }
    }
  }

  // Vertical matches
  for (let c: i32 = 0; c < GRID_SIZE; c++) {
    let runStart: i32 = 0;
    let runLen: i32 = 1;
    for (let r: i32 = 1; r < GRID_SIZE; r++) {
      if (grid[idx(r, c)] == grid[idx(runStart, c)] && grid[idx(r, c)] >= 0) {
        runLen++;
      } else {
        if (runLen >= 3) {
          matchCount++;
          for (let k: i32 = runStart; k < runStart + runLen; k++) {
            matched[idx(k, c)] = true;
          }
        }
        runStart = r;
        runLen = 1;
      }
    }
    if (runLen >= 3) {
      matchCount++;
      for (let k: i32 = runStart; k < runStart + runLen; k++) {
        matched[idx(k, c)] = true;
      }
    }
  }

  return matchCount;
}

// ---------------------------------------------------------------------------
// Score calculation for matched tiles
// ---------------------------------------------------------------------------

function scoreMatches(): i32 {
  let points: i32 = 0;
  let tilesDestroyed: i32 = 0;

  // Count matched tiles per horizontal run for scoring
  for (let r: i32 = 0; r < GRID_SIZE; r++) {
    let runLen: i32 = 0;
    for (let c: i32 = 0; c < GRID_SIZE; c++) {
      if (matched[idx(r, c)]) {
        runLen++;
      } else {
        if (runLen >= 3) {
          points += scoreForRun(runLen);
        }
        runLen = 0;
      }
    }
    if (runLen >= 3) {
      points += scoreForRun(runLen);
    }
  }

  // Count matched tiles per vertical run (avoid double-counting points
  // for tiles already scored in horizontal runs -- but match-3 games
  // typically DO score both directions, so we count both)
  for (let c: i32 = 0; c < GRID_SIZE; c++) {
    let runLen: i32 = 0;
    for (let r: i32 = 0; r < GRID_SIZE; r++) {
      if (matched[idx(r, c)]) {
        runLen++;
      } else {
        if (runLen >= 3) {
          points += scoreForRun(runLen);
        }
        runLen = 0;
      }
    }
    if (runLen >= 3) {
      points += scoreForRun(runLen);
    }
  }

  // Count destroyed tiles
  for (let i: i32 = 0; i < GRID_SIZE * GRID_SIZE; i++) {
    if (matched[i]) {
      tilesDestroyed++;
    }
  }

  tilesDestroyedThisCascade += tilesDestroyed;
  return points;
}

function scoreForRun(length: i32): i32 {
  if (length >= 5) return POINTS_5;
  if (length >= 4) return POINTS_4;
  return POINTS_3;
}

// ---------------------------------------------------------------------------
// Destroy matched tiles (mark as empty)
// ---------------------------------------------------------------------------

function destroyMatched(): void {
  for (let i: i32 = 0; i < GRID_SIZE * GRID_SIZE; i++) {
    if (matched[i]) {
      grid[i] = -1; // empty
    }
  }
}

// ---------------------------------------------------------------------------
// Gravity: tiles fall into empty spaces, new tiles spawn at top
// ---------------------------------------------------------------------------

function applyGravity(): void {
  for (let c: i32 = 0; c < GRID_SIZE; c++) {
    // Compact column: move tiles down to fill gaps
    let writeRow: i32 = GRID_SIZE - 1;
    for (let r: i32 = GRID_SIZE - 1; r >= 0; r--) {
      if (grid[idx(r, c)] >= 0) {
        grid[idx(writeRow, c)] = grid[idx(r, c)];
        if (writeRow != r) {
          grid[idx(r, c)] = -1;
        }
        writeRow--;
      }
    }
    // Fill empty cells at the top with new random tiles
    for (let r: i32 = writeRow; r >= 0; r--) {
      grid[idx(r, c)] = randomTileType();
    }
  }
}

// ---------------------------------------------------------------------------
// Valid move detection (for deadlock check)
// ---------------------------------------------------------------------------

function countValidMoves(): i32 {
  let count: i32 = 0;
  for (let r: i32 = 0; r < GRID_SIZE; r++) {
    for (let c: i32 = 0; c < GRID_SIZE; c++) {
      // Try swap right
      if (c + 1 < GRID_SIZE) {
        swapInGrid(r, c, r, c + 1);
        if (hasMatchAt(r, c) || hasMatchAt(r, c + 1)) count++;
        swapInGrid(r, c, r, c + 1); // swap back
      }
      // Try swap down
      if (r + 1 < GRID_SIZE) {
        swapInGrid(r, c, r + 1, c);
        if (hasMatchAt(r, c) || hasMatchAt(r + 1, c)) count++;
        swapInGrid(r, c, r + 1, c); // swap back
      }
    }
  }
  return count;
}

function swapInGrid(r1: i32, c1: i32, r2: i32, c2: i32): void {
  const temp = grid[idx(r1, c1)];
  grid[idx(r1, c1)] = grid[idx(r2, c2)];
  grid[idx(r2, c2)] = temp;
}

function hasMatchAt(row: i32, col: i32): bool {
  const tile = grid[idx(row, col)];
  if (tile < 0) return false;

  // Check horizontal
  let hCount: i32 = 1;
  let c = col - 1;
  while (c >= 0 && grid[idx(row, c)] == tile) { hCount++; c--; }
  c = col + 1;
  while (c < GRID_SIZE && grid[idx(row, c)] == tile) { hCount++; c++; }
  if (hCount >= 3) return true;

  // Check vertical
  let vCount: i32 = 1;
  let r = row - 1;
  while (r >= 0 && grid[idx(r, col)] == tile) { vCount++; r--; }
  r = row + 1;
  while (r < GRID_SIZE && grid[idx(r, col)] == tile) { vCount++; r++; }
  return vCount >= 3;
}

// ---------------------------------------------------------------------------
// Exported: tick() -- main entry point, called once per engine tick
// ---------------------------------------------------------------------------

export function tick(): void {
  const currentTick: i64 = tick_number();

  if (!initialized) {
    doInit();
    initialized = true;
    log_msg(2, "Prism Pop initialized at tick " + currentTick.toString());
    return;
  }

  // State machine
  if (phase == PHASE_IDLE) {
    if (hasPendingSwap) {
      phase = PHASE_SWAPPING;
      hasPendingSwap = false;
    }
    // else: wait for input
  }

  if (phase == PHASE_SWAPPING) {
    doSwap();
  } else if (phase == PHASE_MATCHING) {
    doMatching();
  } else if (phase == PHASE_DESTROYING) {
    doDestroying();
  } else if (phase == PHASE_FALLING) {
    doFalling();
  } else if (phase == PHASE_CHECKING) {
    doChecking();
  } else if (phase == PHASE_WIN) {
    // Stay in win state
  }

  // Push grid state every tick
  pushGridStateToECS();
}

// ---------------------------------------------------------------------------
// Initialization: spawn all entities
// ---------------------------------------------------------------------------

function doInit(): void {
  // Spawn grid entity
  gridEntityId = spawn_semantic(
    '{"entity_type":"container","role":"grid"}',
    '{"position":{"x":0,"y":0},"size":{"w":8,"h":8}}',
    "grid_spawn"
  );

  // Spawn score entity
  scoreEntityId = spawn_semantic(
    '{"entity_type":"ui","role":"score"}',
    '{"position":{"x":0,"y":0},"score_value":{"points":0}}',
    "score_spawn"
  );

  // Initialize grid with random tiles (no initial matches)
  initGrid();

  // Spawn tile entities
  for (let r: i32 = 0; r < GRID_SIZE; r++) {
    for (let c: i32 = 0; c < GRID_SIZE; c++) {
      const tile = grid[idx(r, c)];
      const eid = spawn_semantic(
        '{"entity_type":"tile","role":"tile"}',
        '{"position":{"x":' + c.toString() + ',"y":' + r.toString() + '},' +
        '"size":{"w":1,"h":1},' +
        '"tile_type":{"type_id":' + tile.toString() + ',"name":"' + TILE_NAMES[tile] + '"}}',
        "tile_spawn_at_" + r.toString() + "_" + c.toString()
      );
      entityIds[idx(r, c)] = eid;
    }
  }

  phase = PHASE_IDLE;
  score = 0;
  cascadeDepth = 0;

  pushScoreToECS();
  pushGridStateToECS();

  log_msg(2, "Grid initialized with " + (GRID_SIZE * GRID_SIZE).toString() + " tiles");
}

// ---------------------------------------------------------------------------
// Phase handlers
// ---------------------------------------------------------------------------

function doSwap(): void {
  if (!inBounds(swapRow1, swapCol1) || !inBounds(swapRow2, swapCol2)) {
    log_msg(3, "Invalid swap coordinates");
    phase = PHASE_IDLE;
    return;
  }

  if (!isAdjacent(swapRow1, swapCol1, swapRow2, swapCol2)) {
    log_msg(3, "Swap not adjacent");
    emitSwapEvent("swap_rejected", "not_adjacent");
    phase = PHASE_IDLE;
    return;
  }

  // Execute swap
  swapInGrid(swapRow1, swapCol1, swapRow2, swapCol2);

  // Check if swap creates any match
  if (hasMatchAt(swapRow1, swapCol1) || hasMatchAt(swapRow2, swapCol2)) {
    // Valid swap
    pushTileToECS(swapRow1, swapCol1);
    pushTileToECS(swapRow2, swapCol2);

    // Also swap entity IDs
    const tempId = entityIds[idx(swapRow1, swapCol1)];
    entityIds[idx(swapRow1, swapCol1)] = entityIds[idx(swapRow2, swapCol2)];
    entityIds[idx(swapRow2, swapCol2)] = tempId;

    emitSwapEvent("swap_accepted", "match_formed");
    cascadeDepth = 0;
    matchesThisCascade = 0;
    tilesDestroyedThisCascade = 0;
    phase = PHASE_MATCHING;
    log_msg(2, "Swap accepted: (" + swapRow1.toString() + "," + swapCol1.toString() +
      ") <-> (" + swapRow2.toString() + "," + swapCol2.toString() + ")");
  } else {
    // Invalid swap -- snap back
    swapInGrid(swapRow1, swapCol1, swapRow2, swapCol2);
    emitSwapEvent("swap_rejected", "no_match");
    phase = PHASE_IDLE;
    log_msg(2, "Swap rejected: no match formed");
  }
}

function doMatching(): void {
  const matchCount = findMatches();
  if (matchCount > 0) {
    matchesThisCascade += matchCount;
    phase = PHASE_DESTROYING;
  } else {
    // No matches found -- cascade is done
    finishCascade();
  }
}

function doDestroying(): void {
  const points = scoreMatches();
  score += points;
  pushScoreToECS();

  // Emit match event
  emitMatchEvent(points);

  // Remove matched tiles from grid
  destroyMatched();

  // Update tile ECS components for destroyed tiles
  for (let i: i32 = 0; i < GRID_SIZE * GRID_SIZE; i++) {
    if (matched[i]) {
      set_component(
        entityIds[i],
        "tile_type",
        '{"type_id":-1,"name":"empty"}',
        "tile_destroyed_by_match"
      );
    }
  }

  log_msg(2, "Destroyed matched tiles, +" + points.toString() +
    " points, total: " + score.toString());

  // Check win
  if (score >= WIN_SCORE) {
    phase = PHASE_WIN;
    emitWinEvent();
    log_msg(2, "WIN! Score: " + score.toString());
    return;
  }

  phase = PHASE_FALLING;
}

function doFalling(): void {
  applyGravity();
  pushAllTilesToECS();
  cascadeDepth++;

  if (cascadeDepth > MAX_CASCADE) {
    log_msg(4, "Max cascade depth exceeded! Breaking cascade loop.");
    emitDegenerateEvent("infinite_cascade");
    finishCascade();
    return;
  }

  phase = PHASE_CHECKING;
}

function doChecking(): void {
  // Check for new matches after gravity
  const matchCount = findMatches();
  if (matchCount > 0) {
    matchesThisCascade += matchCount;
    log_msg(2, "Cascade! Depth " + cascadeDepth.toString() +
      ", found " + matchCount.toString() + " new match(es)");
    phase = PHASE_DESTROYING;
  } else {
    finishCascade();
  }
}

function finishCascade(): void {
  if (matchesThisCascade > 0) {
    emitCascadeCompleteEvent();
  }
  phase = PHASE_IDLE;
}

// ---------------------------------------------------------------------------
// Exported: handleSwap() -- called by engine when Python injects a swap
// ---------------------------------------------------------------------------

export function handleSwap(
  row1: i32,
  col1: i32,
  row2: i32,
  col2: i32
): void {
  swapRow1 = row1;
  swapCol1 = col1;
  swapRow2 = row2;
  swapCol2 = col2;
  hasPendingSwap = true;

  log_msg(1, "Swap queued: (" + row1.toString() + "," + col1.toString() +
    ") <-> (" + row2.toString() + "," + col2.toString() + ")");
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

function emitSwapEvent(eventType: string, detail: string): void {
  emit_event(
    '{"event_type":"' + eventType + '",' +
    '"description":"Tile swap at (' + swapRow1.toString() + ',' + swapCol1.toString() +
    ') <-> (' + swapRow2.toString() + ',' + swapCol2.toString() + '): ' + detail + '",' +
    '"involved_entities":[],' +
    '"caused_by":0,' +
    '"reason":{"GameRule":"' + detail + '"},' +
    '"tick":' + tick_number().toString() + '}'
  );
}

function emitMatchEvent(points: i32): void {
  emit_event(
    '{"event_type":"match_scored",' +
    '"description":"Match scored ' + points.toString() + ' points, total: ' + score.toString() + '",' +
    '"involved_entities":[],' +
    '"caused_by":0,' +
    '"reason":{"GameRule":"match_scored"},' +
    '"tick":' + tick_number().toString() + '}'
  );
}

function emitCascadeCompleteEvent(): void {
  emit_event(
    '{"event_type":"cascade_complete",' +
    '"description":"Cascade resolved: depth ' + cascadeDepth.toString() +
    ', matches ' + matchesThisCascade.toString() +
    ', tiles destroyed ' + tilesDestroyedThisCascade.toString() + '",' +
    '"involved_entities":[],' +
    '"caused_by":0,' +
    '"reason":{"GameRule":"cascade_complete"},' +
    '"tick":' + tick_number().toString() + '}'
  );
}

function emitWinEvent(): void {
  emit_event(
    '{"event_type":"game_won",' +
    '"description":"Player reached ' + score.toString() + ' points (target: ' + WIN_SCORE.toString() + ')",' +
    '"involved_entities":[],' +
    '"caused_by":0,' +
    '"reason":{"GameRule":"win_condition_reached"},' +
    '"tick":' + tick_number().toString() + '}'
  );
}

function emitDegenerateEvent(stateType: string): void {
  emit_event(
    '{"event_type":"degenerate_state",' +
    '"description":"Degenerate state detected: ' + stateType + '",' +
    '"involved_entities":[],' +
    '"caused_by":0,' +
    '"reason":{"GameRule":"degenerate_' + stateType + '"},' +
    '"tick":' + tick_number().toString() + '}'
  );
}

// ---------------------------------------------------------------------------
// Test helpers -- exported so host can inspect module state
// ---------------------------------------------------------------------------

export function get_score(): i32 {
  return score;
}

export function get_phase(): i32 {
  return phase;
}

export function get_cascade_depth(): i32 {
  return cascadeDepth;
}

export function get_tile_at(row: i32, col: i32): i32 {
  if (!inBounds(row, col)) return -1;
  return grid[idx(row, col)];
}

export function get_valid_move_count(): i32 {
  return countValidMoves();
}
