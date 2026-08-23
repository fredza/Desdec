//! The typefaces the interface is drawn with.
//!
//! egui's proportional family is Ubuntu-Light alone, backed by two emoji
//! fonts. Ubuntu-Light stops just past Latin: it has no arrows. Every `↑`,
//! `↓`, `→` and `←` in a help line therefore reached the window as `◻`, the
//! replacement glyph — the disassembly's "the ↑ and ↓ keys step from one
//! instruction to the next" told the reader to press two empty squares.
//!
//! Hack is already shipped for the monospace family and has all four, so it
//! goes on the proportional family as a fallback. No new asset, no larger
//! binary, and Ubuntu-Light still draws everything it can: the fallback is
//! consulted for a code point only when the fonts before it lack it.

use eframe::egui;

/// Name of the monospace font egui ships, as [`egui::FontDefinitions`] keys
/// it. Borrowed rather than re-embedded.
const FALLBACK: &str = "Hack";

/// Gives the context the interface's fonts. Call once, before the first
/// frame; calling it again is harmless but rebuilds the atlas.
pub fn install(ctx: &egui::Context) {
    let mut definitions = egui::FontDefinitions::default();
    if let Some(family) = definitions
        .families
        .get_mut(&egui::FontFamily::Proportional)
    {
        // Directly after Ubuntu-Light, ahead of the emoji fonts: an arrow in
        // a sentence is punctuation, and a text face draws it as such. Behind
        // the emoji fonts it would come out as a pictograph wherever they
        // happen to carry the code point.
        family.insert(1, FALLBACK.to_owned());
    }
    ctx.set_fonts(definitions);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::i18n::{ALL_TEXT, Language, text};

    /// Every translated string, in every language, drawn with the fonts the
    /// application actually installs.
    ///
    /// A missing glyph is not a crash and not a failed assertion about the
    /// text — the string is present in the shapes, letter for letter. It is
    /// only wrong on screen, which is the one place no test was looking.
    #[test]
    fn every_translation_has_its_glyphs() {
        let ctx = egui::Context::default();
        install(&ctx);
        // The atlas is built on the first frame; asking before that answers
        // from an empty font set.
        let _ = ctx.run(crate::testing::window_input(), |_| {});

        let font = egui::FontId::proportional(14.0);
        let mut missing: Vec<String> = Vec::new();
        ctx.fonts(|fonts| {
            for item in ALL_TEXT {
                for language in Language::ALL {
                    let string = text(*language, *item);
                    for character in string.chars() {
                        if !fonts.has_glyph(&font, character) {
                            missing.push(format!("{item:?} ({language:?}): {character:?}"));
                        }
                    }
                }
            }
        });
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "the proportional family cannot draw: {}",
            missing.join(", ")
        );
    }

    /// The fallback is the reason the arrows draw at all: without it the same
    /// four code points are missing, which is what this guards against being
    /// quietly reintroduced by a font change upstream.
    #[test]
    fn the_arrows_need_the_fallback() {
        let ctx = egui::Context::default();
        let _ = ctx.run(crate::testing::window_input(), |_| {});
        let font = egui::FontId::proportional(14.0);
        let bare: Vec<char> = ctx.fonts(|fonts| {
            "←↑→↓"
                .chars()
                .filter(|character| !fonts.has_glyph(&font, *character))
                .collect()
        });
        assert_eq!(
            bare,
            vec!['←', '↑', '→', '↓'],
            "egui's own proportional family gained the arrows: the fallback \
             may no longer be needed, but check before removing it"
        );
    }
}
