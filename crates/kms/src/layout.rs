//! Multi-monitor desktop layout.

use alloc::vec::Vec;

/// One monitor placed on the virtual desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// Stable id (connector index).
    pub id: u8,
    /// Left edge on the desktop.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// UI scale percent (100, 125, 150, 200 ...).
    pub scale: u32,
    /// Primary (taskbar) monitor.
    pub primary: bool,
}

impl Monitor {
    fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }
    fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }
    fn overlaps(&self, o: &Monitor) -> bool {
        (self.x as i64) < o.right() && (o.x as i64) < self.right() && (self.y as i64) < o.bottom() && (o.y as i64) < self.bottom()
    }
}

/// Automatic left-to-right layout in id order; the first is primary and sits at (0,0). Monitors of
/// different heights are top-aligned.
pub fn auto_layout(sizes: &[(u8, u32, u32, u32)]) -> Vec<Monitor> {
    let mut x = 0i64;
    let mut v = Vec::new();
    for (i, &(id, w, h, scale)) in sizes.iter().enumerate() {
        v.push(Monitor { id, x: x as i32, y: 0, w, h, scale: scale.max(100), primary: i == 0 });
        x += w as i64;
    }
    v
}

/// Layout problems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Two monitors overlap.
    Overlap(u8, u8),
    /// A monitor touches no other (cursor could not cross to it).
    Isolated(u8),
    /// Not exactly one primary.
    Primary,
}

/// Check a hand-made layout.
pub fn validate(m: &[Monitor]) -> Result<(), Problem> {
    if m.iter().filter(|x| x.primary).count() != 1 && !m.is_empty() {
        return Err(Problem::Primary);
    }
    for i in 0..m.len() {
        for j in i + 1..m.len() {
            if m[i].overlaps(&m[j]) {
                return Err(Problem::Overlap(m[i].id, m[j].id));
            }
        }
    }
    if m.len() > 1 {
        for a in m {
            let touches = m.iter().any(|b| {
                b.id != a.id
                    && ((a.right() == b.x as i64 || b.right() == a.x as i64) && (a.y as i64) < b.bottom() && (b.y as i64) < a.bottom()
                        || (a.bottom() == b.y as i64 || b.bottom() == a.y as i64) && (a.x as i64) < b.right() && (b.x as i64) < a.right())
            });
            if !touches {
                return Err(Problem::Isolated(a.id));
            }
        }
    }
    Ok(())
}

/// Monitor containing desktop point (x,y).
pub fn at(m: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    m.iter().find(|o| x >= o.x && y >= o.y && (x as i64) < o.right() && (y as i64) < o.bottom())
}

/// Move a cursor to (x,y); if outside every monitor it is clamped to the nearest point on one.
pub fn clamp_cursor(m: &[Monitor], x: i32, y: i32) -> (i32, i32) {
    if at(m, x, y).is_some() || m.is_empty() {
        return (x, y);
    }
    let mut best = (i64::MAX, (x, y));
    for o in m {
        let cx = (x as i64).clamp(o.x as i64, o.right() - 1);
        let cy = (y as i64).clamp(o.y as i64, o.bottom() - 1);
        let d = (cx - x as i64).pow(2) + (cy - y as i64).pow(2);
        if d < best.0 {
            best = (d, (cx as i32, cy as i32));
        }
    }
    best.1
}

/// Suggested UI scale (percent, snapped to 100/125/150/175/200/250/300) from physical DPI.
pub fn suggest_scale(dpi: u32) -> u32 {
    const STEPS: [u32; 7] = [100, 125, 150, 175, 200, 250, 300];
    let want = dpi * 100 / 96;
    *STEPS.iter().min_by_key(|&&s| s.abs_diff(want)).unwrap()
}

/// Bounding box of the whole desktop.
pub fn bounds(m: &[Monitor]) -> Option<(i32, i32, u32, u32)> {
    let x0 = m.iter().map(|o| o.x as i64).min()?;
    let y0 = m.iter().map(|o| o.y as i64).min()?;
    let x1 = m.iter().map(|o| o.right()).max()?;
    let y1 = m.iter().map(|o| o.bottom()).max()?;
    Some((x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
}
