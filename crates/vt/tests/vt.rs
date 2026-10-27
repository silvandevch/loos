use loos_vt::parser::{Event, Parser};
use loos_vt::screen::{Color, Screen, attr};

fn run(cols: usize, rows: usize, s: &[u8]) -> Screen {
    let mut sc = Screen::new(cols, rows);
    sc.write(&mut Parser::new(), s);
    sc
}

#[test]
fn parses_csi_split_across_chunks() {
    let mut p = Parser::new();
    let mut ev = vec![];
    p.feed(b"\x1b[1;3", &mut ev);
    assert!(ev.is_empty());
    p.feed(b"1mA", &mut ev);
    assert_eq!(ev[0], Event::Csi { params: vec![1, 31], private: None, inter: None, fin: 'm' });
    assert_eq!(ev[1], Event::Print('A'));
}

#[test]
fn utf8_and_invalid() {
    let sc = run(10, 2, "é€😀".as_bytes());
    assert_eq!(sc.row_text(0), "é€😀");
    let sc = run(10, 1, b"\xff\xe2\x82x");
    assert_eq!(sc.row_text(0), "\u{FFFD}\u{FFFD}x");
}

#[test]
fn cursor_erase_and_sgr() {
    let sc = run(10, 3, b"hello\x1b[2;3H\x1b[1;31mX\x1b[0m\x1b[1;1H\x1b[K");
    assert_eq!(sc.row_text(0), "");
    assert_eq!(sc.row_text(1), "  X");
    let c = sc.cell(2, 1);
    assert_eq!((c.fg, c.attrs), (Color::Idx(1), attr::BOLD));
    let sc = run(4, 1, b"\x1b[38;2;1;2;3;48;5;200mZ");
    assert_eq!((sc.cell(0, 0).fg, sc.cell(0, 0).bg), (Color::Rgb(1, 2, 3), Color::Idx(200)));
}

#[test]
fn wrap_scroll_and_scrollback() {
    let sc = run(3, 2, b"abcdefg");
    assert_eq!((sc.row_text(0), sc.row_text(1)), ("def".into(), "g".into()));
    assert_eq!(sc.scrollback.len(), 1);
    let sc = run(4, 2, b"a\r\nb\r\nc");
    assert_eq!((sc.row_text(0), sc.row_text(1)), ("b".into(), "c".into()));
}

#[test]
fn osc_title_and_private_modes() {
    let sc = run(5, 1, b"\x1b]0;my term\x07\x1b[?25l");
    assert_eq!(sc.title, "my term");
    assert!(!sc.cursor_visible);
    let sc = run(5, 1, b"\x1b]2;st\x1b\\\x1b[?25l\x1b[?25h");
    assert_eq!(sc.title, "st");
    assert!(sc.cursor_visible);
}

#[test]
fn insert_delete_lines_and_cancel() {
    let sc = run(3, 3, b"a\r\nb\r\nc\x1b[2;1H\x1b[L");
    assert_eq!((sc.row_text(1), sc.row_text(2)), ("".into(), "b".into()));
    let sc = run(3, 3, b"a\r\nb\r\nc\x1b[2;1H\x1b[M");
    assert_eq!((sc.row_text(1), sc.row_text(2)), ("c".into(), "".into()));
    let sc = run(6, 1, b"ab\x1b[12\x18cd");
    assert_eq!(sc.row_text(0), "abcd");
}
