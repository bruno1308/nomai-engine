//! Integration tests for semantic art annotation and text rendering.
//!
//! Tests both the asset path parsing pipeline and the glyph-based text
//! renderer. These are pure CPU tests -- no GPU context required.

#[cfg(feature = "renderer")]
mod tests {
    use nomai_engine::render::annotation::{parse_asset_path, AssetAnnotation};
    use nomai_engine::render::text::TextRenderer;

    // -----------------------------------------------------------------------
    // Asset annotation tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_player_sprite_path() {
        let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
        assert_eq!(ann.entity_type, "player");
        assert_eq!(ann.semantic_type, "character_sprite");
        assert_eq!(ann.represents, "player.idle.facing_right");
        assert!(ann.tags.contains(&"player".to_string()));
        assert!(ann.tags.contains(&"idle".to_string()));
        assert!(ann.tags.contains(&"right".to_string()));
        assert_eq!(ann.asset_path, "assets/sprites/player/idle_right.png");
    }

    #[test]
    fn parse_enemy_sprite_path() {
        let ann = parse_asset_path("assets/sprites/enemy/run_left.png").unwrap();
        assert_eq!(ann.entity_type, "enemy");
        assert_eq!(ann.semantic_type, "character_sprite");
        assert_eq!(ann.represents, "enemy.run.facing_left");
        assert!(ann.tags.contains(&"enemy".to_string()));
        assert!(ann.tags.contains(&"run".to_string()));
        assert!(ann.tags.contains(&"left".to_string()));
    }

    #[test]
    fn parse_tile_path() {
        let ann = parse_asset_path("assets/tiles/grass/normal.png").unwrap();
        assert_eq!(ann.entity_type, "grass");
        assert_eq!(ann.semantic_type, "tile");
        assert_eq!(ann.represents, "tile.grass.normal");
        assert!(ann.tags.contains(&"grass".to_string()));
        assert!(ann.tags.contains(&"normal".to_string()));
    }

    #[test]
    fn parse_invalid_path_returns_none() {
        assert!(
            parse_asset_path("not_assets/random.txt").is_none(),
            "path not starting with assets/ should return None"
        );
        assert!(
            parse_asset_path("").is_none(),
            "empty path should return None"
        );
        assert!(
            parse_asset_path("assets/").is_none(),
            "path with only assets/ should return None"
        );
        assert!(
            parse_asset_path("random.png").is_none(),
            "bare filename should return None"
        );
    }

    #[test]
    fn parse_ui_element_path() {
        let ann = parse_asset_path("assets/ui/health_bar.png").unwrap();
        assert_eq!(ann.semantic_type, "ui_element");
        assert_eq!(ann.represents, "ui.health_bar");
    }

    #[test]
    fn parse_animation_frame_path() {
        let ann = parse_asset_path("assets/sprites/player/walk_right_01.png").unwrap();
        assert_eq!(ann.entity_type, "player");
        // "walk_right_01" -- "right" should still be detected as direction
        // even with trailing digits.
        assert!(
            ann.represents.contains("facing_right"),
            "should detect direction despite trailing digits, got: {}",
            ann.represents
        );
    }

    #[test]
    fn parse_nested_enemy_path() {
        // Deeper nesting: assets/sprites/enemy/melee/idle.png
        let ann = parse_asset_path("assets/sprites/enemy/melee/idle.png").unwrap();
        assert_eq!(ann.entity_type, "enemy");
        assert_eq!(ann.semantic_type, "character_sprite");
    }

    #[test]
    fn annotation_serializes_to_json() {
        let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
        let json = serde_json::to_string(&ann).expect("annotation should serialize to JSON");
        assert!(json.contains("player.idle.facing_right"));
        assert!(json.contains("character_sprite"));
    }

    #[test]
    fn annotation_deserializes_from_json() {
        let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
        let json = serde_json::to_string(&ann).unwrap();
        let deserialized: AssetAnnotation =
            serde_json::from_str(&json).expect("annotation should deserialize from JSON");
        assert_eq!(ann, deserialized);
    }

    // -----------------------------------------------------------------------
    // Text renderer tests
    // -----------------------------------------------------------------------

    #[test]
    fn text_to_draw_commands_produces_nonempty_output() {
        let tr = TextRenderer::new();
        let commands = tr.text_to_draw_commands("Hello", 0.0, 0.0, 2.0, [1.0; 4]);
        assert!(
            !commands.is_empty(),
            "text_to_draw_commands for 'Hello' should produce draw commands"
        );
    }

    #[test]
    fn text_to_draw_commands_empty_string() {
        let tr = TextRenderer::new();
        let commands = tr.text_to_draw_commands("", 0.0, 0.0, 2.0, [1.0; 4]);
        assert!(
            commands.is_empty(),
            "empty string should produce no draw commands"
        );
    }

    #[test]
    fn text_to_draw_commands_unknown_chars_skipped() {
        let tr = TextRenderer::new();
        // Non-ASCII characters are not in the glyph map.
        let commands = tr.text_to_draw_commands("\u{1F600}", 0.0, 0.0, 1.0, [1.0; 4]);
        assert!(
            commands.is_empty(),
            "unknown characters should be silently skipped"
        );
    }

    #[test]
    fn text_to_draw_commands_glyph_pixel_dimensions() {
        let tr = TextRenderer::new();
        let scale = 3.0;
        let commands = tr.text_to_draw_commands("A", 0.0, 0.0, scale, [1.0; 4]);
        assert!(!commands.is_empty());

        // Every draw command pixel should be scale x scale.
        for cmd in &commands {
            assert!(
                (cmd.width - scale).abs() < f32::EPSILON,
                "pixel width should be {scale}, got {}",
                cmd.width
            );
            assert!(
                (cmd.height - scale).abs() < f32::EPSILON,
                "pixel height should be {scale}, got {}",
                cmd.height
            );
        }

        // All pixel positions should fall within the glyph bounds:
        // X: [0, 5*scale), Y: [0, 7*scale).
        // (positions are center of pixel, so within half-pixel offset)
        let half = scale / 2.0;
        for cmd in &commands {
            assert!(
                cmd.x >= half - f32::EPSILON && cmd.x < 5.0 * scale + half + f32::EPSILON,
                "pixel x={} out of glyph x-bounds [0, {})",
                cmd.x,
                5.0 * scale
            );
            assert!(
                cmd.y >= half - f32::EPSILON && cmd.y < 7.0 * scale + half + f32::EPSILON,
                "pixel y={} out of glyph y-bounds [0, {})",
                cmd.y,
                7.0 * scale
            );
        }
    }

    #[test]
    fn text_to_draw_commands_respects_color() {
        let tr = TextRenderer::new();
        let color = [0.1, 0.2, 0.3, 0.9];
        let commands = tr.text_to_draw_commands("X", 0.0, 0.0, 1.0, color);
        assert!(!commands.is_empty());
        for cmd in &commands {
            assert_eq!(cmd.color, color, "all pixels should use the given color");
        }
    }

    #[test]
    fn text_to_draw_commands_multichar_spacing() {
        let tr = TextRenderer::new();
        let scale = 2.0;
        // Render "AB" -- second character should start 6*scale pixels to the right.
        let commands_a = tr.text_to_draw_commands("A", 0.0, 0.0, scale, [1.0; 4]);
        let commands_ab = tr.text_to_draw_commands("AB", 0.0, 0.0, scale, [1.0; 4]);

        // The commands for "AB" should be a superset containing "A" commands
        // plus offset "B" commands.
        assert!(
            commands_ab.len() > commands_a.len(),
            "'AB' should produce more draw commands than 'A'"
        );

        // B's pixels should start at x >= 6*scale (with some tolerance for center offset).
        let a_max_x = commands_a
            .iter()
            .map(|c| c.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let b_commands: Vec<_> = commands_ab
            .iter()
            .filter(|c| c.x > a_max_x + scale)
            .collect();
        assert!(
            !b_commands.is_empty(),
            "'B' should have pixels starting after 'A'"
        );
    }

    #[test]
    fn text_to_draw_commands_respects_position_offset() {
        let tr = TextRenderer::new();
        let offset_x = 100.0;
        let offset_y = 200.0;
        let commands = tr.text_to_draw_commands("A", offset_x, offset_y, 1.0, [1.0; 4]);
        assert!(!commands.is_empty());
        for cmd in &commands {
            assert!(
                cmd.x >= offset_x,
                "pixel x should be >= offset_x={offset_x}, got {}",
                cmd.x
            );
            assert!(
                cmd.y >= offset_y,
                "pixel y should be >= offset_y={offset_y}, got {}",
                cmd.y
            );
        }
    }

    #[test]
    fn text_width_and_height() {
        let tr = TextRenderer::new();
        let scale = 2.0;

        // Width for "Hello" (5 chars) = (5 * 6 - 1) * scale = 29 * 2 = 58
        let width = tr.text_width("Hello", scale);
        assert!(
            (width - 58.0).abs() < f32::EPSILON,
            "text_width for 'Hello' at scale 2 should be 58.0, got {width}"
        );

        // Height is always 7 * scale.
        let height = tr.text_height(scale);
        assert!(
            (height - 14.0).abs() < f32::EPSILON,
            "text_height at scale 2 should be 14.0, got {height}"
        );

        // Empty string has zero width.
        let empty_width = tr.text_width("", scale);
        assert!(
            (empty_width - 0.0).abs() < f32::EPSILON,
            "empty string width should be 0"
        );
    }

    #[test]
    fn text_renderer_all_digits_render() {
        let tr = TextRenderer::new();
        for digit in '0'..='9' {
            let text = String::from(digit);
            let commands = tr.text_to_draw_commands(&text, 0.0, 0.0, 1.0, [1.0; 4]);
            assert!(
                !commands.is_empty(),
                "digit '{digit}' should produce draw commands"
            );
        }
    }

    #[test]
    fn text_renderer_score_display() {
        let tr = TextRenderer::new();
        let commands = tr.text_to_draw_commands("Score: 42", 10.0, 10.0, 2.0, [1.0, 1.0, 0.0, 1.0]);
        assert!(
            !commands.is_empty(),
            "score display string should produce draw commands"
        );
    }
}
