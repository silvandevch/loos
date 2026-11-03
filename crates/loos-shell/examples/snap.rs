//! Renders the desktop headlessly to PPM files, one per Look, for review and docs.
//! `cargo run -p loos-shell --example snap -- <out-dir> [width height]`
//! Output: `<out-dir>/<look-id>.ppm` (desktop with Files, Task Manager and Settings > Looks open).
use loos_shell::desktop::Desktop;
use loos_shell::look::LOOKS;
use std::ffi::c_void;
use std::io::Write;

unsafe extern "C" fn alloc(bytes: usize, _aux: *mut c_void) -> *mut c_void {
    let mut v = vec![0u32; bytes / 4];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p as *mut c_void
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = a.get(1).map(String::as_str).unwrap_or(".");
    let (w, h): (i32, i32) = (a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1280), a.get(3).and_then(|s| s.parse().ok()).unwrap_or(720));
    std::fs::create_dir_all(out).unwrap();
    let mut back = vec![0u32; (w * h) as usize];
    let mut d = Desktop::boxed();
    assert!(d.init(back.as_mut_ptr(), w, h, 0, alloc, std::ptr::null_mut()));
    if a.iter().any(|x| x == "--pages") {
        // Settings maximized on each new page, other apps closed.
        d.aw[0].close();
        d.aw[2].close();
        d.comp.toggle_maximize(d.aw[1].win);
        for (n, pg) in [loos_shell::settings::PG_LOOKS, loos_shell::settings::PG_BEHAVIOR].into_iter().enumerate() {
            d.settings.page = pg;
            d.aw[1].dirty = true;
            for t in 0..4 {
                d.tick(1000 + t * 50, 9, 41);
                d.render();
            }
            d.render_all();
            let path = format!("{out}/page{n}.ppm");
            let mut f = std::fs::File::create(&path).unwrap();
            write!(f, "P6\n{w} {h}\n255\n").unwrap();
            let mut rgb = Vec::new();
            for p in &back {
                rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
            }
            f.write_all(&rgb).unwrap();
            println!("{path}");
        }
        return;
    }
    for (i, l) in LOOKS.iter().enumerate() {
        let text = format!("ui.look={}\n", l.id);
        let _ = text;
        // Apply through the same path the Settings card uses.
        loos_shell::look::store_look(&d.be, l);
        loos_shell::settings::apply(&mut d.settings);
        d.settings.changes += 1;
        for t in 0..4 {
            d.tick(1000 + t * 50, 9, 41);
            d.render();
        }
        d.render_all();
        let path = format!("{out}/{i}-{}.ppm", l.id);
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "P6\n{w} {h}\n255\n").unwrap();
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for p in &back {
            rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        f.write_all(&rgb).unwrap();
        println!("{path}");
    }
}
