Certainly! Below is the Game Design Document formatted specifically for a `.md` (Markdown) file. You can copy this text into any markdown editor (like Obsidian, VS Code, or even Notepad saved with a `.md` extension) and it will render perfectly.

---

# Game Design Document: Prism Pop (Prototype)

## 1. Project Overview

**Prism Pop** is a high-polish, single-level match-3 experience for Windows PC. The primary objective is to demonstrate "Juiciness"—maximizing player satisfaction through extreme visual and auditory feedback.

* **Genre:** Puzzle / "Juicy" Arcade
* **Platform:** Windows PC (Mouse Input)
* **Target Score:** 2,500 Points

---

## 2. Core Gameplay Mechanics

The game utilizes a standard swap-to-match mechanic on a fixed grid.

### 2.1 The Grid

* **Size:** 8 x 8 (64 tiles total).
* **Tile Types:** 5 distinct jewel types (Ruby, Sapphire, Emerald, Topaz, Amethyst).

### 2.2 Movement & Matching

* **Swapping:** Players click a tile and drag to an adjacent slot (Up, Down, Left, Right).
* **Validation:** If a match of 3 or more is formed, the match is processed. If not, tiles perform a "Snap-Back" animation with an elastic wobble.
* **Gravity:** Empty spaces are filled by tiles falling from above. New tiles spawn at the top of the screen to maintain a full board.

---

## 3. The "Juice" Specifications

The "Juice" refers to the non-functional aesthetic feedback that makes the game feel alive.

### 3.1 Visual Feedback (The Sparkle)

| Action | Visual Effect |
| --- | --- |
| **Idle** | Tiles have a subtle "breathing" scale animation and a slow-moving internal glint. |
| **Successful Match** | Tiles shatter into 10–15 physics-based glowing shards of the same color. |
| **Combos** | Consecutive matches trigger a "Screen Shake" that increases in intensity with each combo step. |
| **The Cascade** | Falling tiles leave a faint, colorful motion trail (ghosting effect). |
| **Landing** | When a tile hits its final position, it performs a **Squash and Stretch** (briefly flattens then bounces back). |

### 3.2 UI & Text

* **Floating Text:** Dynamic text like *"Sparkly!"*, *"Juicy!"*, and *"Mega!"* pops up at the match location using a bouncy, spring-physics animation.
* **Score Tally:** The score counter "jumps" and flashes white whenever points are added, smoothly counting up rather than snapping to the new number.

---

## 4. Technical Specs (Windows)

* **Resolution:** 1920 x 1080 (16:9 Aspect Ratio).
* **Input:** Left Mouse Click + Drag.
* **Framerate:** Target 60 FPS (mandatory for fluid particle physics).
* **Engine Recommendation:** Unity (2D Universal Render Pipeline) or Godot.

---

## 5. Level & Win Condition: "Emerald Origin"

This prototype features one hard-coded level designed to be completed in 1–2 minutes of play.

* **Point System:**
* **3-Match:** 300 Points
* **4-Match:** 500 Points + Small Explosion (clears adjacent tiles)
* **5-Match:** 1,000 Points + "Prism Blast" (clears all tiles of that color)


* **Win State:** Upon reaching **2,500 points**, the game freezes for a "Big Freeze" (0.5 seconds), followed by a massive explosion of all remaining tiles on the board, leading to a "Level Complete" screen.

---

## 6. Asset List

1. **Sprites:** 5 Gem Prisms (High Saturation).
2. **Particles:** Glow flares, star-shaped sparkles, and shard fragments.
3. **SFX:** * High-pitched "chime" for matches.
* Deep "thud" for tile landings.
* Synthesized "whoosh" for swaps.



---

**Would you like me to generate a starter Python script (using Pygame) or a C# snippet for Unity that handles the "Squash and Stretch" logic described in this document?**