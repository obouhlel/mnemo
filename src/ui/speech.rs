//! Lecture à voix haute de la note ouverte (voir `crate::tts`) : la sélection de l'éditeur, sinon toute la
//! note. La voix continue quand on change de note ou d'application ; même commande pour l'arrêter.

use super::*;

impl MnemoApp {
    pub(super) fn read_aloud(&mut self, ctx: &egui::Context, now: f64) {
        if self.speaker.speaking() {
            self.speaker.stop();
            return;
        }
        let Some(n) = &self.open else {
            self.notice("Ouvrez une note à lire", true, now);
            return;
        };
        // texte sélectionné dans l'éditeur, s'il y en a
        let selection = egui::TextEdit::load_state(ctx, editor_id(n.id))
            .and_then(|s| s.cursor.char_range())
            .filter(|r| r.primary.index != r.secondary.index && self.cfg.view_mode != ViewMode::Preview)
            .map(|r| {
                let (a, b) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
                n.content[char_to_byte(&n.content, a)..char_to_byte(&n.content, b)].to_string()
            });
        let text = crate::tts::speakable(selection.as_deref().unwrap_or(&n.content));
        if let Err(e) = self.speaker.speak(&text, self.cfg.tts_rate) {
            self.notice(e, true, now);
        }
        // l'icône « arrêter » tout de suite
        ctx.request_repaint();
    }
}
