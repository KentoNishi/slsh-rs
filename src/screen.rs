use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub dim: bool,
    pub bold: bool,
    pub underline: bool,
    pub reverse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: Style::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBuffer {
    Primary,
    Alternate,
}

#[derive(Debug, Clone)]
pub struct Screen {
    size: Size,
    primary: Buffer,
    alternate: Buffer,
    active: ActiveBuffer,
    cursor: Cursor,
    saved_cursor: Cursor,
    scroll_top: u16,
    scroll_bottom: u16,
    style: Style,
    wrap_next: bool,
    g0_dec_special_graphics: bool,
    g1_dec_special_graphics: bool,
    using_g1: bool,
    application_cursor_keys: bool,
}

#[derive(Debug, Clone)]
struct Buffer {
    cells: Vec<Cell>,
}

impl Screen {
    pub fn new(size: Size) -> Self {
        let bottom = size.rows.saturating_sub(1);
        Self {
            size,
            primary: Buffer::new(size),
            alternate: Buffer::new(size),
            active: ActiveBuffer::Primary,
            cursor: Cursor::default(),
            saved_cursor: Cursor::default(),
            scroll_top: 0,
            scroll_bottom: bottom,
            style: Style::default(),
            wrap_next: false,
            g0_dec_special_graphics: false,
            g1_dec_special_graphics: false,
            using_g1: false,
            application_cursor_keys: false,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn style(&self) -> Style {
        self.style
    }

    pub fn active(&self) -> ActiveBuffer {
        self.active
    }

    pub fn reset_style(&mut self) {
        self.style = Style::default();
    }

    pub fn application_cursor_keys(&self) -> bool {
        self.application_cursor_keys
    }

    pub fn cell(&self, cursor: Cursor) -> Cell {
        self.buffer().get(self.size, cursor)
    }

    pub fn feed(&mut self, parser: &mut vte::Parser, bytes: &[u8]) {
        parser.advance(self, bytes);
    }

    pub fn resize(&mut self, size: Size) {
        self.primary.resize(self.size, size);
        self.alternate.resize(self.size, size);
        self.size = size;
        self.scroll_top = 0;
        self.scroll_bottom = size.rows.saturating_sub(1);
        self.clamp_cursor();
    }

    pub fn visible_text_tail(&self, rows: u16) -> String {
        let start = self.size.rows.saturating_sub(rows);
        let mut text = String::new();
        for row in start..self.size.rows {
            for col in 0..self.size.cols {
                text.push(self.cell(Cursor { row, col }).ch);
            }
            text.push('\n');
        }
        text
    }

    pub fn cells(&self) -> &[Cell] {
        &self.buffer().cells
    }

    fn buffer(&self) -> &Buffer {
        match self.active {
            ActiveBuffer::Primary => &self.primary,
            ActiveBuffer::Alternate => &self.alternate,
        }
    }

    fn buffer_mut(&mut self) -> &mut Buffer {
        match self.active {
            ActiveBuffer::Primary => &mut self.primary,
            ActiveBuffer::Alternate => &mut self.alternate,
        }
    }

    fn put_char(&mut self, ch: char) {
        let ch = if self.active_charset_is_dec_special_graphics() {
            map_dec_special_graphics(ch)
        } else {
            ch
        };
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 || self.size.cols == 0 || self.size.rows == 0 {
            return;
        }
        if self.wrap_next {
            self.cursor.col = 0;
            self.linefeed();
            self.wrap_next = false;
        }
        if width == 2 && self.cursor.col + 1 >= self.size.cols {
            self.linefeed();
            self.cursor.col = 0;
        }

        let cursor = self.cursor;
        let style = self.style;
        let size = self.size;
        self.buffer_mut().set(size, cursor, Cell { ch, style });
        if width == 2 && cursor.col + 1 < self.size.cols {
            self.buffer_mut().set(
                size,
                Cursor {
                    row: cursor.row,
                    col: cursor.col + 1,
                },
                Cell { ch: ' ', style },
            );
        }
        self.advance(width as u16);
    }

    fn advance(&mut self, width: u16) {
        if self.cursor.col + width < self.size.cols {
            self.cursor.col += width;
            self.wrap_next = false;
        } else {
            self.wrap_next = true;
        }
    }

    fn linefeed(&mut self) {
        self.wrap_next = false;
        if self.cursor.row == self.scroll_bottom {
            let size = self.size;
            let top = self.scroll_top;
            let bottom = self.scroll_bottom;
            self.buffer_mut().scroll_up(size, top, bottom);
        } else if self.cursor.row + 1 < self.size.rows {
            self.cursor.row += 1;
        }
    }

    fn backspace(&mut self) {
        self.wrap_next = false;
        self.cursor.col = self.cursor.col.saturating_sub(1);
    }

    fn tab(&mut self) {
        self.wrap_next = false;
        let next = ((self.cursor.col / 8) + 1) * 8;
        self.cursor.col = next.min(self.size.cols.saturating_sub(1));
    }

    fn move_cursor(&mut self, row: u16, col: u16) {
        self.wrap_next = false;
        self.cursor = Cursor {
            row: row.min(self.size.rows.saturating_sub(1)),
            col: col.min(self.size.cols.saturating_sub(1)),
        };
    }

    fn move_relative(&mut self, rows: i32, cols: i32) {
        self.wrap_next = false;
        let row = (self.cursor.row as i32 + rows).clamp(0, self.size.rows.saturating_sub(1) as i32)
            as u16;
        let col = (self.cursor.col as i32 + cols).clamp(0, self.size.cols.saturating_sub(1) as i32)
            as u16;
        self.cursor = Cursor { row, col };
    }

    fn erase_display(&mut self, mode: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let buffer = self.buffer_mut();
        match mode {
            0 => {
                for col in cursor.col..size.cols {
                    buffer.set(
                        size,
                        Cursor {
                            row: cursor.row,
                            col,
                        },
                        Cell::default(),
                    );
                }
                for row in cursor.row + 1..size.rows {
                    buffer.clear_row(size, row);
                }
            }
            1 => {
                for row in 0..cursor.row {
                    buffer.clear_row(size, row);
                }
                for col in 0..=cursor.col {
                    buffer.set(
                        size,
                        Cursor {
                            row: cursor.row,
                            col,
                        },
                        Cell::default(),
                    );
                }
            }
            2 | 3 => buffer.clear(),
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let buffer = self.buffer_mut();
        match mode {
            0 => {
                for col in cursor.col..size.cols {
                    buffer.set(
                        size,
                        Cursor {
                            row: cursor.row,
                            col,
                        },
                        Cell::default(),
                    );
                }
            }
            1 => {
                for col in 0..=cursor.col {
                    buffer.set(
                        size,
                        Cursor {
                            row: cursor.row,
                            col,
                        },
                        Cell::default(),
                    );
                }
            }
            2 => buffer.clear_row(size, cursor.row),
            _ => {}
        }
    }

    fn set_scroll_region(&mut self, top: u16, bottom: u16) {
        if top < bottom && bottom < self.size.rows {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.cursor = Cursor::default();
        }
    }

    fn set_alternate(&mut self, enabled: bool) {
        self.active = if enabled {
            self.saved_cursor = self.cursor;
            self.alternate.clear();
            ActiveBuffer::Alternate
        } else {
            self.cursor = self.saved_cursor;
            ActiveBuffer::Primary
        };
    }

    fn active_charset_is_dec_special_graphics(&self) -> bool {
        if self.using_g1 {
            self.g1_dec_special_graphics
        } else {
            self.g0_dec_special_graphics
        }
    }

    fn set_style(&mut self, params: &Params) {
        if params.is_empty() {
            self.style = Style::default();
            return;
        }

        let groups: Vec<&[u16]> = params.iter().collect();
        let mut index = 0;
        while let Some(param) = groups.get(index).copied() {
            let code = param.first().copied().unwrap_or(0);
            match code {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                2 => self.style.dim = true,
                4 => self.style.underline = true,
                7 => self.style.reverse = true,
                22 => {
                    self.style.bold = false;
                    self.style.dim = false;
                }
                24 => self.style.underline = false,
                27 => self.style.reverse = false,
                30..=37 => self.style.fg = Color::Indexed((code - 30) as u8),
                39 => self.style.fg = Color::Default,
                40..=47 => self.style.bg = Color::Indexed((code - 40) as u8),
                49 => self.style.bg = Color::Default,
                38 => {
                    if let Some((color, consumed)) = extended_color(param, &groups[index + 1..]) {
                        self.style.fg = color;
                        index += consumed;
                    }
                }
                48 => {
                    if let Some((color, consumed)) = extended_color(param, &groups[index + 1..]) {
                        self.style.bg = color;
                        index += consumed;
                    }
                }
                90..=97 => self.style.fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => self.style.bg = Color::Indexed((code - 100 + 8) as u8),
                _ => {}
            }
            index += 1;
        }
    }

    fn insert_blank_chars(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let count = count.min(size.cols.saturating_sub(cursor.col));
        let buffer = self.buffer_mut();
        for col in (cursor.col..size.cols.saturating_sub(count)).rev() {
            let from = Cursor {
                row: cursor.row,
                col,
            };
            let to = Cursor {
                row: cursor.row,
                col: col + count,
            };
            buffer.set(size, to, buffer.get(size, from));
        }
        for col in cursor.col..cursor.col + count {
            buffer.set(
                size,
                Cursor {
                    row: cursor.row,
                    col,
                },
                Cell::default(),
            );
        }
    }

    fn delete_chars(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let count = count.min(size.cols.saturating_sub(cursor.col));
        let buffer = self.buffer_mut();
        for col in cursor.col + count..size.cols {
            let from = Cursor {
                row: cursor.row,
                col,
            };
            let to = Cursor {
                row: cursor.row,
                col: col - count,
            };
            buffer.set(size, to, buffer.get(size, from));
        }
        for col in size.cols.saturating_sub(count)..size.cols {
            buffer.set(
                size,
                Cursor {
                    row: cursor.row,
                    col,
                },
                Cell::default(),
            );
        }
    }

    fn insert_lines(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        if cursor.row < self.scroll_top || cursor.row > self.scroll_bottom {
            return;
        }
        let bottom = self.scroll_bottom;
        self.buffer_mut()
            .insert_lines(size, cursor.row, bottom, count);
    }

    fn delete_lines(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        if cursor.row < self.scroll_top || cursor.row > self.scroll_bottom {
            return;
        }
        let bottom = self.scroll_bottom;
        self.buffer_mut()
            .delete_lines(size, cursor.row, bottom, count);
    }

    fn clamp_cursor(&mut self) {
        self.move_cursor(self.cursor.row, self.cursor.col);
        self.saved_cursor.row = self.saved_cursor.row.min(self.size.rows.saturating_sub(1));
        self.saved_cursor.col = self.saved_cursor.col.min(self.size.cols.saturating_sub(1));
    }
}

impl Perform for Screen {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            b'\r' => {
                self.cursor.col = 0;
                self.wrap_next = false;
            }
            0x08 => self.backspace(),
            b'\t' => self.tab(),
            0x0e => self.using_g1 = true,
            0x0f => self.using_g1 = false,
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }

        match action {
            'A' => self.move_relative(-(param(params, 0, 1) as i32), 0),
            'B' => self.move_relative(param(params, 0, 1) as i32, 0),
            'C' => self.move_relative(0, param(params, 0, 1) as i32),
            'D' => self.move_relative(0, -(param(params, 0, 1) as i32)),
            'H' | 'f' => self.move_cursor(
                param(params, 0, 1).saturating_sub(1),
                param(params, 1, 1).saturating_sub(1),
            ),
            'J' => self.erase_display(param(params, 0, 0)),
            'K' => self.erase_line(param(params, 0, 0)),
            'L' => self.insert_lines(param(params, 0, 1)),
            'M' => self.delete_lines(param(params, 0, 1)),
            '@' => self.insert_blank_chars(param(params, 0, 1)),
            'P' => self.delete_chars(param(params, 0, 1)),
            'm' => self.set_style(params),
            'r' => self.set_scroll_region(
                param(params, 0, 1).saturating_sub(1),
                param(params, 1, self.size.rows).saturating_sub(1),
            ),
            'h' if intermediates == b"?" => {
                if has_private_mode(params, &[47, 1047, 1049]) {
                    self.set_alternate(true);
                }
                if has_private_mode(params, &[1]) {
                    self.application_cursor_keys = true;
                }
            }
            'l' if intermediates == b"?" => {
                if has_private_mode(params, &[47, 1047, 1049]) {
                    self.set_alternate(false);
                }
                if has_private_mode(params, &[1]) {
                    self.application_cursor_keys = false;
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }

        if matches!(intermediates, b"(" | b")") {
            let enabled = match byte {
                b'0' => Some(true),
                b'B' => Some(false),
                _ => None,
            };
            if let Some(enabled) = enabled {
                if intermediates == b"(" {
                    self.g0_dec_special_graphics = enabled;
                } else {
                    self.g1_dec_special_graphics = enabled;
                }
            }
            return;
        }

        match byte {
            b'7' => self.saved_cursor = self.cursor,
            b'8' => self.cursor = self.saved_cursor,
            b'D' => self.linefeed(),
            b'M' => {
                if self.cursor.row == self.scroll_top {
                    let size = self.size;
                    let top = self.scroll_top;
                    let bottom = self.scroll_bottom;
                    self.buffer_mut().insert_lines(size, top, bottom, 1);
                } else {
                    self.cursor.row = self.cursor.row.saturating_sub(1);
                }
            }
            b'c' => *self = Screen::new(self.size),
            _ => {}
        }
    }
}

impl Buffer {
    fn new(size: Size) -> Self {
        Self {
            cells: vec![Cell::default(); size.cols as usize * size.rows as usize],
        }
    }

    fn get(&self, size: Size, cursor: Cursor) -> Cell {
        self.index(size, cursor)
            .and_then(|index| self.cells.get(index).copied())
            .unwrap_or_default()
    }

    fn set(&mut self, size: Size, cursor: Cursor, cell: Cell) {
        if let Some(index) = self.index(size, cursor) {
            if let Some(slot) = self.cells.get_mut(index) {
                *slot = cell;
            }
        }
    }

    fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    fn clear_row(&mut self, size: Size, row: u16) {
        for col in 0..size.cols {
            self.set(size, Cursor { row, col }, Cell::default());
        }
    }

    fn scroll_up(&mut self, size: Size, top: u16, bottom: u16) {
        for row in top..bottom {
            for col in 0..size.cols {
                let from = Cursor { row: row + 1, col };
                let to = Cursor { row, col };
                self.set(size, to, self.get(size, from));
            }
        }
        self.clear_row(size, bottom);
    }

    fn insert_lines(&mut self, size: Size, top: u16, bottom: u16, count: u16) {
        let count = count.min(bottom.saturating_sub(top) + 1);
        for row in (top..=bottom.saturating_sub(count)).rev() {
            for col in 0..size.cols {
                let from = Cursor { row, col };
                let to = Cursor {
                    row: row + count,
                    col,
                };
                self.set(size, to, self.get(size, from));
            }
        }
        for row in top..top + count {
            self.clear_row(size, row);
        }
    }

    fn delete_lines(&mut self, size: Size, top: u16, bottom: u16, count: u16) {
        let count = count.min(bottom.saturating_sub(top) + 1);
        for row in top + count..=bottom {
            for col in 0..size.cols {
                let from = Cursor { row, col };
                let to = Cursor {
                    row: row - count,
                    col,
                };
                self.set(size, to, self.get(size, from));
            }
        }
        for row in bottom.saturating_sub(count) + 1..=bottom {
            self.clear_row(size, row);
        }
    }

    fn resize(&mut self, old: Size, new: Size) {
        let old_cells = self.cells.clone();
        self.cells = vec![Cell::default(); new.cols as usize * new.rows as usize];
        let rows = old.rows.min(new.rows);
        let cols = old.cols.min(new.cols);
        for row in 0..rows {
            for col in 0..cols {
                let old_index = row as usize * old.cols as usize + col as usize;
                let new_index = row as usize * new.cols as usize + col as usize;
                self.cells[new_index] = old_cells[old_index];
            }
        }
    }

    fn index(&self, size: Size, cursor: Cursor) -> Option<usize> {
        if cursor.row < size.rows && cursor.col < size.cols {
            Some(cursor.row as usize * size.cols as usize + cursor.col as usize)
        } else {
            None
        }
    }
}

fn param(params: &Params, index: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(index)
        .and_then(|param| param.first().copied())
        .filter(|value| *value != 0)
        .unwrap_or(default)
}

fn has_private_mode(params: &Params, modes: &[u16]) -> bool {
    params
        .iter()
        .filter_map(|param| param.first().copied())
        .any(|value| modes.contains(&value))
}

fn extended_color(current: &[u16], rest: &[&[u16]]) -> Option<(Color, usize)> {
    if current.len() >= 3 && current[1] == 5 {
        return Some((Color::Indexed(current[2].min(u8::MAX as u16) as u8), 0));
    }
    if current.len() >= 5 && current[1] == 2 {
        let rgb = &current[current.len() - 3..];
        return Some((rgb_color(rgb[0], rgb[1], rgb[2]), 0));
    }
    if rest.len() >= 2 && rest[0].first() == Some(&5) {
        return rest[1]
            .first()
            .map(|index| (Color::Indexed((*index).min(u8::MAX as u16) as u8), 2));
    }
    if rest.len() >= 4 && rest[0].first() == Some(&2) {
        return match (
            rest[1].first().copied(),
            rest[2].first().copied(),
            rest[3].first().copied(),
        ) {
            (Some(r), Some(g), Some(b)) => Some((rgb_color(r, g, b), 4)),
            _ => None,
        };
    }
    None
}

fn rgb_color(r: u16, g: u16, b: u16) -> Color {
    Color::Rgb(
        r.min(u8::MAX as u16) as u8,
        g.min(u8::MAX as u16) as u8,
        b.min(u8::MAX as u16) as u8,
    )
}

fn map_dec_special_graphics(ch: char) -> char {
    match ch {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'q' => '─',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        '~' => '·',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(screen: &mut Screen, bytes: &[u8]) {
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, bytes);
    }

    #[test]
    fn prints_and_moves_cursor() {
        let mut screen = Screen::new(Size { cols: 5, rows: 2 });

        feed(&mut screen, b"ab");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'a');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, 'b');
        assert_eq!(screen.cursor(), Cursor { row: 0, col: 2 });
    }

    #[test]
    fn handles_newline_carriage_and_backspace() {
        let mut screen = Screen::new(Size { cols: 5, rows: 2 });

        feed(&mut screen, b"ab\rZ\r\nx\x08y");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'Z');
        assert_eq!(screen.cell(Cursor { row: 1, col: 0 }).ch, 'y');
    }

    #[test]
    fn erases_line_and_display() {
        let mut screen = Screen::new(Size { cols: 5, rows: 2 });

        feed(&mut screen, b"hello\x1b[1;2H\x1b[K");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'h');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, ' ');
        assert_eq!(screen.cell(Cursor { row: 0, col: 4 }).ch, ' ');
    }

    #[test]
    fn scrolls_at_bottom() {
        let mut screen = Screen::new(Size { cols: 3, rows: 2 });

        feed(&mut screen, b"a\r\nb\r\nc");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'b');
    }

    #[test]
    fn tracks_basic_style() {
        let mut screen = Screen::new(Size { cols: 3, rows: 1 });

        feed(&mut screen, b"\x1b[31;1;2mA\x1b[0mB");

        assert_eq!(
            screen.cell(Cursor { row: 0, col: 0 }).style,
            Style {
                fg: Color::Indexed(1),
                bold: true,
                dim: true,
                ..Style::default()
            }
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style,
            Style::default()
        );
    }

    #[test]
    fn tracks_extended_colors() {
        let mut screen = Screen::new(Size { cols: 4, rows: 1 });

        feed(&mut screen, b"\x1b[38;5;196mA\x1b[48;2;10;20;30mB\x1b[0mC");

        assert_eq!(
            screen.cell(Cursor { row: 0, col: 0 }).style.fg,
            Color::Indexed(196)
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style.bg,
            Color::Rgb(10, 20, 30)
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 2 }).style,
            Style::default()
        );
    }

    #[test]
    fn maps_dec_special_graphics() {
        let mut screen = Screen::new(Size { cols: 4, rows: 1 });

        feed(&mut screen, b"\x1b(0lqk\x1b(Bx");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, '┌');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, '─');
        assert_eq!(screen.cell(Cursor { row: 0, col: 2 }).ch, '┐');
        assert_eq!(screen.cell(Cursor { row: 0, col: 3 }).ch, 'x');
    }

    #[test]
    fn maps_shifted_dec_special_graphics() {
        let mut screen = Screen::new(Size { cols: 4, rows: 1 });

        feed(&mut screen, b"\x1b)0\x0elqk\x0fx");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, '┌');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, '─');
        assert_eq!(screen.cell(Cursor { row: 0, col: 2 }).ch, '┐');
        assert_eq!(screen.cell(Cursor { row: 0, col: 3 }).ch, 'x');
    }

    #[test]
    fn switches_alternate_screen() {
        let mut screen = Screen::new(Size { cols: 4, rows: 1 });

        feed(&mut screen, b"main\x1b[?1049halt\x1b[?1049l");

        assert_eq!(screen.active(), ActiveBuffer::Primary);
        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'm');
    }

    #[test]
    fn tracks_application_cursor_key_mode() {
        let mut screen = Screen::new(Size { cols: 3, rows: 1 });

        feed(&mut screen, b"\x1b[?1h");
        assert!(screen.application_cursor_keys());

        feed(&mut screen, b"\x1b[?1l");
        assert!(!screen.application_cursor_keys());
    }

    #[test]
    fn resize_preserves_overlap() {
        let mut screen = Screen::new(Size { cols: 4, rows: 2 });

        feed(&mut screen, b"abcd");
        screen.resize(Size { cols: 2, rows: 1 });

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'a');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, 'b');
        assert_eq!(screen.cursor(), Cursor { row: 0, col: 1 });
    }
}
