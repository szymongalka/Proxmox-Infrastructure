//! Palette from szymongalka-dark's active CRT v4 stylesheet.
//! Terminal fonts remain a preference of the terminal emulator.
use ratatui::style::{Color, Style};

pub const BG: Color = Color::Rgb(0x09, 0x0a, 0x09);
pub const CARD: Color = Color::Rgb(0x0e, 0x0e, 0x0e);
pub const ELEVATED: Color = Color::Rgb(0x13, 0x13, 0x13);
pub const HOVER: Color = Color::Rgb(0x19, 0x19, 0x19);
pub const TEXT: Color = Color::Rgb(0xc6, 0xc6, 0xc6);
pub const STRONG: Color = Color::Rgb(0xec, 0xec, 0xec);
pub const MUTED: Color = Color::Rgb(0x99, 0x99, 0x99);
pub const BORDER: Color = Color::Rgb(0x23, 0x23, 0x23);
pub const BORDER_STRONG: Color = Color::Rgb(0x34, 0x34, 0x34);
pub const PRIMARY: Color = Color::Rgb(0xe8, 0xe8, 0xe8);
pub const INK: Color = Color::Rgb(0x11, 0x11, 0x11);
pub const AMBER: Color = Color::Rgb(0xff, 0xcf, 0x5c);
pub const ERROR: Color = Color::Rgb(0xd0, 0x34, 0x2d);

pub fn base() -> Style {
    Style::default().fg(TEXT).bg(BG)
}
pub fn card() -> Style {
    Style::default().fg(TEXT).bg(CARD)
}
pub fn button() -> Style {
    Style::default().fg(INK).bg(PRIMARY)
}
