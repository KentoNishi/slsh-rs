use crossterm::event::KeyEvent;
#[cfg(not(windows))]
use crossterm::event::MouseEvent;
use std::io;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    #[cfg(not(windows))]
    Mouse(MouseEvent),
    #[cfg(not(windows))]
    Paste(String),
    Resize(u16, u16),
}

#[cfg(not(windows))]
pub fn poll(timeout: Duration) -> io::Result<bool> {
    crossterm::event::poll(timeout)
}

#[cfg(not(windows))]
pub fn read() -> io::Result<Option<InputEvent>> {
    match crossterm::event::read()? {
        crossterm::event::Event::Key(key) => Ok(Some(InputEvent::Key(key))),
        crossterm::event::Event::Mouse(mouse) => Ok(Some(InputEvent::Mouse(mouse))),
        crossterm::event::Event::Paste(text) => Ok(Some(InputEvent::Paste(text))),
        crossterm::event::Event::Resize(cols, rows) => Ok(Some(InputEvent::Resize(cols, rows))),
        _ => Ok(None),
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::ptr;

    const STD_INPUT_HANDLE: i32 = -10;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 0x00000102;
    const KEY_EVENT: u16 = 0x0001;
    const WINDOW_BUFFER_SIZE_EVENT: u16 = 0x0004;

    const RIGHT_ALT_PRESSED: u32 = 0x0001;
    const LEFT_ALT_PRESSED: u32 = 0x0002;
    const RIGHT_CTRL_PRESSED: u32 = 0x0004;
    const LEFT_CTRL_PRESSED: u32 = 0x0008;
    const SHIFT_PRESSED: u32 = 0x0010;

    const VK_BACK: u16 = 0x08;
    const VK_TAB: u16 = 0x09;
    const VK_RETURN: u16 = 0x0D;
    const VK_ESCAPE: u16 = 0x1B;
    const VK_PRIOR: u16 = 0x21;
    const VK_NEXT: u16 = 0x22;
    const VK_END: u16 = 0x23;
    const VK_HOME: u16 = 0x24;
    const VK_LEFT: u16 = 0x25;
    const VK_UP: u16 = 0x26;
    const VK_RIGHT: u16 = 0x27;
    const VK_DOWN: u16 = 0x28;
    const VK_INSERT: u16 = 0x2D;
    const VK_DELETE: u16 = 0x2E;
    const VK_F1: u16 = 0x70;
    const VK_F24: u16 = 0x87;
    const VK_0: u16 = b'0' as u16;
    const VK_9: u16 = b'9' as u16;
    const VK_A: u16 = b'A' as u16;
    const VK_Z: u16 = b'Z' as u16;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeyEventRecord {
        key_down: i32,
        repeat_count: u16,
        virtual_key_code: u16,
        virtual_scan_code: u16,
        unicode_char: u16,
        control_key_state: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    union InputRecordEvent {
        key_event: KeyEventRecord,
        window_buffer_size: Coord,
        _padding: [u32; 4],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct InputRecord {
        event_type: u16,
        event: InputRecordEvent,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(nStdHandle: i32) -> *mut c_void;
        fn WaitForSingleObject(hHandle: *mut c_void, dwMilliseconds: u32) -> u32;
        fn ReadConsoleInputW(
            hConsoleInput: *mut c_void,
            lpBuffer: *mut InputRecord,
            nLength: u32,
            lpNumberOfEventsRead: *mut u32,
        ) -> i32;
    }

    pub fn poll(timeout: Duration) -> io::Result<bool> {
        let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
        match unsafe { WaitForSingleObject(stdin_handle()?, millis) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub fn read() -> io::Result<Option<InputEvent>> {
        let mut record = MaybeUninit::<InputRecord>::zeroed();
        let mut read = 0;
        let ok = unsafe { ReadConsoleInputW(stdin_handle()?, record.as_mut_ptr(), 1, &mut read) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            return Ok(None);
        }

        let record = unsafe { record.assume_init() };
        match record.event_type {
            KEY_EVENT => {
                let key = unsafe { record.event.key_event };
                Ok(map_key(key).map(InputEvent::Key))
            }
            WINDOW_BUFFER_SIZE_EVENT => {
                let size = unsafe { record.event.window_buffer_size };
                Ok(Some(InputEvent::Resize(
                    size.x.max(1) as u16,
                    size.y.max(1) as u16,
                )))
            }
            _ => Ok(None),
        }
    }

    fn stdin_handle() -> io::Result<*mut c_void> {
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        if handle.is_null() || handle == ptr::null_mut::<c_void>().wrapping_offset(-1) {
            Err(io::Error::last_os_error())
        } else {
            Ok(handle)
        }
    }

    fn map_key(record: KeyEventRecord) -> Option<KeyEvent> {
        let modifiers = modifiers(record.control_key_state);
        let kind = if record.key_down != 0 {
            if record.repeat_count > 1 {
                KeyEventKind::Repeat
            } else {
                KeyEventKind::Press
            }
        } else {
            KeyEventKind::Release
        };

        let code = named_key(record.virtual_key_code, modifiers)
            .or_else(|| unicode_key(record.unicode_char, record.virtual_key_code, modifiers))?;

        Some(KeyEvent {
            code,
            modifiers,
            kind,
            state: KeyEventState::NONE,
        })
    }

    fn named_key(vk: u16, modifiers: KeyModifiers) -> Option<KeyCode> {
        match vk {
            VK_BACK => Some(KeyCode::Backspace),
            VK_TAB if modifiers.contains(KeyModifiers::SHIFT) => Some(KeyCode::BackTab),
            VK_TAB => Some(KeyCode::Tab),
            VK_RETURN => Some(KeyCode::Enter),
            VK_ESCAPE => Some(KeyCode::Esc),
            VK_PRIOR => Some(KeyCode::PageUp),
            VK_NEXT => Some(KeyCode::PageDown),
            VK_END => Some(KeyCode::End),
            VK_HOME => Some(KeyCode::Home),
            VK_LEFT => Some(KeyCode::Left),
            VK_UP => Some(KeyCode::Up),
            VK_RIGHT => Some(KeyCode::Right),
            VK_DOWN => Some(KeyCode::Down),
            VK_INSERT => Some(KeyCode::Insert),
            VK_DELETE => Some(KeyCode::Delete),
            VK_F1..=VK_F24 => Some(KeyCode::F((vk - VK_F1 + 1) as u8)),
            VK_A..=VK_Z if modifiers.contains(KeyModifiers::CONTROL) => {
                Some(KeyCode::Char((vk as u8 as char).to_ascii_lowercase()))
            }
            _ => None,
        }
    }

    fn unicode_key(unicode: u16, vk: u16, modifiers: KeyModifiers) -> Option<KeyCode> {
        match unicode {
            0 => ascii_virtual_key(vk, modifiers).map(KeyCode::Char),
            0x08 | 0x7f => Some(KeyCode::Backspace),
            0x09 => Some(KeyCode::Tab),
            0x0d | 0x0a => Some(KeyCode::Enter),
            0x1b => Some(KeyCode::Esc),
            0x01..=0x1a | 0x1c..=0x1f => char::from_u32(unicode as u32).map(KeyCode::Char),
            _ => char::from_u32(unicode as u32).map(KeyCode::Char),
        }
    }

    fn ascii_virtual_key(vk: u16, modifiers: KeyModifiers) -> Option<char> {
        match vk {
            VK_A..=VK_Z => {
                let ch = vk as u8 as char;
                Some(if modifiers.contains(KeyModifiers::SHIFT) {
                    ch
                } else {
                    ch.to_ascii_lowercase()
                })
            }
            VK_0..=VK_9 => Some(vk as u8 as char),
            _ => None,
        }
    }

    fn modifiers(state: u32) -> KeyModifiers {
        let mut modifiers = KeyModifiers::empty();
        if state & SHIFT_PRESSED != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        modifiers
    }
}

#[cfg(windows)]
pub use windows::{poll, read};
