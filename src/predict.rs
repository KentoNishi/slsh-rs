use crate::key::KeyIntent;
use crate::screen::{Cell, Cursor, Screen};
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
    owned: Vec<OwnedCell>,
    edit_anchor: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OwnedCell {
    pos: Cursor,
    cell: Cell,
}

impl BasePredictor {
    pub fn new(enabled: bool) -> Self {
        Self {
            overlay: Overlay {
                enabled,
                cells: Vec::new(),
                cursor: None,
            },
            owned: Vec::new(),
            edit_anchor: None,
        }
    }

    pub fn toggle(&mut self) {
        self.overlay.enabled = !self.overlay.enabled;
        self.clear();
    }

    pub fn clear(&mut self) {
        self.overlay.cells.clear();
        self.overlay.cursor = None;
        self.owned.clear();
        self.edit_anchor = None;
    }

    pub fn on_key(&mut self, intent: KeyIntent, screen: &Screen) {
        match intent {
            KeyIntent::Printable(ch) => self.predict_printable(ch, screen),
            KeyIntent::Backspace => self.predict_backspace(screen),
            KeyIntent::TogglePrediction => self.toggle(),
            KeyIntent::Nonlinear | KeyIntent::Unsupported => self.clear(),
        }
    }

    pub fn reconcile(&mut self, screen: &Screen) {
        let mut kept = Vec::new();
        for cell in &self.overlay.cells {
            let confirmed = screen.cell(cell.pos);
            if confirmed == cell.cell || confirmed_matches_prediction(confirmed, cell.cell) {
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
        self.validate_owned_span(screen);
        self.validate_remote_cursor(screen);
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

        if self.owned.is_empty() && self.overlay.cells.is_empty() {
            self.edit_anchor = Some(screen.cursor());
        } else if self.overlay.cells.is_empty() && self.expected_cursor(screen) != screen.cursor() {
            self.clear();
            self.edit_anchor = Some(screen.cursor());
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
        let cell = Cell {
            ch,
            style: predicted_style(under.style),
        };
        self.overlay.cells.push(OverlayCell {
            pos: cursor,
            cell,
            under,
        });
        self.owned.retain(|owned| owned.pos != cursor);
        self.owned.push(OwnedCell { pos: cursor, cell });

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

    fn predict_backspace(&mut self, screen: &Screen) {
        if !self.overlay.enabled || hidden_input_guard(screen) {
            self.clear();
            return;
        }

        let Some(target) = self.backspace_target(screen) else {
            self.clear();
            return;
        };

        if let Some(index) = self
            .overlay
            .cells
            .iter()
            .rposition(|cell| cell.pos == target)
        {
            self.overlay.cells.remove(index);
            self.remove_owned(target);
            self.overlay.cursor = Some(target);
            return;
        }

        if !self.remove_owned(target) {
            self.clear();
            return;
        }

        let under = screen.cell(target);
        if under.ch != ' ' {
            self.overlay.cells.push(OverlayCell {
                pos: target,
                cell: Cell {
                    ch: ' ',
                    style: predicted_style(under.style),
                },
                under,
            });
        }
        self.overlay.cursor = Some(target);
    }

    fn backspace_target(&self, screen: &Screen) -> Option<Cursor> {
        let cursor = self.overlay.cursor.unwrap_or_else(|| screen.cursor());
        (cursor.col > 0).then_some(Cursor {
            row: cursor.row,
            col: cursor.col - 1,
        })
    }

    fn remove_owned(&mut self, target: Cursor) -> bool {
        let Some(index) = self.owned.iter().rposition(|owned| owned.pos == target) else {
            return false;
        };
        self.owned.remove(index);
        if self.owned.is_empty() && self.overlay.cells.is_empty() {
            self.edit_anchor = None;
        }
        true
    }

    fn validate_owned_span(&mut self, screen: &Screen) {
        for owned in &self.owned {
            if self
                .overlay
                .cells
                .iter()
                .any(|cell| cell.pos == owned.pos && cell.cell == owned.cell)
            {
                continue;
            }
            let confirmed = screen.cell(owned.pos);
            if confirmed != owned.cell && !confirmed_matches_prediction(confirmed, owned.cell) {
                self.clear();
                return;
            }
        }

        if self.owned.is_empty() && self.overlay.cells.is_empty() {
            self.edit_anchor = None;
            return;
        }

        if self.overlay.cells.is_empty()
            && !self.owned.is_empty()
            && self.expected_cursor(screen) != screen.cursor()
        {
            self.clear();
        }
    }

    fn validate_remote_cursor(&mut self, screen: &Screen) {
        if self.overlay.cells.is_empty() {
            return;
        }

        let cursor = screen.cursor();
        let at_overlay_cursor = self.overlay.cursor == Some(cursor);
        let at_pending_cell = self.overlay.cells.iter().any(|cell| cell.pos == cursor);
        if !at_overlay_cursor && !at_pending_cell {
            self.clear();
        }
    }

    fn expected_cursor(&self, screen: &Screen) -> Cursor {
        self.owned
            .last()
            .map(|owned| advance_cursor(screen, owned.pos, width_of(owned.cell.ch)))
            .or(self.edit_anchor)
            .unwrap_or_else(|| screen.cursor())
    }
}

fn advance_cursor(screen: &Screen, cursor: Cursor, width: u16) -> Cursor {
    if cursor.col + width < screen.size().cols {
        Cursor {
            row: cursor.row,
            col: cursor.col + width,
        }
    } else {
        Cursor {
            row: (cursor.row + 1).min(screen.size().rows.saturating_sub(1)),
            col: 0,
        }
    }
}

fn width_of(ch: char) -> u16 {
    UnicodeWidthChar::width(ch).unwrap_or(0) as u16
}

fn predicted_style(mut style: crate::screen::Style) -> crate::screen::Style {
    style.dim = true;
    style
}

fn confirmed_matches_prediction(confirmed: Cell, predicted: Cell) -> bool {
    let mut predicted_confirmed_style = predicted.style;
    predicted_confirmed_style.dim = false;
    confirmed.ch == predicted.ch && confirmed.style == predicted_confirmed_style
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

    fn feed(screen: &mut Screen, text: &[u8]) {
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, text);
    }

    #[test]
    fn predicts_printable_at_cursor() {
        let screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 2 });
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'a');
        assert!(predictor.overlay.cells[0].cell.style.dim);
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
    fn backspace_does_not_hide_prompt_cell() {
        let screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Backspace, &screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn backspace_hides_confirmed_owned_cell() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        feed(&mut screen, b"a");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 2 });
        assert_eq!(predictor.overlay.cells[0].cell.ch, ' ');
        assert_eq!(predictor.overlay.cells[0].under.ch, 'a');
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 2 }));
    }

    #[test]
    fn repeated_backspace_hides_confirmed_owned_cells_in_order() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in ['a', 'b', 'c'] {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed(&mut screen, b"abc");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.pos.col)
                .collect::<Vec<_>>(),
            vec![4, 3, 2]
        );
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 2 }));
    }

    #[test]
    fn confirmed_remote_backspace_removes_deletion_overlay() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        feed(&mut screen, b"a");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        feed(&mut screen, b"\x08 \x08");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn nonlinear_input_clears_owned_cells_before_backspace() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        feed(&mut screen, b"a");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Nonlinear, &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn remote_cursor_jump_clears_owned_cells_before_backspace() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        feed(&mut screen, b"a\r\n$ ");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
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
    fn remote_cursor_jump_clears_unconfirmed_overlay() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('d'), &screen);
        feed(&mut screen, b"\x1b[2;1H");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn partial_remote_echo_keeps_remaining_overlay() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);
        feed(&mut screen, b"a");
        predictor.reconcile(&screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'b');
    }

    #[test]
    fn hidden_input_suppresses_overlay() {
        let screen = screen_with(b"password: ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('s'), &screen);

        assert!(predictor.overlay.cells.is_empty());
    }
}
