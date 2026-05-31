use crate::predict::Overlay;
use crate::screen::{Cell, Color, Cursor, Screen, Size, Style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub size: Size,
    pub cells: Vec<Cell>,
    pub cursor: Cursor,
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

    pub fn render(&mut self, screen: &Screen, overlay: &Overlay) -> String {
        let next = compose_frame(screen, overlay);
        let output =
            if self.force_full || self.last.as_ref().map(|last| last.size) != Some(next.size) {
                full_draw(&next)
            } else {
                diff_draw(self.last.as_ref().expect("last frame exists"), &next)
            };
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

    Frame {
        size: screen.size(),
        cells,
        cursor: overlay.cursor.unwrap_or_else(|| screen.cursor()),
    }
}

fn full_draw(frame: &Frame) -> String {
    let mut out = String::from("\x1b[?25l\x1b[0m\x1b[2J");
    for row in 0..frame.size.rows {
        emit_changed_row(&mut out, frame, row);
    }
    move_cursor(&mut out, frame.cursor);
    out
}

fn diff_draw(last: &Frame, next: &Frame) -> String {
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

    let first = row as usize * frame.size.cols as usize + start as usize;
    move_cursor(out, Cursor { row, col: start });
    set_style(out, frame.cells[first].style);
    for col in start..end {
        let index = row as usize * frame.size.cols as usize + col as usize;
        out.push(frame.cells[index].ch);
    }
}

fn move_cursor(out: &mut String, cursor: Cursor) {
    out.push_str(&format!("\x1b[{};{}H", cursor.row + 1, cursor.col + 1));
}

fn set_style(out: &mut String, style: Style) {
    out.push_str("\x1b[0m");
    if style.bold {
        out.push_str("\x1b[1m");
    }
    if style.underline {
        out.push_str("\x1b[4m");
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
    use crate::predict::{Overlay, OverlayCell};
    use crate::screen::{Cell, Size};

    fn screen_with(text: &[u8]) -> Screen {
        let mut screen = Screen::new(Size { cols: 5, rows: 1 });
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, text);
        screen
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
            }],
            cursor: Some(Cursor { row: 0, col: 1 }),
        };

        let frame = compose_frame(&screen, &overlay);

        assert_eq!(frame.cells[0].ch, 'x');
        assert_eq!(frame.cursor, Cursor { row: 0, col: 1 });
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
}
