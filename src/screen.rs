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
    pub strikethrough: bool,
    pub synthetic_strike: bool,
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
    saved: SavedState,
    scroll_top: u16,
    scroll_bottom: u16,
    style: Style,
    wrap_next: bool,
    g0_dec_special_graphics: bool,
    g1_dec_special_graphics: bool,
    using_g1: bool,
    origin_mode: bool,
    application_cursor_keys: bool,
}

#[derive(Debug, Clone)]
struct Buffer {
    cells: Vec<Cell>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SavedState {
    cursor: Cursor,
    style: Style,
    wrap_next: bool,
    g0_dec_special_graphics: bool,
    g1_dec_special_graphics: bool,
    using_g1: bool,
    origin_mode: bool,
}

impl Screen {
    pub fn new(size: Size) -> Self {
        Self::new_at(size, Cursor::default())
    }

    pub fn new_at(size: Size, cursor: Cursor) -> Self {
        let bottom = size.rows.saturating_sub(1);
        let mut screen = Self {
            size,
            primary: Buffer::new(size),
            alternate: Buffer::new(size),
            active: ActiveBuffer::Primary,
            cursor,
            saved: SavedState::default(),
            scroll_top: 0,
            scroll_bottom: bottom,
            style: Style::default(),
            wrap_next: false,
            g0_dec_special_graphics: false,
            g1_dec_special_graphics: false,
            using_g1: false,
            origin_mode: false,
            application_cursor_keys: false,
        };
        screen.clamp_cursor();
        screen
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

    pub fn content_below_cursor(&self) -> bool {
        for row in self.cursor.row.saturating_add(1)..self.size.rows {
            for col in 0..self.size.cols {
                if self.cell(Cursor { row, col }) != Cell::default() {
                    return true;
                }
            }
        }
        false
    }

    pub fn cell(&self, cursor: Cursor) -> Cell {
        self.buffer().get(self.size, cursor)
    }

    pub fn feed(&mut self, parser: &mut vte::Parser, bytes: &[u8]) {
        parser.advance(self, bytes);
    }

    #[cfg(test)]
    pub fn resize(&mut self, size: Size) {
        self.resize_buffers(size);
        self.clamp_after_resize(size);
    }

    pub fn resize_for_remote_reflow(&mut self, size: Size) -> bool {
        let clear_active = self.active == ActiveBuffer::Alternate
            || self.application_cursor_keys
            || self.content_below_cursor();
        self.resize_buffers(size);
        if clear_active {
            self.buffer_mut().clear();
            self.style = Style::default();
            self.wrap_next = false;
        }
        self.clamp_after_resize(size);
        clear_active
    }

    fn resize_buffers(&mut self, size: Size) {
        self.primary.resize(self.size, size);
        self.alternate.resize(self.size, size);
    }

    fn clamp_after_resize(&mut self, size: Size) {
        self.size = size;
        self.scroll_top = 0;
        self.scroll_bottom = size.rows.saturating_sub(1);
        self.clamp_cursor();
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
            let blank = self.blank_cell();
            self.buffer_mut().scroll_up(size, top, bottom, 1, blank);
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

    fn move_cursor_addressed(&mut self, row: u16, col: u16) {
        let row = if self.origin_mode {
            self.scroll_top
                .saturating_add(row)
                .min(self.scroll_bottom)
                .min(self.size.rows.saturating_sub(1))
        } else {
            row.min(self.size.rows.saturating_sub(1))
        };
        self.move_cursor(row, col);
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
        let blank = self.blank_cell();
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
                        blank,
                    );
                }
                for row in cursor.row + 1..size.rows {
                    buffer.clear_row(size, row, blank);
                }
            }
            1 => {
                for row in 0..cursor.row {
                    buffer.clear_row(size, row, blank);
                }
                for col in 0..=cursor.col {
                    buffer.set(
                        size,
                        Cursor {
                            row: cursor.row,
                            col,
                        },
                        blank,
                    );
                }
            }
            2 | 3 => buffer.clear_with(blank),
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let blank = self.blank_cell();
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
                        blank,
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
                        blank,
                    );
                }
            }
            2 => buffer.clear_row(size, cursor.row, blank),
            _ => {}
        }
    }

    fn set_scroll_region(&mut self, top: u16, bottom: u16) {
        if top < bottom && bottom < self.size.rows {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.move_cursor_addressed(0, 0);
        }
    }

    fn set_alternate(&mut self, enabled: bool) {
        self.active = if enabled {
            self.save_state();
            self.cursor = Cursor::default();
            self.alternate.clear();
            ActiveBuffer::Alternate
        } else {
            self.restore_state();
            ActiveBuffer::Primary
        };
    }

    fn save_state(&mut self) {
        self.saved = SavedState {
            cursor: self.cursor,
            style: self.style,
            wrap_next: self.wrap_next,
            g0_dec_special_graphics: self.g0_dec_special_graphics,
            g1_dec_special_graphics: self.g1_dec_special_graphics,
            using_g1: self.using_g1,
            origin_mode: self.origin_mode,
        };
    }

    fn restore_state(&mut self) {
        self.cursor = self.saved.cursor;
        self.style = self.saved.style;
        self.wrap_next = self.saved.wrap_next;
        self.g0_dec_special_graphics = self.saved.g0_dec_special_graphics;
        self.g1_dec_special_graphics = self.saved.g1_dec_special_graphics;
        self.using_g1 = self.saved.using_g1;
        self.origin_mode = self.saved.origin_mode;
        self.clamp_cursor();
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
                9 => self.style.strikethrough = true,
                7 => self.style.reverse = true,
                22 => {
                    self.style.bold = false;
                    self.style.dim = false;
                }
                24 => self.style.underline = false,
                27 => self.style.reverse = false,
                29 => self.style.strikethrough = false,
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
        let blank = self.blank_cell();
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
                blank,
            );
        }
    }

    fn delete_chars(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let count = count.min(size.cols.saturating_sub(cursor.col));
        let blank = self.blank_cell();
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
                blank,
            );
        }
    }

    fn erase_chars(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        let count = count.min(size.cols.saturating_sub(cursor.col));
        let blank = self.blank_cell();
        let buffer = self.buffer_mut();
        for col in cursor.col..cursor.col + count {
            buffer.set(
                size,
                Cursor {
                    row: cursor.row,
                    col,
                },
                blank,
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
        let blank = self.blank_cell();
        self.buffer_mut()
            .insert_lines(size, cursor.row, bottom, count, blank);
    }

    fn delete_lines(&mut self, count: u16) {
        let size = self.size;
        let cursor = self.cursor;
        if cursor.row < self.scroll_top || cursor.row > self.scroll_bottom {
            return;
        }
        let bottom = self.scroll_bottom;
        let blank = self.blank_cell();
        self.buffer_mut()
            .delete_lines(size, cursor.row, bottom, count, blank);
    }

    fn scroll_up_lines(&mut self, count: u16) {
        let size = self.size;
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let blank = self.blank_cell();
        self.buffer_mut().scroll_up(size, top, bottom, count, blank);
    }

    fn scroll_down_lines(&mut self, count: u16) {
        let size = self.size;
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let blank = self.blank_cell();
        self.buffer_mut()
            .scroll_down(size, top, bottom, count, blank);
    }

    fn blank_cell(&self) -> Cell {
        Cell {
            ch: ' ',
            style: Style {
                bg: self.style.bg,
                ..Style::default()
            },
        }
    }

    fn clamp_cursor(&mut self) {
        self.move_cursor(self.cursor.row, self.cursor.col);
        self.saved.cursor.row = self.saved.cursor.row.min(self.size.rows.saturating_sub(1));
        self.saved.cursor.col = self.saved.cursor.col.min(self.size.cols.saturating_sub(1));
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
            'G' | '`' => self.move_cursor(self.cursor.row, param(params, 0, 1).saturating_sub(1)),
            'H' | 'f' => self.move_cursor_addressed(
                param(params, 0, 1).saturating_sub(1),
                param(params, 1, 1).saturating_sub(1),
            ),
            'd' => self.move_cursor(param(params, 0, 1).saturating_sub(1), self.cursor.col),
            'J' => self.erase_display(param(params, 0, 0)),
            'K' => self.erase_line(param(params, 0, 0)),
            'L' => self.insert_lines(param(params, 0, 1)),
            'M' => self.delete_lines(param(params, 0, 1)),
            '@' => self.insert_blank_chars(param(params, 0, 1)),
            'P' => self.delete_chars(param(params, 0, 1)),
            'S' => self.scroll_up_lines(param(params, 0, 1)),
            'T' => self.scroll_down_lines(param(params, 0, 1)),
            'X' => self.erase_chars(param(params, 0, 1)),
            'm' => self.set_style(params),
            'r' => self.set_scroll_region(
                param(params, 0, 1).saturating_sub(1),
                param(params, 1, self.size.rows).saturating_sub(1),
            ),
            's' => self.save_state(),
            'u' => self.restore_state(),
            'h' if intermediates == b"?" => {
                if has_private_mode(params, &[47, 1047, 1049]) {
                    self.set_alternate(true);
                }
                if has_private_mode(params, &[6]) {
                    self.origin_mode = true;
                    self.move_cursor_addressed(0, 0);
                }
                if has_private_mode(params, &[1]) {
                    self.application_cursor_keys = true;
                }
            }
            'l' if intermediates == b"?" => {
                if has_private_mode(params, &[47, 1047, 1049]) {
                    self.set_alternate(false);
                }
                if has_private_mode(params, &[6]) {
                    self.origin_mode = false;
                    self.move_cursor(0, 0);
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
            b'7' => self.save_state(),
            b'8' => self.restore_state(),
            b'D' => self.linefeed(),
            b'M' => {
                if self.cursor.row == self.scroll_top {
                    let size = self.size;
                    let top = self.scroll_top;
                    let bottom = self.scroll_bottom;
                    let blank = self.blank_cell();
                    self.buffer_mut().insert_lines(size, top, bottom, 1, blank);
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

    fn clear_with(&mut self, blank: Cell) {
        self.cells.fill(blank);
    }

    fn clear_row(&mut self, size: Size, row: u16, blank: Cell) {
        for col in 0..size.cols {
            self.set(size, Cursor { row, col }, blank);
        }
    }

    fn scroll_up(&mut self, size: Size, top: u16, bottom: u16, count: u16, blank: Cell) {
        let count = count.min(bottom.saturating_sub(top) + 1);
        for row in top..=bottom.saturating_sub(count) {
            for col in 0..size.cols {
                let from = Cursor {
                    row: row + count,
                    col,
                };
                let to = Cursor { row, col };
                self.set(size, to, self.get(size, from));
            }
        }
        for row in bottom.saturating_sub(count) + 1..=bottom {
            self.clear_row(size, row, blank);
        }
    }

    fn scroll_down(&mut self, size: Size, top: u16, bottom: u16, count: u16, blank: Cell) {
        let count = count.min(bottom.saturating_sub(top) + 1);
        for row in (top + count..=bottom).rev() {
            for col in 0..size.cols {
                let from = Cursor {
                    row: row - count,
                    col,
                };
                let to = Cursor { row, col };
                self.set(size, to, self.get(size, from));
            }
        }
        for row in top..top + count {
            self.clear_row(size, row, blank);
        }
    }

    fn insert_lines(&mut self, size: Size, top: u16, bottom: u16, count: u16, blank: Cell) {
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
            self.clear_row(size, row, blank);
        }
    }

    fn delete_lines(&mut self, size: Size, top: u16, bottom: u16, count: u16, blank: Cell) {
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
            self.clear_row(size, row, blank);
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
    fn starts_at_initial_cursor() {
        let mut screen = Screen::new_at(Size { cols: 5, rows: 3 }, Cursor { row: 2, col: 1 });

        feed(&mut screen, b"ok");

        assert_eq!(screen.cell(Cursor { row: 2, col: 1 }).ch, 'o');
        assert_eq!(screen.cell(Cursor { row: 2, col: 2 }).ch, 'k');
        assert_eq!(screen.cursor(), Cursor { row: 2, col: 3 });
    }

    #[test]
    fn handles_newline_carriage_and_backspace() {
        let mut screen = Screen::new(Size { cols: 5, rows: 2 });

        feed(&mut screen, b"ab\rZ\r\nx\x08y");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'Z');
        assert_eq!(screen.cell(Cursor { row: 1, col: 0 }).ch, 'y');
    }

    #[test]
    fn handles_absolute_row_and_column_cursor_motions() {
        let mut screen = Screen::new(Size { cols: 8, rows: 4 });

        feed(&mut screen, b"\x1b[3dA\x1b[5GB\x1b[2`C");

        assert_eq!(screen.cell(Cursor { row: 2, col: 0 }).ch, 'A');
        assert_eq!(screen.cell(Cursor { row: 2, col: 4 }).ch, 'B');
        assert_eq!(screen.cell(Cursor { row: 2, col: 1 }).ch, 'C');
        assert_eq!(screen.cursor(), Cursor { row: 2, col: 2 });
    }

    #[test]
    fn nano_style_vertical_positioning_leaves_cursor_on_edit_row() {
        let mut screen = Screen::new(Size { cols: 40, rows: 24 });

        feed(
            &mut screen,
            b"\x1b[?1049h\x1b[H\x1b[2J\
              GNU nano\r\
              \x1b[23d^G Help\r\
              \x1b[24d^X Exit\r\
              \x1b[22d\x1b[2d",
        );

        assert_eq!(screen.cursor(), Cursor { row: 1, col: 0 });
        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'G');
        assert_eq!(screen.cell(Cursor { row: 1, col: 0 }).ch, ' ');
        assert_eq!(screen.cell(Cursor { row: 22, col: 0 }).ch, '^');
        assert_eq!(screen.cell(Cursor { row: 23, col: 0 }).ch, '^');
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

        feed(&mut screen, b"\x1b[31;1;2;9mA\x1b[29mB\x1b[0mC");

        assert_eq!(
            screen.cell(Cursor { row: 0, col: 0 }).style,
            Style {
                fg: Color::Indexed(1),
                bold: true,
                dim: true,
                strikethrough: true,
                ..Style::default()
            }
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style,
            Style {
                fg: Color::Indexed(1),
                bold: true,
                dim: true,
                ..Style::default()
            }
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 2 }).style,
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
    fn save_restore_preserves_rendition_state() {
        let mut screen = Screen::new(Size { cols: 5, rows: 1 });

        feed(&mut screen, b"\x1b[31mA\x1b7\x1b[42mB\x1b8C");

        assert_eq!(
            screen.cell(Cursor { row: 0, col: 0 }).style.fg,
            Color::Indexed(1)
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style.fg,
            Color::Indexed(1)
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style.bg,
            Color::Default
        );
    }

    #[test]
    fn csi_save_restore_preserves_rendition_state() {
        let mut screen = Screen::new(Size { cols: 5, rows: 1 });

        feed(&mut screen, b"\x1b[34mA\x1b[s\x1b[42mB\x1b[uC");

        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style.fg,
            Color::Indexed(4)
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style.bg,
            Color::Default
        );
    }

    #[test]
    fn origin_mode_addresses_within_scroll_region() {
        let mut screen = Screen::new(Size { cols: 5, rows: 5 });

        feed(&mut screen, b"\x1b[2;4r\x1b[?6h\x1b[1;1HX");

        assert_eq!(screen.cell(Cursor { row: 1, col: 0 }).ch, 'X');
        assert_eq!(screen.cursor(), Cursor { row: 1, col: 1 });

        feed(&mut screen, b"\x1b[?6l\x1b[1;1HY");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'Y');
    }

    #[test]
    fn erase_uses_current_background_color() {
        let mut screen = Screen::new(Size { cols: 4, rows: 1 });

        feed(&mut screen, b"abcd\x1b[42m\x1b[1;2H\x1b[K");

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 'a');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, ' ');
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 1 }).style.bg,
            Color::Indexed(2)
        );
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 3 }).style.bg,
            Color::Indexed(2)
        );
    }

    #[test]
    fn erase_chars_clears_stale_colored_cells() {
        let mut screen = Screen::new(Size { cols: 6, rows: 1 });

        feed(&mut screen, b"\x1b[42mstatus\x1b[0m\x1b[1;1H\x1b[4X");

        for col in 0..4 {
            assert_eq!(screen.cell(Cursor { row: 0, col }).ch, ' ');
            assert_eq!(screen.cell(Cursor { row: 0, col }).style.bg, Color::Default);
        }
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 4 }).style.bg,
            Color::Indexed(2)
        );
    }

    #[test]
    fn status_bar_repaint_does_not_leak_style_after_restore() {
        let mut screen = Screen::new(Size { cols: 12, rows: 4 });

        feed(
            &mut screen,
            b"\x1b[2;1Hedit\
              \x1b7\
              \x1b[4;1H\x1b[42mstatus\
              \x1b8\
              \x1b[3;1H\x1b[K",
        );

        for col in 0..screen.size().cols {
            assert_eq!(screen.cell(Cursor { row: 2, col }).style.bg, Color::Default);
        }
        assert_eq!(
            screen.cell(Cursor { row: 3, col: 0 }).style.bg,
            Color::Indexed(2)
        );
    }

    #[test]
    fn scroll_up_respects_region_above_status_bar() {
        let mut screen = Screen::new(Size { cols: 8, rows: 4 });

        feed(
            &mut screen,
            b"one\r\n\
              two\r\n\
              three\r\n\
              \x1b[42mstatus\x1b[0m\
              \x1b[1;3r\x1b[0m\x1b[S",
        );

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, 't');
        assert_eq!(screen.cell(Cursor { row: 1, col: 0 }).ch, 't');
        assert_eq!(screen.cell(Cursor { row: 2, col: 0 }).ch, ' ');
        assert_eq!(
            screen.cell(Cursor { row: 2, col: 0 }).style.bg,
            Color::Default
        );
        assert_eq!(screen.cell(Cursor { row: 3, col: 0 }).ch, 's');
        assert_eq!(
            screen.cell(Cursor { row: 3, col: 0 }).style.bg,
            Color::Indexed(2)
        );
    }

    #[test]
    fn scroll_down_respects_region_above_status_bar() {
        let mut screen = Screen::new(Size { cols: 8, rows: 4 });

        feed(
            &mut screen,
            b"one\r\n\
              two\r\n\
              three\r\n\
              \x1b[42mstatus\x1b[0m\
              \x1b[1;3r\x1b[0m\x1b[T",
        );

        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, ' ');
        assert_eq!(
            screen.cell(Cursor { row: 0, col: 0 }).style.bg,
            Color::Default
        );
        assert_eq!(screen.cell(Cursor { row: 1, col: 0 }).ch, 'o');
        assert_eq!(screen.cell(Cursor { row: 2, col: 0 }).ch, 't');
        assert_eq!(screen.cell(Cursor { row: 3, col: 0 }).ch, 's');
        assert_eq!(
            screen.cell(Cursor { row: 3, col: 0 }).style.bg,
            Color::Indexed(2)
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
        let mut screen = Screen::new_at(Size { cols: 4, rows: 3 }, Cursor { row: 2, col: 0 });

        feed(&mut screen, b"p\x1b[?1049halt\x1b[?1049l");

        assert_eq!(screen.active(), ActiveBuffer::Primary);
        assert_eq!(screen.cursor(), Cursor { row: 2, col: 1 });
        assert_eq!(screen.cell(Cursor { row: 2, col: 0 }).ch, 'p');
        assert_eq!(
            screen
                .alternate
                .get(screen.size, Cursor { row: 0, col: 0 })
                .ch,
            'a'
        );
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
    fn detects_content_below_cursor() {
        let mut screen = Screen::new(Size { cols: 8, rows: 3 });

        feed(&mut screen, b"$ \x1b[3;1Hstatus\x1b[1;3H");

        assert!(screen.content_below_cursor());

        feed(&mut screen, b"\x1b[2J$ ");

        assert!(!screen.content_below_cursor());
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

    #[test]
    fn resize_reflow_clears_active_layout_content() {
        let mut screen = Screen::new(Size { cols: 10, rows: 4 });

        feed(&mut screen, b"pane\x1b[4;1H\x1b[42mstatus\x1b[0m\x1b[2;1H");

        assert!(screen.resize_for_remote_reflow(Size { cols: 10, rows: 3 }));
        assert!(screen.cells().iter().all(|cell| *cell == Cell::default()));
        assert_eq!(screen.style(), Style::default());
        assert_eq!(screen.cursor(), Cursor { row: 1, col: 0 });
    }

    #[test]
    fn resize_reflow_preserves_simple_prompt() {
        let mut screen = Screen::new(Size { cols: 10, rows: 4 });

        feed(&mut screen, b"$ ");

        assert!(!screen.resize_for_remote_reflow(Size { cols: 8, rows: 4 }));
        assert_eq!(screen.cell(Cursor { row: 0, col: 0 }).ch, '$');
        assert_eq!(screen.cell(Cursor { row: 0, col: 1 }).ch, ' ');
        assert_eq!(screen.cursor(), Cursor { row: 0, col: 2 });
    }
}
