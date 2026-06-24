use crate::key::KeyIntent;
use crate::predict::{Overlay, OverlayCell, OverlayKind, PredictorPlugin};
use crate::screen::{ActiveBuffer, Cell, Color, Cursor, Screen};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone)]
pub struct BasePredictor {
    pub overlay: Overlay,
    owned: Vec<OwnedCell>,
    edit_anchor: Option<Cursor>,
    submitted: bool,
    nonlinear_block: Option<NonlinearBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OwnedCell {
    pos: Cursor,
    cell: Cell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NonlinearBlock {
    unsettled: Vec<OwnedCell>,
    expected_cursor: Option<Cursor>,
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
            submitted: false,
            nonlinear_block: None,
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
        self.submitted = false;
        self.nonlinear_block = None;
    }

    pub fn on_key(&mut self, intent: KeyIntent, screen: &Screen) {
        match intent {
            KeyIntent::Printable(ch) => self.predict_printable(ch, screen),
            KeyIntent::Backspace => self.predict_backspace(screen),
            KeyIntent::Submit => self.submit(screen),
            KeyIntent::TogglePrediction => self.toggle(),
            KeyIntent::Nonlinear | KeyIntent::Unsupported => self.block_until_remote_output(screen),
        }
    }

    pub fn reconcile(&mut self, screen: &Screen) {
        self.update_nonlinear_block(screen);
        let mut kept = Vec::new();
        let mut conflict = None;

        for (index, cell) in self.overlay.cells.iter().copied().enumerate() {
            let confirmed = screen.cell(cell.pos);
            if is_deletion_prediction(cell) {
                if let Some(cell) = reconcile_deletion_prediction(cell, confirmed, screen) {
                    kept.push(cell);
                } else if confirmed_matches_prior_same_position(cell.pos, confirmed, &kept) {
                    kept.push(cell);
                }
                continue;
            }
            if printable_prediction_confirmed(cell, confirmed, screen, !kept.is_empty()) {
                continue;
            }
            if confirmed != cell.under {
                if confirmed_matches_prior_same_position(cell.pos, confirmed, &kept) {
                    kept.push(cell);
                    continue;
                } else {
                    conflict = Some(index);
                    break;
                }
            }
            kept.push(cell);
        }

        if let Some(index) = conflict {
            let suffix = self.pending_suffix_after_conflict(index, screen);
            if suffix.is_empty()
                || !remote_cursor_accepts_suffix(screen, &suffix, self.overlay.cursor)
            {
                self.clear();
                return;
            }
            self.overlay.cells = suffix;
            self.retain_owned_overlay_cells();
            self.edit_anchor = self.overlay.cells.first().map(|cell| cell.pos);
        } else {
            self.overlay.cells = kept;
        }
        if self.overlay.cells.is_empty() {
            self.overlay.cursor = None;
        }
        self.validate_owned_span(screen);
        self.validate_remote_cursor(screen);
    }

    fn predict_printable(&mut self, ch: char, screen: &Screen) {
        if self.submitted || self.nonlinear_block.is_some() {
            return;
        }
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
            kind: OverlayKind::Printable,
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
        if self.submitted || self.nonlinear_block.is_some() {
            return;
        }
        if !self.overlay.enabled || hidden_input_guard(screen) {
            self.clear();
            return;
        }

        let Some(target) = self.backspace_target(screen) else {
            if self.overlay.cells.is_empty() {
                self.clear();
            }
            return;
        };

        if let Some(index) = self
            .overlay
            .cells
            .iter()
            .rposition(|cell| cell.pos == target)
        {
            let mut cell = self.overlay.cells[index];
            if is_deletion_prediction(cell) {
                self.overlay.cells.remove(index);
            } else {
                cell.cell.style = predicted_deletion_style(cell.cell.style);
                cell.kind = OverlayKind::Deletion { remote_seen: false };
                self.overlay.cells[index] = cell;
            }
            self.remove_owned(target);
            self.overlay.cursor = Some(target);
            return;
        }

        let under = screen.cell(target);
        if !self.remove_owned(target) {
            let Some(start) = confirmed_deletion_start(target, screen) else {
                if self.overlay.cells.is_empty() {
                    self.clear();
                }
                return;
            };
            self.edit_anchor.get_or_insert(start);
        }

        if under.ch != ' ' {
            self.overlay.cells.push(OverlayCell {
                pos: target,
                cell: Cell {
                    ch: under.ch,
                    style: predicted_deletion_style(under.style),
                },
                under,
                kind: OverlayKind::Deletion { remote_seen: true },
            });
        }
        self.overlay.cursor = Some(target);
    }

    fn submit(&mut self, screen: &Screen) {
        if !command_submit_context(screen) {
            self.block_until_remote_output(screen);
            return;
        }
        if !self.overlay.cells.is_empty() || !self.owned.is_empty() {
            self.submitted = true;
        }
    }

    fn block_until_remote_output(&mut self, screen: &Screen) {
        let unsettled = if command_submit_context(screen) {
            self.owned
                .iter()
                .filter(|owned| !owned_confirmed(**owned, screen))
                .copied()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let expected_cursor = (!unsettled.is_empty()).then(|| self.expected_cursor(screen));
        let block = NonlinearBlock {
            unsettled,
            expected_cursor,
        };
        self.clear();
        self.nonlinear_block = Some(block);
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

    fn pending_suffix_after_conflict(&self, conflict: usize, screen: &Screen) -> Vec<OverlayCell> {
        let mut suffix = Vec::new();
        for cell in self.overlay.cells.iter().skip(conflict + 1) {
            if !is_printable_prediction(*cell) || screen.cell(cell.pos) != cell.under {
                return Vec::new();
            }
            suffix.push(*cell);
        }
        suffix
    }

    fn retain_owned_overlay_cells(&mut self) {
        self.owned.retain(|owned| {
            self.overlay.cells.iter().any(|cell| {
                cell.kind == OverlayKind::Printable
                    && cell.pos == owned.pos
                    && cell.cell == owned.cell
            })
        });
    }

    fn validate_owned_span(&mut self, screen: &Screen) {
        for owned in &self.owned {
            if self.overlay.cells.iter().any(|cell| {
                cell.kind == OverlayKind::Printable
                    && cell.pos == owned.pos
                    && cell.cell == owned.cell
            }) {
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
        let still_redrawing_overlay_rows = cursor_on_overlay_rows(cursor, &self.overlay.cells);
        let pending_wrap_before_overlay =
            cursor_before_wrapped_overlay_row(cursor, &self.overlay.cells, screen);
        if !at_overlay_cursor
            && !at_pending_cell
            && !still_redrawing_overlay_rows
            && !pending_wrap_before_overlay
        {
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

    fn update_nonlinear_block(&mut self, screen: &Screen) {
        if self
            .nonlinear_block
            .as_ref()
            .is_some_and(|block| nonlinear_block_released(block, screen))
        {
            self.nonlinear_block = None;
        }
    }
}

impl PredictorPlugin for BasePredictor {
    fn name(&self) -> &'static str {
        "base"
    }

    fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    fn on_key(&mut self, intent: KeyIntent, screen: &Screen) {
        BasePredictor::on_key(self, intent, screen);
    }

    fn reconcile(&mut self, screen: &Screen) {
        BasePredictor::reconcile(self, screen);
    }

    fn clear(&mut self) {
        BasePredictor::clear(self);
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

fn predicted_deletion_style(mut style: crate::screen::Style) -> crate::screen::Style {
    style.fg = Color::Indexed(8);
    style.dim = true;
    style.strikethrough = true;
    style
}

fn confirmed_matches_prediction(confirmed: Cell, predicted: Cell) -> bool {
    let mut predicted_confirmed_style = predicted.style;
    predicted_confirmed_style.dim = false;
    confirmed.ch == predicted.ch && confirmed.style == predicted_confirmed_style
}

fn owned_confirmed(owned: OwnedCell, screen: &Screen) -> bool {
    let confirmed = screen.cell(owned.pos);
    confirmed == owned.cell || confirmed_matches_prediction(confirmed, owned.cell)
}

fn nonlinear_block_released(block: &NonlinearBlock, screen: &Screen) -> bool {
    if !block
        .unsettled
        .iter()
        .all(|owned| owned_confirmed(*owned, screen))
    {
        return false;
    }

    block
        .expected_cursor
        .is_none_or(|expected_cursor| screen.cursor() != expected_cursor)
}

fn printable_prediction_confirmed(
    cell: OverlayCell,
    confirmed: Cell,
    screen: &Screen,
    prior_pending: bool,
) -> bool {
    if !is_printable_prediction(cell) {
        return false;
    }
    if confirmed != cell.cell && !confirmed_matches_prediction(confirmed, cell.cell) {
        return false;
    }
    if cell.cell.ch == ' ' && confirmed == cell.under {
        if prior_pending {
            return false;
        }
        return cursor_reached(
            screen.cursor(),
            advance_cursor(screen, cell.pos, width_of(cell.cell.ch)),
            screen,
        );
    }
    true
}

fn reconcile_deletion_prediction(
    cell: OverlayCell,
    confirmed: Cell,
    screen: &Screen,
) -> Option<OverlayCell> {
    let OverlayKind::Deletion { remote_seen } = cell.kind else {
        return None;
    };
    if cell.cell.ch == ' ' && confirmed == cell.under {
        let remote_echoed_space = cursor_reached(
            screen.cursor(),
            advance_cursor(screen, cell.pos, width_of(cell.cell.ch)),
            screen,
        );
        if remote_seen && !remote_echoed_space {
            return None;
        }
        return Some(OverlayCell {
            kind: OverlayKind::Deletion {
                remote_seen: remote_seen || remote_echoed_space,
            },
            ..cell
        });
    }
    if confirmed.ch == cell.cell.ch {
        return Some(OverlayCell {
            kind: OverlayKind::Deletion { remote_seen: true },
            ..cell
        });
    }
    if !remote_seen && confirmed == cell.under {
        return Some(cell);
    }
    None
}

fn confirmed_matches_prior_same_position(
    pos: Cursor,
    confirmed: Cell,
    prior: &[OverlayCell],
) -> bool {
    prior.iter().any(|cell| {
        cell.pos == pos
            && matches!(cell.kind, OverlayKind::Deletion { remote_seen: true })
            && confirmed.ch == cell.cell.ch
    })
}

fn is_printable_prediction(cell: OverlayCell) -> bool {
    cell.kind == OverlayKind::Printable
}

fn is_deletion_prediction(cell: OverlayCell) -> bool {
    matches!(cell.kind, OverlayKind::Deletion { .. })
}

fn remote_cursor_accepts_suffix(
    screen: &Screen,
    suffix: &[OverlayCell],
    overlay_cursor: Option<Cursor>,
) -> bool {
    let cursor = screen.cursor();
    overlay_cursor == Some(cursor)
        || suffix.iter().any(|cell| cell.pos == cursor)
        || cursor_on_overlay_rows(cursor, suffix)
        || cursor_before_wrapped_overlay_row(cursor, suffix, screen)
}

fn cursor_on_overlay_rows(cursor: Cursor, cells: &[OverlayCell]) -> bool {
    let (Some(first), Some(last)) = (cells.first(), cells.last()) else {
        return false;
    };
    (first.pos.row..=last.pos.row).contains(&cursor.row)
}

fn cursor_before_wrapped_overlay_row(
    cursor: Cursor,
    cells: &[OverlayCell],
    screen: &Screen,
) -> bool {
    let Some(first) = cells.first() else {
        return false;
    };
    first.pos.col == 0
        && first.pos.row == cursor.row.saturating_add(1)
        && cursor.col + 1 == screen.size().cols
}

fn cursor_reached(cursor: Cursor, target: Cursor, screen: &Screen) -> bool {
    cursor_index(cursor, screen) >= cursor_index(target, screen)
}

fn cursor_index(cursor: Cursor, screen: &Screen) -> u32 {
    cursor.row as u32 * screen.size().cols as u32 + cursor.col as u32
}

pub fn hidden_input_guard(screen: &Screen) -> bool {
    let line = current_line_before_cursor(screen).to_ascii_lowercase();
    ["password", "passphrase", "secret", "token"]
        .iter()
        .any(|needle| line.contains(needle))
        || line.trim_start().starts_with("login:")
}

fn current_line_before_cursor(screen: &Screen) -> String {
    let cursor = screen.cursor();
    let mut line = String::new();
    for col in 0..cursor.col {
        line.push(
            screen
                .cell(Cursor {
                    row: cursor.row,
                    col,
                })
                .ch,
        );
    }
    line
}

fn command_submit_context(screen: &Screen) -> bool {
    screen.active() == ActiveBuffer::Primary
        && !screen.application_cursor_keys()
        && !screen.content_below_cursor()
}

fn confirmed_deletion_start(target: Cursor, screen: &Screen) -> Option<Cursor> {
    let start = command_start_before_cursor(screen)?;
    (cursor_index(target, screen) >= cursor_index(start, screen)).then_some(start)
}

fn command_start_before_cursor(screen: &Screen) -> Option<Cursor> {
    if !command_submit_context(screen) {
        return None;
    }

    let cursor = screen.cursor();
    let mut prompt_end = None;
    for col in 0..cursor.col {
        let ch = screen.cell(Cursor {
            row: cursor.row,
            col,
        });
        if matches!(ch.ch, '$' | '#' | '>' | '%') {
            prompt_end = Some(col.saturating_add(1));
        }
    }

    let mut col = prompt_end?;
    while col < cursor.col
        && screen
            .cell(Cursor {
                row: cursor.row,
                col,
            })
            .ch
            == ' '
    {
        col += 1;
    }
    Some(Cursor {
        row: cursor.row,
        col,
    })
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

    fn screen_with_size(size: Size, text: &[u8]) -> Screen {
        let mut screen = Screen::new(size);
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, text);
        screen
    }

    fn feed(screen: &mut Screen, text: &[u8]) {
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, text);
    }

    fn feed_each_reconcile(screen: &mut Screen, predictor: &mut BasePredictor, text: &[u8]) {
        let mut parser = vte::Parser::new();
        for byte in text {
            screen.feed(&mut parser, std::slice::from_ref(byte));
            predictor.reconcile(screen);
        }
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
    fn last_login_above_prompt_does_not_suppress_prediction() {
        let screen = screen_with_size(
            Size { cols: 60, rows: 4 },
            b"Last login: Tue Jun 23 01:45:42 2026\r\n$ ",
        );
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('e'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'e');
    }

    #[test]
    fn active_login_prompt_suppresses_prediction() {
        let screen = screen_with(b"login: ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('s'), &screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn active_password_prompt_suppresses_prediction() {
        let screen = screen_with(b"Password: ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('s'), &screen);

        assert!(predictor.overlay.cells.is_empty());
    }

    #[test]
    fn backspace_marks_unconfirmed_overlay_for_deletion() {
        let screen = screen_with(b"");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'a');
        assert_eq!(
            predictor.overlay.cells[0].kind,
            OverlayKind::Deletion { remote_seen: false }
        );
        assert_eq!(predictor.overlay.cells[0].cell.style.fg, Color::Indexed(8));
        assert!(predictor.overlay.cells[0].cell.style.dim);
        assert!(predictor.overlay.cells[0].cell.style.strikethrough);
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 0 }));
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
    fn backspace_does_not_hide_nonspace_prompt_cell() {
        let screen = screen_with(b"$");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Backspace, &screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn backspace_marks_unowned_confirmed_command_text_for_deletion() {
        let screen = screen_with(b"$ echo");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 5 });
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'o');
        assert_eq!(
            predictor.overlay.cells[0].kind,
            OverlayKind::Deletion { remote_seen: true }
        );
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 5 }));
    }

    #[test]
    fn repeated_backspace_marks_unowned_confirmed_command_text_in_order() {
        let screen = screen_with(b"$ abc");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| (cell.pos.col, cell.cell.ch))
                .collect::<Vec<_>>(),
            vec![(4, 'c'), (3, 'b')]
        );
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 3 }));
    }

    #[test]
    fn backspace_moves_early_over_unowned_command_space() {
        let screen = screen_with(b"$ a b");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'b');
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 3 }));
    }

    #[test]
    fn unowned_confirmed_deletion_clears_after_remote_erase() {
        let mut screen = screen_with(b"$ abc");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Backspace, &screen);
        feed(&mut screen, b"\x08 \x08");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn backspace_marks_confirmed_owned_cell_for_deletion() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        feed(&mut screen, b"a");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 2 });
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'a');
        assert_eq!(predictor.overlay.cells[0].cell.style.fg, Color::Indexed(8));
        assert!(predictor.overlay.cells[0].cell.style.dim);
        assert!(predictor.overlay.cells[0].cell.style.strikethrough);
        assert_eq!(predictor.overlay.cells[0].under.ch, 'a');
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 2 }));
    }

    #[test]
    fn repeated_backspace_marks_confirmed_owned_cells_in_order() {
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
        assert!(predictor
            .overlay
            .cells
            .iter()
            .all(|cell| cell.cell.style.strikethrough));
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 2 }));
    }

    #[test]
    fn extra_backspace_at_prompt_keeps_pending_deletions() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in ['a', 'b', 'c'] {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed(&mut screen, b"abc");
        predictor.reconcile(&screen);
        for _ in 0..3 {
            predictor.on_key(KeyIntent::Backspace, &screen);
        }

        let before = predictor.overlay.clone();
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay, before);
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 2 }));
    }

    #[test]
    fn extra_backspace_at_start_keeps_unconfirmed_deletions() {
        let screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in ['a', 'b', 'c'] {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        for _ in 0..3 {
            predictor.on_key(KeyIntent::Backspace, &screen);
        }

        let before = predictor.overlay.clone();
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(predictor.overlay, before);
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
    fn unconfirmed_backspace_marker_survives_echo_then_clears_on_delete() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.reconcile(&screen);

        assert_eq!(
            predictor.overlay.cells[0].kind,
            OverlayKind::Deletion { remote_seen: false }
        );

        feed(&mut screen, b"a");
        predictor.reconcile(&screen);

        assert_eq!(
            predictor.overlay.cells[0].kind,
            OverlayKind::Deletion { remote_seen: true }
        );

        feed(&mut screen, b"\x08 \x08");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn replace_after_unconfirmed_backspace_survives_remote_catchup() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "ab"
        );
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 3 }));

        feed(&mut screen, b"a");
        predictor.reconcile(&screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "ab"
        );

        feed(&mut screen, b"\x08 \x08");
        predictor.reconcile(&screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'b');

        feed(&mut screen, b"b");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn space_waits_for_remote_cursor_progress() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable(' '), &screen);
        predictor.reconcile(&screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, ' ');

        feed(&mut screen, b" ");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn partial_echo_before_space_keeps_space_prediction() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "a b".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }

        feed(&mut screen, b"a");
        predictor.reconcile(&screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            " b"
        );
    }

    #[test]
    fn backspaced_blank_space_clears_after_remote_cursor_returns() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable(' '), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.reconcile(&screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(
            predictor.overlay.cells[0].kind,
            OverlayKind::Deletion { remote_seen: false }
        );

        feed_each_reconcile(&mut screen, &mut predictor, b" \x08");

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn previous_echo_does_not_clear_backspaced_blank_space() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Printable(' '), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        feed_each_reconcile(&mut screen, &mut predictor, b"a");

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            " "
        );
        assert_eq!(
            predictor.overlay.cells[0].kind,
            OverlayKind::Deletion { remote_seen: false }
        );

        feed_each_reconcile(&mut screen, &mut predictor, b" \x08");

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn repeated_replace_after_backspace_survives_remote_catchup() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);
        predictor.on_key(KeyIntent::Printable('c'), &screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "abc"
        );

        feed(&mut screen, b"a");
        predictor.reconcile(&screen);
        assert_eq!(predictor.overlay.cells.len(), 3);

        feed(&mut screen, b"\x08 \x08b");
        predictor.reconcile(&screen);
        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "bc"
        );

        feed(&mut screen, b"\x08 \x08c");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn typo_correction_survives_remote_pending_wrap_at_last_kept_char() {
        let mut screen = screen_with_size(Size { cols: 7, rows: 3 }, b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "abc nyow".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        for _ in 0..3 {
            predictor.on_key(KeyIntent::Backspace, &screen);
        }
        for ch in "ow more".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }

        feed_each_reconcile(&mut screen, &mut predictor, b"abc n");

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .filter(|cell| cell.kind == OverlayKind::Printable)
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "ow more"
        );
        assert!(!predictor.overlay.cells.is_empty());
    }

    #[test]
    fn long_divergent_edit_survives_remote_catchup() {
        let mut screen = screen_with_size(Size { cols: 80, rows: 5 }, b"$ ");
        let mut predictor = BasePredictor::new(true);
        let typed = "test phrase typing another typo word nyow";

        for ch in typed.chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        for _ in 0..3 {
            predictor.on_key(KeyIntent::Backspace, &screen);
        }
        for ch in "ow and keep going".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }

        feed_each_reconcile(&mut screen, &mut predictor, typed.as_bytes());

        assert!(!predictor.overlay.cells.is_empty());

        feed_each_reconcile(&mut screen, &mut predictor, b"\x08 \x08\x08 \x08\x08 \x08");

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .filter(|cell| cell.kind == OverlayKind::Printable)
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "ow and keep going"
        );
        assert!(!predictor.overlay.cells.is_empty());
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
    fn submit_keeps_pending_overlay_visible() {
        let screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "echo hi".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        predictor.on_key(KeyIntent::Submit, &screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "echo hi"
        );
        assert!(predictor.submitted);
    }

    #[test]
    fn submitted_overlay_does_not_extend_before_remote_catches_up() {
        let screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "echo hi".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        predictor.on_key(KeyIntent::Submit, &screen);
        predictor.on_key(KeyIntent::Printable('x'), &screen);
        predictor.on_key(KeyIntent::Backspace, &screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "echo hi"
        );
    }

    #[test]
    fn submit_in_fullscreen_app_clears_and_waits_for_remote_output() {
        let mut screen = screen_with(b"$ \x1b[3;1Hstatus\x1b[1;3H");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Submit, &screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);

        assert!(predictor.overlay.cells.is_empty());
        assert!(!predictor.submitted);

        feed(&mut screen, b"\r\n");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'b');
    }

    #[test]
    fn submit_in_alternate_screen_clears_and_waits_for_remote_output() {
        let mut screen = screen_with(b"\x1b[?1049h");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        predictor.on_key(KeyIntent::Submit, &screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);

        assert!(predictor.overlay.cells.is_empty());

        feed(&mut screen, b"\r\n");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Printable('b'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'b');
    }

    #[test]
    fn printable_after_nonlinear_waits_for_remote_output() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Nonlinear, &screen);
        predictor.on_key(KeyIntent::Printable('x'), &screen);

        assert!(predictor.overlay.cells.is_empty());

        feed(&mut screen, b"\r$ ");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Printable('x'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'x');
    }

    #[test]
    fn printable_after_cursor_motion_waits_for_pending_echo_to_settle() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "abcdef".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed_each_reconcile(&mut screen, &mut predictor, b"ab");
        predictor.on_key(KeyIntent::Nonlinear, &screen);

        predictor.on_key(KeyIntent::Printable('X'), &screen);
        assert!(predictor.overlay.cells.is_empty());

        feed_each_reconcile(&mut screen, &mut predictor, b"cdef");
        predictor.on_key(KeyIntent::Printable('X'), &screen);
        assert!(predictor.overlay.cells.is_empty());

        feed(&mut screen, b"\x1b[D");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Printable('X'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'X');
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 7 });
    }

    #[test]
    fn printable_after_cursor_motion_on_confirmed_text_waits_for_cursor_move() {
        let mut screen = screen_with(b"$ abc");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Nonlinear, &screen);
        predictor.on_key(KeyIntent::Printable('X'), &screen);

        assert!(predictor.overlay.cells.is_empty());

        feed(&mut screen, b"\x1b[D");
        predictor.reconcile(&screen);
        predictor.on_key(KeyIntent::Printable('X'), &screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'X');
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 4 });
    }

    #[test]
    fn submitted_overlay_clears_after_remote_moves_to_next_prompt() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "echo hi".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        predictor.on_key(KeyIntent::Submit, &screen);
        feed(&mut screen, b"echo hi\r\nhi\r\n$ ");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert!(!predictor.submitted);
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
    fn conflicting_prefix_keeps_linear_pending_suffix() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "abcdef".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed(&mut screen, b"abcX");
        predictor.reconcile(&screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| (cell.pos.col, cell.cell.ch))
                .collect::<Vec<_>>(),
            vec![(6, 'e'), (7, 'f')]
        );
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 8 }));

        feed(&mut screen, b"ef");
        predictor.reconcile(&screen);

        assert!(predictor.overlay.cells.is_empty());
        assert_eq!(predictor.overlay.cursor, None);
    }

    #[test]
    fn conflicting_prefix_keeps_suffix_during_same_line_redraw() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "abcdef".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed(&mut screen, b"abcX\r");
        predictor.reconcile(&screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "ef"
        );
    }

    #[test]
    fn conflicting_prefix_clears_suffix_after_remote_cursor_jump() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "abcdef".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed(&mut screen, b"abcX\r\n$ ");
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
    fn same_line_cursor_redraw_keeps_unconfirmed_overlay() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        for ch in "abcdef".chars() {
            predictor.on_key(KeyIntent::Printable(ch), &screen);
        }
        feed(&mut screen, b"\r");
        predictor.reconcile(&screen);

        assert_eq!(
            predictor
                .overlay
                .cells
                .iter()
                .map(|cell| cell.cell.ch)
                .collect::<String>(),
            "abcdef"
        );
    }

    #[test]
    fn full_repaint_burst_keeps_unconfirmed_overlay_at_returned_cursor() {
        let mut screen = screen_with(b"$ ");
        let mut predictor = BasePredictor::new(true);

        predictor.on_key(KeyIntent::Printable('a'), &screen);
        feed(&mut screen, b"\x1b[H\x1b[2J\x1b[1;1H$ \x1b[1;3H");
        predictor.reconcile(&screen);

        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'a');
        assert_eq!(predictor.overlay.cells[0].pos, Cursor { row: 0, col: 2 });
        assert_eq!(predictor.overlay.cursor, Some(Cursor { row: 0, col: 3 }));
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
