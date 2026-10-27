use loos_kms::commit::*;
use loos_kms::edid::*;
use loos_kms::layout::*;
use loos_kms::mode::*;

/// Build a valid 128-byte EDID with a 1920x1080@60 DTD (CEA-861 timing) named "TESTMON".
fn edid_1080p() -> [u8; 128] {
    let mut b = [0u8; 128];
    b[..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
    // manufacturer "LOO": L=12,O=15,O=15 -> 01100 01111 01111
    let id: u16 = (12 << 10) | (15 << 5) | 15;
    b[8..10].copy_from_slice(&id.to_be_bytes());
    b[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
    b[18] = 1;
    b[19] = 4;
    b[20] = 0x80; // digital
    b[21] = 53; // 530 mm
    b[22] = 30; // 300 mm
    b[24] = 0x02; // preferred timing in first DTD
    b[35] = 0b0010_0001; // 640x480@60 + 720x400@70? bit7=720x400@70, bit5=640x480@60
    b[36] = 0b0000_1000; // 1024x768@60 is bit 3 of byte36? (bit 15-12 => index 12 => bit 3)
    for s in 0..8 {
        b[38 + s * 2] = 1;
        b[39 + s * 2] = 1;
    }
    b[38] = (1280 / 8 - 31) as u8; // 1280 wide
    b[39] = (3 << 6) | 0; // 16:9, 60 Hz -> 1280x720
    // DTD: 148.5 MHz, 1920/88/44/148, 1080/4/5/36, +hs +vs
    let d = &mut b[54..72];
    d[..2].copy_from_slice(&14850u16.to_le_bytes());
    d[2] = (1920 & 0xFF) as u8;
    let hbl = 280u32; // 88+44+148
    d[3] = (hbl & 0xFF) as u8;
    d[4] = (((1920 >> 8) << 4) | (hbl >> 8)) as u8;
    d[5] = (1080 & 0xFF) as u8;
    let vbl = 45u32;
    d[6] = vbl as u8;
    d[7] = (((1080 >> 8) << 4) | (vbl >> 8)) as u8;
    d[8] = 88;
    d[9] = 44;
    d[10] = (4 << 4) | 5;
    d[11] = 0;
    d[12] = (520 & 0xFF) as u8;
    d[13] = (290 & 0xFF) as u8;
    d[14] = (((520 >> 8) << 4) | (290 >> 8)) as u8;
    d[17] = 0x1E; // digital separate, +vsync, +hsync
    // monitor name descriptor
    let n = &mut b[72..90];
    n[3] = 0xFC;
    n[5..13].copy_from_slice(b"TESTMON\n");
    for x in &mut n[13..18] {
        *x = 0x20;
    }
    let sum = b[..127].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    b[127] = 0u8.wrapping_sub(sum);
    b
}

#[test]
fn edid_parse() {
    let raw = edid_1080p();
    let e = Edid::parse(&raw).unwrap();
    assert_eq!((e.manufacturer_str().as_str(), e.product, e.version, e.digital), ("LOO", 0x1234, (1, 4), true));
    assert_eq!(e.name, "TESTMON");
    let m = e.detailed[0];
    assert_eq!((m.hdisplay, m.vdisplay, m.htotal, m.vtotal, m.clock), (1920, 1080, 2200, 1125, 148_500));
    assert!(m.preferred && m.valid());
    assert_eq!(m.refresh_mhz(), 60_000);
    assert_eq!(m.name(), "1920x1080");
    assert_eq!(m.flags & (flags::PHSYNC | flags::PVSYNC), flags::PHSYNC | flags::PVSYNC);
    assert_eq!(e.dpi_x(), Some(92)); // 1920 px over the 530 mm panel width
    let modes = e.modes();
    assert!(modes[0].preferred);
    assert!(modes.iter().any(|m| m.hdisplay == 1280 && m.vdisplay == 720));
    assert!(modes.iter().any(|m| m.hdisplay == 640 && m.vdisplay == 480));
    assert!(modes.iter().any(|m| m.hdisplay == 1024 && m.vdisplay == 768));
    // corrupt inputs
    assert_eq!(Edid::parse(&raw[..100]), Err(Error::Short));
    let mut bad = raw;
    bad[0] = 1;
    assert_eq!(Edid::parse(&bad), Err(Error::BadHeader));
    let mut bad = raw;
    bad[30] ^= 1;
    assert_eq!(Edid::parse(&bad), Err(Error::BadChecksum));
    // fuzz: never panics
    for i in 0..128 {
        for v in [0u8, 0xFF, 0x55] {
            let mut f = raw;
            f[i] = v;
            let sum = f[..127].iter().fold(0u8, |a, &x| a.wrapping_add(x));
            f[127] = 0u8.wrapping_sub(sum);
            let _ = Edid::parse(&f).map(|e| e.modes());
        }
    }
}

#[test]
fn cvt_matches_published_values() {
    let m = cvt_rb(1920, 1080, 60).unwrap();
    // VESA CVT-RB 1920x1080@60: htotal 2080, vtotal 1111, clock 138.5 MHz
    assert_eq!((m.htotal, m.vtotal), (2080, 1111));
    assert!((138_000..=139_000).contains(&m.clock), "{}", m.clock);
    assert!(m.valid() && (59_900..=60_100).contains(&m.refresh_mhz()));
    let m = cvt_rb(1024, 768, 60).unwrap();
    assert_eq!((m.htotal, m.vtotal, m.vsync_end - m.vsync_start), (1184, 790, 4)); // matches `cvt -r 1024 768 60`; 4:3 -> 4 sync lines
    assert!(cvt_rb(10, 10, 60).is_none() && cvt_rb(1920, 1080, 1000).is_none());
}

#[test]
fn pick_modes() {
    let e = Edid::parse(&edid_1080p()).unwrap();
    let modes = e.modes();
    assert_eq!(pick(&modes, Request::Preferred).unwrap().name(), "1920x1080");
    assert_eq!(pick(&modes, Request::Size { w: 1024, h: 768, max_hz: 0 }).unwrap().vdisplay, 768);
    assert!(pick(&modes, Request::Size { w: 3840, h: 2160, max_hz: 0 }).is_none());
    // no preferred flag -> largest progressive wins
    let mut np = modes.clone();
    for m in &mut np {
        m.preferred = false;
    }
    assert_eq!(pick(&np, Request::Preferred).unwrap().name(), "1920x1080");
    // refresh cap picks the lower rate
    let a = cvt_rb(1280, 720, 60).unwrap();
    let b = cvt_rb(1280, 720, 75).unwrap();
    assert_eq!(pick(&[a, b], Request::Size { w: 1280, h: 720, max_hz: 60 }).unwrap(), a);
    assert_eq!(pick(&[a, b], Request::Size { w: 1280, h: 720, max_hz: 0 }).unwrap(), b);
    let mut v = vec![a, a, b];
    sort_and_dedup(&mut v);
    assert_eq!(v.len(), 2);
}

fn caps() -> Caps {
    Caps { crtcs: 2, max_dim: 8192, pitch_align: 64, max_pixel_rate: 600_000_000, scaling: false, possible_crtcs: &[0b11, 0b11, 0b10] }
}

fn st(crtc: u8, conn: u8, mode: Mode) -> CrtcState {
    let fb = Fb { w: mode.hdisplay as u32, h: mode.vdisplay as u32, pitch: mode.hdisplay as u32 * 4, format: Format::Xrgb8888, size: mode.hdisplay as u64 * 4 * mode.vdisplay as u64 };
    let r = Rect { x: 0, y: 0, w: fb.w, h: fb.h };
    CrtcState { crtc, mode: Some(mode), fb: Some(fb), src: r, dst: r, connector: conn }
}

#[test]
fn commit_validation() {
    let m = cvt_rb(1920, 1080, 60).unwrap();
    assert_eq!(check(&caps(), &[st(0, 0, m), st(1, 1, m)]), Ok(()));
    assert_eq!(check(&caps(), &[st(2, 0, m)]), Err(Reject::NoSuchCrtc(2)));
    assert_eq!(check(&caps(), &[st(0, 2, m)]), Err(Reject::Routing(0))); // connector 2 only reaches crtc 1
    assert_eq!(check(&caps(), &[st(0, 0, m), st(0, 1, m)]), Err(Reject::Routing(0)));
    let mut s = st(0, 0, m);
    s.fb.as_mut().unwrap().pitch = 100;
    assert_eq!(check(&caps(), &[s]), Err(Reject::BadPitch(0)));
    let mut s = st(0, 0, m);
    s.fb.as_mut().unwrap().size -= 1;
    assert_eq!(check(&caps(), &[s]), Err(Reject::BadPitch(0)));
    let mut s = st(0, 0, m);
    s.src.w += 1;
    assert_eq!(check(&caps(), &[s]), Err(Reject::BadRect(0)));
    let mut s = st(0, 0, m);
    s.dst = Rect { x: 0, y: 0, w: 960, h: 540 };
    assert_eq!(check(&caps(), &[s]), Err(Reject::NoScaling(0)));
    let mut c = caps();
    c.scaling = true;
    assert_eq!(check(&c, &[s]), Ok(()));
    let mut s = st(0, 0, m);
    s.fb = None;
    assert_eq!(check(&caps(), &[s]), Err(Reject::BadFb(0)));
    let big = cvt_rb(3840, 2160, 60).unwrap();
    assert_eq!(check(&caps(), &[st(0, 0, big), st(1, 1, big)]), Err(Reject::Bandwidth));
    let mut off = st(0, 0, m);
    off.mode = None;
    assert_eq!(check(&caps(), &[off]), Ok(()));
}

#[test]
fn crtc_assignment_uses_augmenting_paths() {
    // connector 0 can use crtc0|1, connector 2 only crtc1: a greedy pass gives crtc0 to conn0, crtc1 to conn2.
    let mut c = caps();
    let a = assign_crtcs(&c, &[2, 0]);
    assert_eq!(a, vec![Some(1), Some(0)]);
    // greedy would put conn0 on crtc0 first only if listed first; reverse order forces an augment
    let a = assign_crtcs(&c, &[0, 2]);
    assert_eq!(a, vec![Some(0), Some(1)]);
    c.possible_crtcs = &[0b01, 0b01];
    assert_eq!(assign_crtcs(&c, &[0, 1]), vec![Some(0), None]); // only one CRTC reachable by both
}

#[test]
fn desktop_layout() {
    let l = auto_layout(&[(0, 1920, 1080, 100), (1, 2560, 1440, 150)]);
    assert_eq!((l[1].x, l[0].primary, l[1].primary), (1920, true, false));
    assert_eq!(validate(&l), Ok(()));
    assert_eq!(bounds(&l), Some((0, 0, 4480, 1440)));
    assert_eq!(at(&l, 2000, 100).unwrap().id, 1);
    assert!(at(&l, 100, 1200).is_none()); // below the shorter first monitor
    assert_eq!(clamp_cursor(&l, 100, 1200), (100, 1079));
    assert_eq!(clamp_cursor(&l, 99999, 0), (4479, 0));
    let mut bad = l.clone();
    bad[1].x = 1000;
    assert_eq!(validate(&bad), Err(Problem::Overlap(0, 1)));
    bad[1].x = 5000;
    assert_eq!(validate(&bad), Err(Problem::Isolated(0)));
    bad[1].primary = true;
    assert_eq!(validate(&bad), Err(Problem::Primary));
    assert_eq!((suggest_scale(96), suggest_scale(120), suggest_scale(192), suggest_scale(160)), (100, 125, 200, 175));
}
