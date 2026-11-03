use loos_compositor::d3d11::*;
use loos_compositor::{CompError, Compositor, Rect};

fn frame(c: &mut Compositor) -> (Vec<u32>, Vec<Rect>) {
    let (w, h) = c.size();
    let mut out = vec![0xdead_beef; (w * h) as usize];
    let r = c.compose(&mut out, w as usize);
    (out, r)
}

fn solid(c: &mut Compositor, x: i32, y: i32, w: i32, h: i32, argb: u32) -> u32 {
    let id = c.create_surface(x, y, w, h).unwrap();
    c.commit(id, &vec![argb; (w * h) as usize], None).unwrap();
    id
}

#[test]
fn first_frame_paints_everything_then_only_damage() {
    let mut c = Compositor::new(64, 48);
    c.background = 0x112233;
    let (px, r) = frame(&mut c);
    assert_eq!(r, [Rect::new(0, 0, 64, 48)]);
    assert!(px.iter().all(|&p| p == 0x112233));
    let (_, r) = frame(&mut c);
    assert!(r.is_empty(), "nothing changed, nothing repainted");
    let id = solid(&mut c, 10, 10, 8, 8, 0xffff0000);
    let (px, r) = frame(&mut c);
    assert_eq!(r, [Rect::new(10, 10, 8, 8)]);
    assert_eq!(px[12 * 64 + 12], 0xff0000);
    assert_eq!(px[0], 0xdead_beef, "untouched area is not rewritten");
    c.move_to(id, 20, 20).unwrap();
    let (_, r) = frame(&mut c);
    assert_eq!(r.len(), 2);
}

#[test]
fn z_order_blend_and_opacity() {
    let mut c = Compositor::new(32, 32);
    c.background = 0;
    let a = solid(&mut c, 0, 0, 16, 16, 0xff0000ff);
    let b = solid(&mut c, 8, 8, 16, 16, 0x80ff0000); // 50% red over blue
    let (px, _) = frame(&mut c);
    assert_eq!(px[4 * 32 + 4], 0x0000ff);
    let m = px[10 * 32 + 10];
    assert!((m >> 16) & 255 >= 126 && (m >> 16) & 255 <= 129 && m & 255 >= 126 && m & 255 <= 129, "{m:#x}");
    assert_eq!(px[20 * 32 + 20] >> 16 & 255, 128, "over background");
    c.raise(a).unwrap();
    assert_eq!(c.stack(), [b, a]);
    assert_eq!(c.surface_at(10, 10), Some(a));
    c.set_visible(a, false).unwrap();
    assert_eq!(c.surface_at(10, 10), Some(b));
    c.set_opacity(b, 0).unwrap();
    let (px, _) = frame(&mut c);
    assert_eq!(px[10 * 32 + 10], 0);
}

#[test]
fn partial_commit_clips_and_errors() {
    let mut c = Compositor::new(16, 16);
    frame(&mut c);
    let id = c.create_surface(2, 2, 4, 4).unwrap();
    assert_eq!(c.commit(id, &[0; 3], None), Err(CompError::BadSize));
    assert_eq!(c.commit(99, &[0; 16], None), Err(CompError::NoSurface));
    c.commit(id, &[0xffffffff; 16], Some(Rect::new(1, 1, 2, 2))).unwrap();
    let (_, r) = frame(&mut c);
    assert_eq!(r, [Rect::new(3, 3, 2, 2)]);
    c.destroy_surface(id).unwrap();
    let (px, _) = frame(&mut c);
    assert_eq!(px[3 * 16 + 3], c_bg());
}
fn c_bg() -> u32 {
    0x0020_3040
}

#[test]
fn offscreen_surfaces_and_cursor() {
    let mut c = Compositor::new(20, 20);
    frame(&mut c);
    solid(&mut c, -5, -5, 10, 10, 0xff00ff00);
    solid(&mut c, 18, 18, 10, 10, 0xff00ff00);
    let (px, _) = frame(&mut c);
    assert_eq!(px[0], 0x00ff00);
    assert_eq!(px[19 * 20 + 19], 0x00ff00);
    c.set_cursor(Some((5, 5)));
    let (px, _) = frame(&mut c);
    assert_eq!(px[5 * 20 + 5], 0, "cursor outline");
    assert_eq!(px[7 * 20 + 6], 0xffffff, "cursor fill");
    c.set_cursor(None);
    let (px, _) = frame(&mut c);
    assert_eq!(px[5 * 20 + 5], c_bg());
}

#[test]
fn d3d11_clear_present_flip() {
    let mut c = Compositor::new(32, 32);
    c.background = 0;
    let ctx = Context;
    let mut sc = SwapChain::new(&mut c, 4, 4, 8, 8, Format::B8G8R8X8, 2, SwapEffect::FlipSequential).unwrap();
    ctx.clear_render_target(sc.back_buffer(), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(sc.present(&mut c, 1), hr::S_OK);
    ctx.clear_render_target(sc.back_buffer(), [0.0, 1.0, 0.0, 1.0]);
    let (px, _) = frame(&mut c);
    assert_eq!(px[5 * 32 + 5], 0xff0000);
    assert_eq!(sc.present(&mut c, 0), hr::S_OK);
    let (px, _) = frame(&mut c);
    assert_eq!(px[5 * 32 + 5], 0x00ff00);
    assert_eq!(sc.present_count(), 2);
    c.set_visible(sc.surface(), false).unwrap();
    assert_eq!(sc.present(&mut c, 0), hr::DXGI_STATUS_OCCLUDED);
    assert_eq!(sc.resize(&mut c, 16, 16), hr::S_OK);
    assert_eq!(sc.back_buffer().w, 16);
    let s = sc.surface();
    sc.destroy(&mut c);
    assert!(c.surface(s).is_none());
}

#[test]
fn d3d11_formats_and_copies() {
    let ctx = Context;
    let mut rgba = Texture2D::new(4, 4, Format::R8G8B8A8).unwrap();
    ctx.update_subresource(&mut rgba, Rect::new(0, 0, 2, 1), &[0xff0000ff, 0x80_00_ff_00], 2).unwrap(); // R8G8B8A8: r=ff then g=ff
    assert_eq!(rgba.pixels()[0], 0xffff0000, "R channel lands in the red byte");
    assert_eq!(rgba.get(0, 0), Some(0xff0000ff));
    assert_eq!(rgba.get(1, 0), Some(0x8000ff00));
    let mut dst = Texture2D::new(4, 4, Format::R8G8B8A8).unwrap();
    ctx.copy_subresource_region(&mut dst, 2, 2, &rgba, Rect::new(0, 0, 2, 1)).unwrap();
    assert_eq!(dst.get(2, 2), rgba.get(0, 0));
    assert!(ctx.copy_subresource_region(&mut dst, 3, 3, &rgba, Rect::new(0, 0, 2, 2)).is_err());
    assert!(ctx.copy_resource(&mut dst, &Texture2D::new(2, 2, Format::R8G8B8A8).unwrap()).is_err());
    ctx.copy_resource(&mut dst, &rgba).unwrap();
    assert_eq!(dst, rgba);
    assert_eq!(Texture2D::new(0, 4, Format::B8G8R8A8), Err(hr::E_INVALIDARG));
    assert!(SwapChain::new(&mut Compositor::new(8, 8), 0, 0, 4, 4, Format::B8G8R8A8, 0, SwapEffect::Discard).is_err());
}
