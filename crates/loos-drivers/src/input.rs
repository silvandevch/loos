//! Input events shared by every keyboard/mouse driver (PS/2 now, USB HID later).

/// Modifier bits in [`KeyEvent::mods`].
pub mod mods {
    /// Shift.
    pub const SHIFT: u8 = 1;
    /// Control.
    pub const CTRL: u8 = 2;
    /// Alt.
    pub const ALT: u8 = 4;
    /// Super / Windows key.
    pub const SUPER: u8 = 8;
}

/// Non-character key codes (same values the kernel's input layer already uses for specials).
pub mod keys {
    /// Up arrow.
    pub const UP: u32 = 0x100;
    /// Down arrow.
    pub const DOWN: u32 = 0x101;
    /// Left arrow.
    pub const LEFT: u32 = 0x102;
    /// Right arrow.
    pub const RIGHT: u32 = 0x103;
    /// Home.
    pub const HOME: u32 = 0x104;
    /// End.
    pub const END: u32 = 0x105;
    /// Page up.
    pub const PGUP: u32 = 0x106;
    /// Page down.
    pub const PGDN: u32 = 0x107;
    /// First function key (F1); F`n` is `F1 + n - 1`.
    pub const F1: u32 = 0x120;
    /// Super key.
    pub const SUPER: u32 = 0x140;
    /// Delete.
    pub const DELETE: u32 = 127;
}

/// A key press or release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Unshifted symbol or a [`keys`] code.
    pub key: u32,
    /// Typed character (shift applied), 0 when the key produces none.
    pub ch: u32,
    /// True on press.
    pub down: bool,
    /// Active [`mods`] after this event.
    pub mods: u8,
}

/// A relative mouse movement / button / wheel report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseEvent {
    /// Horizontal delta (right positive).
    pub dx: i32,
    /// Vertical delta (down positive, screen convention).
    pub dy: i32,
    /// Wheel delta (up positive).
    pub wheel: i32,
    /// Button bits: 1 left, 2 right, 4 middle, 8 back, 16 forward.
    pub buttons: u8,
}

/// Either kind of input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Keyboard.
    Key(KeyEvent),
    /// Mouse.
    Mouse(MouseEvent),
}
