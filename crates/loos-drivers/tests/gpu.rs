use loos_drivers::gpu::*;
use loos_drivers::hw::Mmio;
use std::collections::HashMap;

struct Regs(HashMap<usize, u16>);
impl Mmio for Regs {
    fn read32(&mut self, _: usize) -> u32 { 0 }
    fn write32(&mut self, _: usize, _: u32) {}
    fn read16(&mut self, off: usize) -> u16 { *self.0.get(&off).unwrap_or(&0) }
    fn write16(&mut self, off: usize, v: u16) {
        // The adapter clamps absurd sizes like real hardware.
        let v = if off == 0x500 + 2 * 1 && v > 4096 { 4096 } else { v };
        self.0.insert(off, v);
    }
    fn read8(&mut self, _: usize) -> u8 { 0 }
    fn write8(&mut self, _: usize, _: u8) {}
}

fn edid(w: u32, h: u32, name: &str) -> [u8; 128] {
    let mut e = [0u8; 128];
    e[0..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    let m: u16 = (7 << 10) | (5 << 5) | 14; // "GEN"
    e[8..10].copy_from_slice(&m.to_be_bytes());
    e[35] = 0x21; // 640x480 + 800x600
    e[36] = 0x08; // 1024x768
    e[54..56].copy_from_slice(&15000u16.to_le_bytes());
    e[56] = w as u8;
    e[58] = ((w >> 8) as u8) << 4;
    e[59] = h as u8;
    e[61] = ((h >> 8) as u8) << 4;
    e[72 + 3] = 0xfc;
    let n = name.as_bytes();
    e[77..77 + n.len()].copy_from_slice(n);
    e[77 + n.len()] = b'\n';
    let sum = e[..127].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    e[127] = 0u8.wrapping_sub(sum);
    e
}

#[test]
fn edid_parse() {
    let e = parse_edid(&edid(1920, 1080, "LoMon")).unwrap();
    assert_eq!((e.vendor.as_str(), e.name.as_str()), ("GEN", "LoMon"));
    assert_eq!(e.preferred, Some(Mode { w: 1920, h: 1080 }));
    assert_eq!(e.established, [Mode { w: 640, h: 480 }, Mode { w: 800, h: 600 }, Mode { w: 1024, h: 768 }]);
    let mut bad = edid(1920, 1080, "x");
    bad[20] ^= 1;
    assert!(parse_edid(&bad).is_err());
    assert!(parse_edid(&[0; 128]).is_err());
}

#[test]
fn mode_selection() {
    let sup = [Mode { w: 800, h: 600 }, Mode { w: 1280, h: 720 }, Mode { w: 1920, h: 1080 }];
    let e = parse_edid(&edid(1920, 1080, "a")).unwrap();
    assert_eq!(select_mode(&sup, Some(&e)), Some(Mode { w: 1920, h: 1080 }));
    let e = parse_edid(&edid(1366, 768, "a")).unwrap();
    assert_eq!(select_mode(&sup, Some(&e)), Some(Mode { w: 1280, h: 720 }), "largest that fits the native size");
    assert_eq!(select_mode(&sup, None), Some(Mode { w: 1920, h: 1080 }));
    assert_eq!(select_mode(&[], None), None);
}

#[test]
fn bochs_modeset_and_flip() {
    let mut r = Regs(HashMap::new());
    r.0.insert(0x500, 0xb0c5);
    r.0.insert(0x500 + 20, 256); // 16 MiB
    let mut g = BochsVga::new(r, 0x500).unwrap();
    assert!(g.modes().contains(&Mode { w: 1920, h: 1080 }));
    assert!(g.set_mode(Mode { w: 1234, h: 567 }).is_err());
    assert!(g.flip(0).is_err(), "no mode yet");
    let info = g.set_mode(Mode { w: 1280, h: 720 }).unwrap();
    assert_eq!((info.width, info.height, info.pitch), (1280, 720, 5120));
    g.flip(1).unwrap();
    assert!(g.flip(2).is_err());
    assert!(BochsVga::new(Regs(HashMap::new()), 0x500).is_err(), "wrong id");
    let mut small = Regs(HashMap::new());
    small.0.insert(0x500, 0xb0c0);
    small.0.insert(0x500 + 20, 64); // 4 MiB: no room for 2 pages of 1080p
    let g = BochsVga::new(small, 0x500).unwrap();
    assert!(!g.modes().contains(&Mode { w: 1920, h: 1080 }));
}
