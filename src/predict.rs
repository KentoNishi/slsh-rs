use crate::screen::{Cell, Cursor, Screen};
use crate::tmux::KeyIntent;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub enabled: bool,
    pub cells: Vec<OverlayCell>,
    pub cursor: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCell {
    pub pos: Cursor,
    pub cell: Cell,
    pub under: Cell,
}

#[derive(Debug, Clone)]
pub struct BasePredictor {
    pub overlay: Overlay,
}

impl BasePredictor {
    pub fn new(enabled: bool) -> Self {
        Self {
            overlay: Overlay {
                enabled,
                cells: Vec::new(),
                cursor: None,
            },
        }
    }

    pub fn toggle(&mut self) {
        self.overlay.enabled = !self.overlay.enabled;
        self.clear();
    }

    pub fn clear(&mut self) {
        self.overlay.cells.clear();
        self.overlay.cursor = None;
    }

    pub fn on_key(&mut self, intent: KeyIntent, screen: &Screen) {
        match intent {
            KeyIntent::Printable(ch) => self.predict_printable(ch, screen),
            KeyIntent::Backspace => self.predict_backspace(),
            KeyIntent::TogglePrediction => self.toggle(),
            KeyIntent::Nonlinear | KeyIntent::Unsupported => self.clear(),
        }
    }

    pub fn reconcile(&mut self, screen: &Screen) {
        let mut kept = Vec::new();
        for cell in &self.overlay.cells {
            let confirmed = screen.cell(cell.pos);
            if confirmed == cell.cell {
                continue;
            }
            if confirmed != cell.under {
                self.clear();
                return;
            }
            kept.push(*cell);
        }

        self.overlay.cells = kept;
        if self.overlay.cells.is_empty() {
            self.overlay.cursor = None;
        }
    }

    fn predict_printable(&mut self, ch: char, screen: &Screen) {
        if !self.overlay.enabled || hidden_input_guard(screen) {
            self.clear();
            return;
        }

        let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if width == 0 || width > 2 {
            return;
        }

        let mut cursor = self.overlay.cursor.unwrap_or_else(|| screen.cursor());
        if width == 2 && cursor.col + 1 >= screen.size().cols {
            cursor.col = 0;
            cursor.row += 1;
        }
        if cursor.row >= screen.size().rows {
            self.clear();
            return;
        }

        let under = screen.cell(cursor);
        self.overlay.cells.push(OverlayCell {
            pos: cursor,
            cell: Cell {
                ch,
                style: under.style,
            },
            under,
        });

        if cursor.col + width < screen.size().cols {
            cursor.col += width;
            self.overlay.cursor = Some(cursor);
        } else if cursor.row + 1 < screen.size().rows {
            self.overlay.cursor = Some(Cursor {
                row: cursor.row + 1,
                col: 0,
            });
        } else {
            self.clear();
        }
    }

    fn predict_backspace(&mut self) {
        if self.overlay.cells.pop().is_some() {
            self.overlay.cursor = self.overlay.cells.last().map(|cell| Cursor {
                row: cell.pos.row,
                col: cell.pos.col + 1,
            });
        } else {
            self.clear();
        }
    }
}

pub fn hidden_input_guard(screen: &Screen) -> bool {
    let tail = screen.visible_text_tail(3).to_ascii_lowercase();
    [
        "password",
        "passphrase",
        "secret",
        "token",
        "sudo",
        "login:",
    ]
    .iter()
    .any(|needle| tail.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Size;

    fn screen_with(text: &[u8]) -> Screen {
        let mut screen = Screen::new(Size { cols: 20, rows: 3 });
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, text);
        screen
    }

    #[test]
    fn predicts_printable_at_cursor() {
        let screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 2 });
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'a');
    }

    #[test]
    fn backspace_removes_overlay() {
        let screen = screen_with(b"");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn nonlinear_input_clears_overlay() {
        let screen = screen_with(b"");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Nonlinear, &screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn matching_remote_echo_confirms_overlay() {
        let mut screen = screen_with(b"");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, b"a");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn conflicting_remote_truth_clears_overlay() {
        let mut screen = screen_with(b"");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, b"b");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn unchanged_under_cell_keeps_overlay() {
        let screen = screen_with(b"");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.reconcile(&screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
    }

    #[test]
    fn hidden_input_suppresses_overlay() {
        let screen = screen_with(b"password: ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('s'), &screen);

        assert!(predictor.overlay.cells.is_empty());
    }
}
