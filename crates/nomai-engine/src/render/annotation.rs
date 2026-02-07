//! Semantic art annotation -- convention-based asset path parsing.
//!
//! Parses asset file paths against a naming convention to produce
//! [`AssetAnnotation`] structs that describe what a sprite or tile
//! represents. This feeds into the manifest's visual layer so the AI
//! verification loop can answer questions like "is the right sprite
//! showing for this state?"
//!
//! # Path Convention
//!
//! ```text
//! assets/sprites/{entity_type}/{state}_{direction}.png
//! assets/tiles/{tile_type}/{variant}.png
//! assets/ui/{element_name}.png
//! ```
//!
//! # Example
//!
//! ```
//! use nomai_engine::render::annotation::parse_asset_path;
//!
//! let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
//! assert_eq!(ann.entity_type, "player");
//! assert_eq!(ann.represents, "player.idle.facing_right");
//! assert!(ann.tags.contains(&"idle".to_string()));
//! ```

use serde::{Deserialize, Serialize};

/// Semantic annotation derived from an asset file path.
///
/// Produced by [`parse_asset_path`] using directory and filename
/// conventions. The annotation describes what the asset visually
/// represents, making it queryable by the AI verification engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetAnnotation {
    /// The original file path (e.g. `"assets/sprites/player/idle_right.png"`).
    pub asset_path: String,

    /// High-level type derived from the directory structure.
    ///
    /// Examples: `"character_sprite"`, `"tile"`, `"ui_element"`.
    pub semantic_type: String,

    /// Dot-separated semantic description of what the asset depicts.
    ///
    /// Examples: `"player.idle.facing_right"`, `"tile.grass.normal"`,
    /// `"ui.health_bar"`.
    pub represents: String,

    /// Searchable tags extracted from the path components.
    pub tags: Vec<String>,

    /// The entity type this asset belongs to (from directory name).
    ///
    /// Examples: `"player"`, `"enemy"`, `"grass"`.
    pub entity_type: String,
}

/// Known direction keywords that indicate sprite facing.
const DIRECTION_KEYWORDS: &[&str] = &[
    "left", "right", "up", "down", "north", "south", "east", "west",
];

/// Parse an asset file path into a semantic annotation.
///
/// Returns `None` if the path does not start with `"assets/"` or does
/// not have enough components to extract meaningful information.
///
/// # Path Convention
///
/// The parser expects paths of the form:
/// - `assets/sprites/{entity_type}/{state}_{direction}.png` -- character sprites
/// - `assets/tiles/{tile_type}/{variant}.png` -- tile textures
/// - `assets/ui/{element_name}.png` -- UI elements
///
/// Filename parts separated by underscores are split into state and
/// optional direction. If the last part matches a known direction keyword,
/// it becomes a `"facing_{direction}"` suffix.
pub fn parse_asset_path(path: &str) -> Option<AssetAnnotation> {
    // Normalize separators to forward slash.
    let normalized = path.replace('\\', "/");

    // Must start with "assets/".
    if !normalized.starts_with("assets/") {
        return None;
    }

    // Strip the "assets/" prefix and split into components.
    let without_prefix = &normalized["assets/".len()..];
    let parts: Vec<&str> = without_prefix.split('/').collect();

    // Need at least a category and a filename.
    if parts.len() < 2 {
        return None;
    }

    let category = parts[0]; // "sprites", "tiles", "ui"

    // Get the filename and strip the extension.
    let filename_with_ext = *parts.last()?;
    let filename = filename_with_ext
        .rsplit_once('.')
        .map(|(name, _ext)| name)
        .unwrap_or(filename_with_ext);

    if filename.is_empty() {
        return None;
    }

    // Determine semantic_type from category.
    let semantic_type = match category {
        "sprites" => {
            // Check if this is a character-like entity based on common names,
            // otherwise default to "sprite".
            "character_sprite".to_owned()
        }
        "tiles" => "tile".to_owned(),
        "ui" => "ui_element".to_owned(),
        other => other.to_owned(),
    };

    // Extract entity_type from the middle path component(s).
    // For sprites: assets/sprites/{entity_type}/...
    // For tiles:   assets/tiles/{tile_type}/...
    // For ui:      entity_type is the filename itself
    let entity_type = if parts.len() >= 3 {
        // There is a subdirectory between category and filename.
        parts[1].to_owned()
    } else {
        // Only category/filename -- entity_type is derived from filename.
        filename.to_owned()
    };

    // Split filename by underscores to extract state and direction.
    let filename_parts: Vec<&str> = filename.split('_').collect();

    // Check for direction keyword, handling optional trailing frame numbers.
    //
    // Patterns handled:
    //   "idle_right"      -> state="idle", direction="right"
    //   "walk_right_01"   -> state="walk", direction="right" (01 is frame index)
    //   "idle"            -> state="idle", direction=None
    let (state_parts, direction) = if filename_parts.len() > 1 {
        let last = *filename_parts.last().unwrap();
        let last_clean = last.trim_end_matches(|c: char| c.is_ascii_digit());

        if DIRECTION_KEYWORDS.contains(&last_clean) {
            // Last part is a direction (possibly with trailing digits like "right01").
            (
                &filename_parts[..filename_parts.len() - 1],
                Some(last_clean),
            )
        } else if last_clean.is_empty() && filename_parts.len() > 2 {
            // Last part is purely numeric (frame index like "01").
            // Check the second-to-last part for a direction keyword.
            let second_last = filename_parts[filename_parts.len() - 2];
            if DIRECTION_KEYWORDS.contains(&second_last) {
                (
                    &filename_parts[..filename_parts.len() - 2],
                    Some(second_last),
                )
            } else {
                (&filename_parts[..], None)
            }
        } else {
            (&filename_parts[..], None)
        }
    } else {
        (&filename_parts[..], None)
    };

    // Build the state string from remaining filename parts.
    let state = state_parts.join("_");

    // Build the "represents" string.
    let represents = match category {
        "sprites" => {
            let mut repr = format!("{entity_type}.{state}");
            if let Some(dir) = direction {
                repr.push_str(&format!(".facing_{dir}"));
            }
            repr
        }
        "tiles" => {
            format!("tile.{entity_type}.{state}")
        }
        "ui" => {
            format!("ui.{state}")
        }
        _ => {
            format!("{category}.{entity_type}.{state}")
        }
    };

    // Build tags.
    let mut tags = Vec::new();
    tags.push(entity_type.clone());
    for part in &filename_parts {
        let tag = part.to_string();
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    if let Some(dir) = direction {
        let dir_tag = dir.to_string();
        if !tags.contains(&dir_tag) {
            tags.push(dir_tag);
        }
    }
    // Add the category as a tag.
    let cat_tag = category.to_string();
    if !tags.contains(&cat_tag) {
        tags.push(cat_tag);
    }

    Some(AssetAnnotation {
        asset_path: path.to_owned(),
        semantic_type,
        represents,
        tags,
        entity_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_player_idle_right() {
        let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
        assert_eq!(ann.entity_type, "player");
        assert_eq!(ann.semantic_type, "character_sprite");
        assert_eq!(ann.represents, "player.idle.facing_right");
        assert!(ann.tags.contains(&"player".to_string()));
        assert!(ann.tags.contains(&"idle".to_string()));
        assert!(ann.tags.contains(&"right".to_string()));
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_asset_path("not_assets/random.txt").is_none());
        assert!(parse_asset_path("assets/").is_none());
        assert!(parse_asset_path("").is_none());
    }

    #[test]
    fn parse_tile_path() {
        let ann = parse_asset_path("assets/tiles/grass/normal.png").unwrap();
        assert_eq!(ann.entity_type, "grass");
        assert_eq!(ann.semantic_type, "tile");
        assert_eq!(ann.represents, "tile.grass.normal");
    }

    #[test]
    fn parse_ui_path() {
        let ann = parse_asset_path("assets/ui/health_bar.png").unwrap();
        assert_eq!(ann.semantic_type, "ui_element");
        assert_eq!(ann.represents, "ui.health_bar");
    }

    #[test]
    fn serializes_to_json() {
        let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
        let json = serde_json::to_string(&ann).expect("should serialize to JSON");
        assert!(json.contains("player.idle.facing_right"));
    }
}
