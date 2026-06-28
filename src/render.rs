use crate::predict::{Overlay, OverlayKind};
use crate::screen::{Cell, Color, Cursor, Screen, Size, Style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub size: Size,
    pub cells: Vec<Cell>,
    pub cursor: Cursor,
    pub native_cursor_visible: bool,
}

#[derive(Debug, Default)]
pub struct Renderer {
    last: Option<Frame>,
    force_full: bool,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            last: None,
            force_full: true,
        }
    }

    pub fn invalidate(&mut self) {
        self.force_full = true;
    }

    pub fn sync_to_terminal(&mut self, screen: &Screen, overlay: &Overlay) {
        self.last = Some(compose_frame(screen, overlay));
        self.force_full = false;
    }

    pub fn render(&mut self, screen: &Screen, overlay: &Overlay) -> String {
        let next = compose_frame(screen, overlay);
        let mut output =
            if self.force_full || self.last.as_ref().map(|last| last.size) != Some(next.size) {
                full_draw(&next)
            } else {
                diff_draw(self.last.as_ref().expect("last frame exists"), &next)
            };
        if !output.is_empty() {
            output.insert_str(0, "\x1b[?25l");
            set_style(&mut output, screen.style());
            if next.native_cursor_visible {
                output.push_str("\x1b[?25h");
            }
        }
        self.last = Some(next);
        self.force_full = false;
        output
    }
}

pub fn compose_frame(screen: &Screen, overlay: &Overlay) -> Frame {
    let mut cells = screen.cells().to_vec();
    for overlay_cell in &overlay.cells {
        let index = overlay_cell.pos.row as usize * screen.size().cols as usize
            + overlay_cell.pos.col as usize;
        if let Some(cell) = cells.get_mut(index) {
            *cell = overlay_cell.cell;
        }
    }
    let cursor = overlay.cursor.unwrap_or_else(|| screen.cursor());
    let native_cursor_visible = overlay.cursor.is_none();
    if !native_cursor_visible {
        let index = cursor.row as usize * screen.size().cols as usize + cursor.col as usize;
        if let Some(cell) = cells.get_mut(index) {
            if overlay.cells.iter().any(|overlay_cell| {
                overlay_cell.pos == cursor
                    && matches!(overlay_cell.kind, OverlayKind::Deletion { .. })
            }) {
                cell.style.underline = true;
            } else {
                cell.style.reverse = true;
            }
        }
    }

    Frame {
        size: screen.size(),
        cells,
        cursor,
        native_cursor_visible,
    }
}

fn full_draw(frame: &Frame) -> String {
    let mut out = String::from("\x1b[0m\x1b[2J");
    for row in 0..frame.size.rows {
        emit_changed_row(&mut out, frame, row);
    }
    move_cursor(&mut out, frame.cursor);
    out
}

fn diff_draw(last: &Frame, next: &Frame) -> String {
    if last == next {
        return String::new();
    }

    let mut out = String::new();
    for row in 0..next.size.rows {
        let mut col = 0;
        while col < next.size.cols {
            let index = row as usize * next.size.cols as usize + col as usize;
            if last.cells.get(index) == next.cells.get(index) {
                col += 1;
                continue;
            }

            let start = col;
            let style = next.cells[index].style;
            col += 1;
            while col < next.size.cols {
                let index = row as usize * next.size.cols as usize + col as usize;
                if last.cells.get(index) == next.cells.get(index)
                    || next.cells[index].style != style
                {
                    break;
                }
                col += 1;
            }
            emit_row_span(&mut out, next, row, start, col);
        }
    }
    move_cursor(&mut out, next.cursor);
    out
}

fn emit_changed_row(out: &mut String, frame: &Frame, row: u16) {
    let row_start = row as usize * frame.size.cols as usize;
    let mut row_end = frame.size.cols;
    while row_end > 0 && frame.cells[row_start + row_end as usize - 1] == Cell::default() {
        row_end -= 1;
    }

    let mut col = 0;
    while col < row_end {
        let start = col;
        let index = row as usize * frame.size.cols as usize + col as usize;
        let style = frame.cells[index].style;
        col += 1;
        while col < row_end {
            let index = row as usize * frame.size.cols as usize + col as usize;
            if frame.cells[index].style != style {
                break;
            }
            col += 1;
        }
        emit_row_span(out, frame, row, start, col);
    }
}

fn emit_row_span(out: &mut String, frame: &Frame, row: u16, start: u16, end: u16) {
    if start == end {
        return;
    }

    if end == frame.size.cols {
        let suffix = default_suffix_start(frame, row, start, end);
        if suffix < end {
            emit_row_span(out, frame, row, start, suffix);
            move_cursor(out, Cursor { row, col: suffix });
            set_style(out, Style::default());
            out.push_str("\x1b[K");
            return;
        }
    }

    let first = row as usize * frame.size.cols as usize + start as usize;
    move_cursor(out, Cursor { row, col: start });
    set_style(out, frame.cells[first].style);
    for col in start..end {
        let index = row as usize * frame.size.cols as usize + col as usize;
        push_cell_text(out, frame.cells[index]);
    }
}

fn push_cell_text(out: &mut String, cell: Cell) {
    if cell.style.synthetic_strike {
        out.push(if cell.ch == ' ' { '·' } else { cell.ch });
        out.push('\u{0336}');
    } else {
        out.push(cell.ch);
    }
}

fn default_suffix_start(frame: &Frame, row: u16, start: u16, end: u16) -> u16 {
    let mut suffix = end;
    while suffix > start {
        let index = row as usize * frame.size.cols as usize + suffix as usize - 1;
        if frame.cells[index] != Cell::default() {
            break;
        }
        suffix -= 1;
    }
    suffix
}

fn move_cursor(out: &mut String, cursor: Cursor) {
    out.push_str(&format!("\x1b[{};{}H", cursor.row + 1, cursor.col + 1));
}

fn set_style(out: &mut String, style: Style) {
    out.push_str("\x1b[0m");
    if style.dim {
        out.push_str("\x1b[2m");
    }
    if style.bold {
        out.push_str("\x1b[1m");
    }
    if style.underline {
        out.push_str("\x1b[4m");
    }
    if style.strikethrough {
        out.push_str("\x1b[9m");
    }
    if style.reverse {
        out.push_str("\x1b[7m");
    }
    match style.fg {
        Color::Default => {}
        Color::Indexed(index) if index < 8 => out.push_str(&format!("\x1b[{}m", 30 + index)),
        Color::Indexed(index) if index < 16 => {
            out.push_str(&format!("\x1b[{}m", 90 + index.saturating_sub(8)));
        }
        Color::Indexed(index) => out.push_str(&format!("\x1b[38;5;{index}m")),
        Color::Rgb(r, g, b) => out.push_str(&format!("\x1b[38;2;{r};{g};{b}m")),
    }
    match style.bg {
        Color::Default => {}
        Color::Indexed(index) if index < 8 => out.push_str(&format!("\x1b[{}m", 40 + index)),
        Color::Indexed(index) if index < 16 => {
            out.push_str(&format!("\x1b[{}m", 100 + index.saturating_sub(8)));
        }
        Color::Indexed(index) => out.push_str(&format!("\x1b[48;5;{index}m")),
        Color::Rgb(r, g, b) => out.push_str(&format!("\x1b[48;2;{r};{g};{b}m")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predict::{Overlay, OverlayCell, OverlayKind};
    use crate::screen::{Cell, Size};

    fn screen_with(text: &[u8]) -> Screen {
        let mut screen = Screen::new(Size { cols: 5, rows: 1 });
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, text);
        screen
    }

    fn printable(pos: Cursor, ch: char) -> OverlayCell {
        OverlayCell {
            pos,
            cell: Cell {
                ch,
                style: Default::default(),
            },
            under: Cell::default(),
            kind: OverlayKind::Printable,
        }
    }

    fn deletion(pos: Cursor, ch: char) -> OverlayCell {
        let style = Style {
            dim: true,
            strikethrough: true,
            synthetic_strike: true,
            ..Style::default()
        };
        OverlayCell {
            pos,
            cell: Cell { ch, style },
            under: Cell {
                ch,
                style: Default::default(),
            },
            kind: OverlayKind::Deletion { remote_seen: false },
        }
    }

    fn overlay(cells: Vec<OverlayCell>, cursor: Option<Cursor>) -> Overlay {
        Overlay {
            enabled: true,
            cells,
            cursor,
        }
    }

    #[test]
    fn first_render_is_full_draw() {
        let screen = screen_with(b"hi");
        let mut renderer = Renderer::new();

        let out = renderer.render(
            &screen,
            &Overlay {
                enabled: true,
                cells: Vec::new(),
                cursor: None,
            },
        );

        assert!(out.contains("\x1b[2J"));
        assert!(out.contains("hi"));
        assert!(!out.contains("hi   "));
    }

    #[test]
    fn first_render_preserves_mixed_styles() {
        let screen = screen_with(b"\x1b[31mR\x1b[0mN");
        let mut renderer = Renderer::new();

        let out = renderer.render(
            &screen,
            &Overlay {
                enabled: true,
                cells: Vec::new(),
                cursor: None,
            },
        );

        assert!(out.contains("\x1b[31mR"));
        assert!(out.contains("\x1b[0mN"));
    }

    #[test]
    fn render_emits_extended_colors() {
        let screen = screen_with(b"\x1b[38;5;196mA\x1b[48;2;10;20;30mB");
        let mut renderer = Renderer::new();

        let out = renderer.render(
            &screen,
            &Overlay {
                enabled: true,
                cells: Vec::new(),
                cursor: None,
            },
        );

        assert!(out.contains("\x1b[38;5;196mA"));
        assert!(out.contains("\x1b[48;2;10;20;30mB"));
    }

    #[test]
    fn render_emits_dim_style() {
        let screen = screen_with(b"\x1b[2mD");
        let mut renderer = Renderer::new();

        let out = renderer.render(
            &screen,
            &Overlay {
                enabled: true,
                cells: Vec::new(),
                cursor: None,
            },
        );

        assert!(out.contains("\x1b[2mD"));
    }

    #[test]
    fn render_emits_strikethrough_style() {
        let screen = screen_with(b"\x1b[9mS");
        let mut renderer = Renderer::new();

        let out = renderer.render(
            &screen,
            &Overlay {
                enabled: true,
                cells: Vec::new(),
                cursor: None,
            },
        );

        assert!(out.contains("\x1b[9mS"));
    }

    #[test]
    fn render_restores_current_remote_style() {
        let mut screen = screen_with(b"\x1b[31mR\x1b[0m");
        let mut renderer = Renderer::new();
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };
        renderer.render(&screen, &overlay);
        screen.feed(&mut vte::Parser::new(), b"\x1b[1;1H\x1b[32mG\x1b[0m");

        let out = renderer.render(&screen, &overlay);

        assert!(out.ends_with("\x1b[0m\x1b[?25h"));
    }

    #[test]
    fn second_render_patches_single_cell() {
        let mut renderer = Renderer::new();
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };
        let first = screen_with(b"hi");
        renderer.render(&first, &overlay);

        let second = screen_with(b"ha");
        let out = renderer.render(&second, &overlay);

        assert!(!out.contains("\x1b[2J"));
        assert!(out.contains("\x1b[1;2H"));
        assert!(out.contains('a'));
        assert!(!out.contains("ha"));
    }

    #[test]
    fn unchanged_frame_emits_nothing() {
        let mut renderer = Renderer::new();
        let screen = screen_with(b"hi");
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: Some(Cursor { row: 0, col: 2 }),
        };

        renderer.render(&screen, &overlay);
        let out = renderer.render(&screen, &overlay);

        assert!(out.is_empty());
    }

    #[test]
    fn second_render_clears_default_tail_with_erase_line() {
        let mut renderer = Renderer::new();
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };
        let first = screen_with(b"hello");
        renderer.render(&first, &overlay);

        let second = screen_with(b"hi");
        let out = renderer.render(&second, &overlay);

        assert!(!out.contains("\x1b[2J"));
        assert!(out.contains("\x1b[1;3H\x1b[0m\x1b[K"));
        assert!(!out.contains("   "));
    }

    #[test]
    fn overlay_composes_over_confirmed_cells() {
        let screen = screen_with(b"");
        let overlay = Overlay {
            enabled: true,
            cells: vec![OverlayCell {
                pos: Cursor { row: 0, col: 0 },
                cell: Cell {
                    ch: 'x',
                    style: Default::default(),
                },
                under: Cell::default(),
                kind: OverlayKind::Printable,
            }],
            cursor: Some(Cursor { row: 0, col: 1 }),
        };

        let frame = compose_frame(&screen, &overlay);

        assert_eq!(frame.cells[0].ch, 'x');
        assert_eq!(frame.cursor, Cursor { row: 0, col: 1 });
    }

    #[test]
    fn overlay_cursor_draws_software_cursor() {
        let screen = screen_with(b"$ ");
        let overlay = overlay(
            vec![printable(Cursor { row: 0, col: 2 }, 'a')],
            Some(Cursor { row: 0, col: 3 }),
        );

        let frame = compose_frame(&screen, &overlay);

        assert_eq!(frame.cursor, Cursor { row: 0, col: 3 });
        assert!(!frame.native_cursor_visible);
        assert!(frame.cells[3].style.reverse);
    }

    #[test]
    fn deletion_under_overlay_cursor_stays_struck() {
        let screen = screen_with(b"$ a");
        let overlay = overlay(
            vec![deletion(Cursor { row: 0, col: 2 }, 'a')],
            Some(Cursor { row: 0, col: 2 }),
        );

        let frame = compose_frame(&screen, &overlay);

        assert_eq!(frame.cells[2].ch, 'a');
        assert!(frame.cells[2].style.strikethrough);
        assert!(frame.cells[2].style.synthetic_strike);
        assert!(frame.cells[2].style.underline);
        assert!(!frame.cells[2].style.reverse);
    }

    #[test]
    fn deletion_overlay_emits_combining_strike() {
        let screen = screen_with(b"$ a");
        let overlay = overlay(
            vec![deletion(Cursor { row: 0, col: 2 }, 'a')],
            Some(Cursor { row: 0, col: 2 }),
        );
        let mut renderer = Renderer::new();

        let out = renderer.render(&screen, &overlay);

        assert!(out.contains("a\u{0336}"));
    }

    #[test]
    fn pending_overlay_hides_native_cursor() {
        let screen = screen_with(b"$ ");
        let overlay = overlay(
            vec![printable(Cursor { row: 0, col: 2 }, 'a')],
            Some(Cursor { row: 0, col: 3 }),
        );
        let mut renderer = Renderer::new();

        let out = renderer.render(&screen, &overlay);

        assert!(out.contains("\x1b[?25l"));
        assert!(!out.contains("\x1b[?25h"));
        assert!(out.contains("\x1b[7m"));
    }

    #[test]
    fn pending_overlay_moves_software_cursor_without_showing_native_cursor() {
        let screen = screen_with(b"$ ");
        let mut renderer = Renderer::new();
        renderer.render(
            &screen,
            &overlay(
                vec![printable(Cursor { row: 0, col: 2 }, 'a')],
                Some(Cursor { row: 0, col: 3 }),
            ),
        );

        let out = renderer.render(
            &screen,
            &overlay(
                vec![
                    printable(Cursor { row: 0, col: 2 }, 'a'),
                    printable(Cursor { row: 0, col: 3 }, 'b'),
                ],
                Some(Cursor { row: 0, col: 4 }),
            ),
        );

        assert!(out.contains('b'));
        assert!(out.contains("\x1b[?25l"));
        assert!(!out.contains("\x1b[?25h"));
    }

    #[test]
    fn repaint_diff_preserves_pending_software_cursor() {
        let mut renderer = Renderer::new();
        let overlay = overlay(
            vec![printable(Cursor { row: 0, col: 2 }, 'a')],
            Some(Cursor { row: 0, col: 3 }),
        );
        renderer.render(&screen_with(b"$  z"), &overlay);

        let out = renderer.render(&screen_with(b"\x1b[H\x1b[2J$ "), &overlay);

        assert!(!out.contains("\x1b[2J"));
        assert!(out.contains("\x1b[?25l"));
        assert!(!out.contains("\x1b[?25h"));
        assert!(out.contains("\x1b[7m"));
    }

    #[test]
    fn native_cursor_returns_when_overlay_clears() {
        let screen = screen_with(b"$ ");
        let mut renderer = Renderer::new();
        renderer.render(
            &screen,
            &overlay(
                vec![printable(Cursor { row: 0, col: 2 }, 'a')],
                Some(Cursor { row: 0, col: 3 }),
            ),
        );

        let out = renderer.render(&screen, &overlay(Vec::new(), None));

        assert!(out.contains("\x1b[?25h"));
    }

    #[test]
    fn invalidate_forces_full_draw_once() {
        let screen = screen_with(b"hi");
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };
        let mut renderer = Renderer::new();
        renderer.render(&screen, &overlay);
        renderer.invalidate();

        let out = renderer.render(&screen, &overlay);
        assert!(out.contains("\x1b[2J"));

        let next = renderer.render(&screen, &overlay);
        assert!(!next.contains("\x1b[2J"));
    }

    #[test]
    fn synced_terminal_uses_diff_without_full_draw() {
        let first = screen_with(b"hi");
        let second = screen_with(b"ho");
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };
        let mut renderer = Renderer::new();

        renderer.sync_to_terminal(&first, &overlay);
        let out = renderer.render(&second, &overlay);

        assert!(!out.contains("\x1b[2J"));
        assert!(out.contains("\x1b[1;2H"));
        assert!(out.contains('o'));
    }
}
