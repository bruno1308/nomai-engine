//! Simple glyph-based text rendering for the debug renderer.
//!
//! Renders text as colored rectangles using 5x7 pixel bitmap glyphs.
//! Each "lit" pixel in a glyph becomes a [`DrawCommand`] rectangle,
//! allowing text to be rendered through the same quad pipeline as
//! entities. This avoids font atlas complexity for the MVP debug
//! renderer.
//!
//! # Usage
//!
//! ```ignore
//! use nomai_engine::render::text::TextRenderer;
//!
//! let text_renderer = TextRenderer::new();
//! let commands = text_renderer.text_to_draw_commands("Score: 42", 10.0, 10.0, 2.0, [1.0; 4]);
//! // Feed `commands` into the same render pipeline as entity DrawCommands.
//! ```

use std::collections::HashMap;

use super::renderer::DrawCommand;

/// Render text as colored rectangles using 5x7 pixel bitmap glyphs.
///
/// Each character is represented as a 5-wide by 7-tall grid of booleans.
/// A `true` value means the pixel is "lit" and should be drawn. The
/// [`text_to_draw_commands`](Self::text_to_draw_commands) method converts
/// a string into [`DrawCommand`]s -- one colored rectangle per lit pixel,
/// scaled by the given factor.
///
/// This is intentionally simple: no kerning, no anti-aliasing, no font
/// loading. It exists for debug HUD overlays (score, tick count, FPS).
pub struct TextRenderer {
    /// Map from ASCII character to its 5x7 bitmap glyph.
    ///
    /// Each glyph is `[[bool; 5]; 7]` where the outer array is rows
    /// (top to bottom) and the inner array is columns (left to right).
    glyphs: HashMap<char, [[bool; 5]; 7]>,
}

impl TextRenderer {
    /// Create a new text renderer with built-in glyphs for ASCII 0-9,
    /// A-Z, a-z, and basic punctuation.
    pub fn new() -> Self {
        let mut glyphs = HashMap::new();

        // Helper to convert a compact row representation to bool arrays.
        // Each u8 encodes 5 bits (bit 4=leftmost, bit 0=rightmost).
        fn glyph_from_rows(rows: [u8; 7]) -> [[bool; 5]; 7] {
            let mut g = [[false; 5]; 7];
            for (r, &bits) in rows.iter().enumerate() {
                for c in 0..5 {
                    g[r][c] = (bits >> (4 - c)) & 1 == 1;
                }
            }
            g
        }

        // --- Digits 0-9 ---
        glyphs.insert(
            '0',
            glyph_from_rows([
                0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            '1',
            glyph_from_rows([
                0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
            ]),
        );
        glyphs.insert(
            '2',
            glyph_from_rows([
                0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
            ]),
        );
        glyphs.insert(
            '3',
            glyph_from_rows([
                0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            '4',
            glyph_from_rows([
                0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
            ]),
        );
        glyphs.insert(
            '5',
            glyph_from_rows([
                0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            '6',
            glyph_from_rows([
                0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            '7',
            glyph_from_rows([
                0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
            ]),
        );
        glyphs.insert(
            '8',
            glyph_from_rows([
                0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            '9',
            glyph_from_rows([
                0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
            ]),
        );

        // --- Uppercase A-Z ---
        glyphs.insert(
            'A',
            glyph_from_rows([
                0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'B',
            glyph_from_rows([
                0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
            ]),
        );
        glyphs.insert(
            'C',
            glyph_from_rows([
                0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            'D',
            glyph_from_rows([
                0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100,
            ]),
        );
        glyphs.insert(
            'E',
            glyph_from_rows([
                0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
            ]),
        );
        glyphs.insert(
            'F',
            glyph_from_rows([
                0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
            ]),
        );
        glyphs.insert(
            'G',
            glyph_from_rows([
                0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
            ]),
        );
        glyphs.insert(
            'H',
            glyph_from_rows([
                0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'I',
            glyph_from_rows([
                0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
            ]),
        );
        glyphs.insert(
            'J',
            glyph_from_rows([
                0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
            ]),
        );
        glyphs.insert(
            'K',
            glyph_from_rows([
                0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
            ]),
        );
        glyphs.insert(
            'L',
            glyph_from_rows([
                0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
            ]),
        );
        glyphs.insert(
            'M',
            glyph_from_rows([
                0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'N',
            glyph_from_rows([
                0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'O',
            glyph_from_rows([
                0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            'P',
            glyph_from_rows([
                0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
            ]),
        );
        glyphs.insert(
            'Q',
            glyph_from_rows([
                0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
            ]),
        );
        glyphs.insert(
            'R',
            glyph_from_rows([
                0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
            ]),
        );
        glyphs.insert(
            'S',
            glyph_from_rows([
                0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
            ]),
        );
        glyphs.insert(
            'T',
            glyph_from_rows([
                0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
            ]),
        );
        glyphs.insert(
            'U',
            glyph_from_rows([
                0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            'V',
            glyph_from_rows([
                0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
            ]),
        );
        glyphs.insert(
            'W',
            glyph_from_rows([
                0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
            ]),
        );
        glyphs.insert(
            'X',
            glyph_from_rows([
                0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'Y',
            glyph_from_rows([
                0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
            ]),
        );
        glyphs.insert(
            'Z',
            glyph_from_rows([
                0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
            ]),
        );

        // --- Lowercase a-z ---
        glyphs.insert(
            'a',
            glyph_from_rows([
                0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111,
            ]),
        );
        glyphs.insert(
            'b',
            glyph_from_rows([
                0b10000, 0b10000, 0b10110, 0b11001, 0b10001, 0b10001, 0b11110,
            ]),
        );
        glyphs.insert(
            'c',
            glyph_from_rows([
                0b00000, 0b00000, 0b01110, 0b10000, 0b10000, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            'd',
            glyph_from_rows([
                0b00001, 0b00001, 0b01101, 0b10011, 0b10001, 0b10001, 0b01111,
            ]),
        );
        glyphs.insert(
            'e',
            glyph_from_rows([
                0b00000, 0b00000, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110,
            ]),
        );
        glyphs.insert(
            'f',
            glyph_from_rows([
                0b00110, 0b01001, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000,
            ]),
        );
        glyphs.insert(
            'g',
            glyph_from_rows([
                0b00000, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110,
            ]),
        );
        glyphs.insert(
            'h',
            glyph_from_rows([
                0b10000, 0b10000, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'i',
            glyph_from_rows([
                0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110,
            ]),
        );
        glyphs.insert(
            'j',
            glyph_from_rows([
                0b00010, 0b00000, 0b00110, 0b00010, 0b00010, 0b10010, 0b01100,
            ]),
        );
        glyphs.insert(
            'k',
            glyph_from_rows([
                0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010,
            ]),
        );
        glyphs.insert(
            'l',
            glyph_from_rows([
                0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
            ]),
        );
        glyphs.insert(
            'm',
            glyph_from_rows([
                0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'n',
            glyph_from_rows([
                0b00000, 0b00000, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001,
            ]),
        );
        glyphs.insert(
            'o',
            glyph_from_rows([
                0b00000, 0b00000, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110,
            ]),
        );
        glyphs.insert(
            'p',
            glyph_from_rows([
                0b00000, 0b00000, 0b11110, 0b10001, 0b11110, 0b10000, 0b10000,
            ]),
        );
        glyphs.insert(
            'q',
            glyph_from_rows([
                0b00000, 0b00000, 0b01101, 0b10011, 0b01111, 0b00001, 0b00001,
            ]),
        );
        glyphs.insert(
            'r',
            glyph_from_rows([
                0b00000, 0b00000, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000,
            ]),
        );
        glyphs.insert(
            's',
            glyph_from_rows([
                0b00000, 0b00000, 0b01110, 0b10000, 0b01110, 0b00001, 0b11110,
            ]),
        );
        glyphs.insert(
            't',
            glyph_from_rows([
                0b01000, 0b01000, 0b11100, 0b01000, 0b01000, 0b01001, 0b00110,
            ]),
        );
        glyphs.insert(
            'u',
            glyph_from_rows([
                0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b10011, 0b01101,
            ]),
        );
        glyphs.insert(
            'v',
            glyph_from_rows([
                0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
            ]),
        );
        glyphs.insert(
            'w',
            glyph_from_rows([
                0b00000, 0b00000, 0b10001, 0b10001, 0b10101, 0b10101, 0b01010,
            ]),
        );
        glyphs.insert(
            'x',
            glyph_from_rows([
                0b00000, 0b00000, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001,
            ]),
        );
        glyphs.insert(
            'y',
            glyph_from_rows([
                0b00000, 0b00000, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110,
            ]),
        );
        glyphs.insert(
            'z',
            glyph_from_rows([
                0b00000, 0b00000, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111,
            ]),
        );

        // --- Punctuation and symbols ---
        glyphs.insert(
            ' ',
            glyph_from_rows([
                0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000,
            ]),
        );
        glyphs.insert(
            '.',
            glyph_from_rows([
                0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
            ]),
        );
        glyphs.insert(
            ',',
            glyph_from_rows([
                0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b00100, 0b01000,
            ]),
        );
        glyphs.insert(
            ':',
            glyph_from_rows([
                0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000,
            ]),
        );
        glyphs.insert(
            ';',
            glyph_from_rows([
                0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b00100, 0b01000,
            ]),
        );
        glyphs.insert(
            '!',
            glyph_from_rows([
                0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
            ]),
        );
        glyphs.insert(
            '?',
            glyph_from_rows([
                0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
            ]),
        );
        glyphs.insert(
            '-',
            glyph_from_rows([
                0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
            ]),
        );
        glyphs.insert(
            '+',
            glyph_from_rows([
                0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
            ]),
        );
        glyphs.insert(
            '=',
            glyph_from_rows([
                0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000,
            ]),
        );
        glyphs.insert(
            '/',
            glyph_from_rows([
                0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
            ]),
        );
        glyphs.insert(
            '(',
            glyph_from_rows([
                0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
            ]),
        );
        glyphs.insert(
            ')',
            glyph_from_rows([
                0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
            ]),
        );
        glyphs.insert(
            '[',
            glyph_from_rows([
                0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
            ]),
        );
        glyphs.insert(
            ']',
            glyph_from_rows([
                0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
            ]),
        );
        glyphs.insert(
            '_',
            glyph_from_rows([
                0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
            ]),
        );
        glyphs.insert(
            '#',
            glyph_from_rows([
                0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
            ]),
        );
        glyphs.insert(
            '%',
            glyph_from_rows([
                0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011,
            ]),
        );

        Self { glyphs }
    }

    /// Convert a text string into [`DrawCommand`]s for rendering.
    ///
    /// Each lit pixel in the glyph bitmap produces one colored rectangle
    /// of size `scale x scale`. Characters are spaced 6 pixels apart
    /// horizontally (5 glyph columns + 1 gap) and the glyph height is
    /// 7 pixels. All coordinates are in world units.
    ///
    /// Characters not present in the glyph map are silently skipped.
    ///
    /// # Arguments
    ///
    /// * `text` - The string to render.
    /// * `x` - X position of the top-left corner of the first character (world units).
    /// * `y` - Y position of the top-left corner of the first character (world units).
    /// * `scale` - Size of each glyph pixel in world units.
    /// * `color` - RGBA color for all pixels.
    pub fn text_to_draw_commands(
        &self,
        text: &str,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        let half = scale / 2.0;

        for (char_idx, ch) in text.chars().enumerate() {
            let Some(glyph) = self.glyphs.get(&ch) else {
                continue;
            };

            let char_x = x + (char_idx as f32) * 6.0 * scale;

            for (row, row_data) in glyph.iter().enumerate() {
                for (col, &lit) in row_data.iter().enumerate() {
                    if lit {
                        commands.push(DrawCommand {
                            x: char_x + (col as f32) * scale + half,
                            y: y + (row as f32) * scale + half,
                            width: scale,
                            height: scale,
                            color,
                        });
                    }
                }
            }
        }

        commands
    }

    /// Return the glyph bitmap for a character, if present.
    ///
    /// Useful for testing and introspection.
    pub fn glyph(&self, ch: char) -> Option<&[[bool; 5]; 7]> {
        self.glyphs.get(&ch)
    }

    /// Width of a rendered string in world units, given the scale factor.
    ///
    /// Each character occupies 6 scaled pixels (5 glyph + 1 gap), except
    /// the last character which occupies 5 scaled pixels.
    pub fn text_width(&self, text: &str, scale: f32) -> f32 {
        let len = text.chars().count();
        if len == 0 {
            return 0.0;
        }
        // Each char is 5 pixels wide + 1 pixel gap, last char has no trailing gap.
        (len as f32 * 6.0 - 1.0) * scale
    }

    /// Height of rendered text in world units, given the scale factor.
    ///
    /// Always 7 scaled pixels (single line).
    pub fn text_height(&self, scale: f32) -> f32 {
        7.0 * scale
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_renderer_has_digits() {
        let tr = TextRenderer::new();
        for ch in '0'..='9' {
            assert!(
                tr.glyph(ch).is_some(),
                "TextRenderer should have glyph for '{ch}'"
            );
        }
    }

    #[test]
    fn text_renderer_has_uppercase() {
        let tr = TextRenderer::new();
        for ch in 'A'..='Z' {
            assert!(
                tr.glyph(ch).is_some(),
                "TextRenderer should have glyph for '{ch}'"
            );
        }
    }

    #[test]
    fn text_renderer_has_lowercase() {
        let tr = TextRenderer::new();
        for ch in 'a'..='z' {
            assert!(
                tr.glyph(ch).is_some(),
                "TextRenderer should have glyph for '{ch}'"
            );
        }
    }

    #[test]
    fn text_renderer_space_produces_no_lit_pixels() {
        let tr = TextRenderer::new();
        let commands = tr.text_to_draw_commands(" ", 0.0, 0.0, 1.0, [1.0; 4]);
        assert!(
            commands.is_empty(),
            "space should produce no draw commands (no lit pixels)"
        );
    }

    #[test]
    fn text_renderer_produces_commands_for_known_chars() {
        let tr = TextRenderer::new();
        let commands = tr.text_to_draw_commands("A", 0.0, 0.0, 1.0, [1.0; 4]);
        assert!(
            !commands.is_empty(),
            "'A' should produce at least one draw command"
        );
    }

    #[test]
    fn text_renderer_glyph_dimensions() {
        let tr = TextRenderer::new();
        for ch in 'A'..='Z' {
            let glyph = tr.glyph(ch).unwrap();
            assert_eq!(glyph.len(), 7, "glyph for '{ch}' should have 7 rows");
            for row in glyph {
                assert_eq!(row.len(), 5, "glyph row for '{ch}' should have 5 columns");
            }
        }
    }
}
