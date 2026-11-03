//! Visual language for LoOS Switch: quiet dark surfaces, one blue accent,
//! generous spacing, and a single obvious next action per screen.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, RichText, Stroke, Vec2};

pub const BG: Color32 = Color32::from_rgb(8, 12, 18);
pub const SIDEBAR: Color32 = Color32::from_rgb(12, 18, 28);
pub const TOPBAR: Color32 = Color32::from_rgb(10, 15, 23);
pub const CARD: Color32 = Color32::from_rgb(18, 27, 42);
pub const CARD_ALT: Color32 = Color32::from_rgb(22, 33, 51);
pub const CARD_HOVER: Color32 = Color32::from_rgb(26, 39, 60);
pub const BORDER: Color32 = Color32::from_rgb(36, 49, 73);
pub const TEXT: Color32 = Color32::from_rgb(243, 246, 251);
pub const MUTED: Color32 = Color32::from_rgb(145, 161, 184);
pub const DIM: Color32 = Color32::from_rgb(103, 119, 141);
pub const ACCENT: Color32 = Color32::from_rgb(59, 130, 246);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(96, 165, 250);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(27, 55, 102);
pub const SUCCESS: Color32 = Color32::from_rgb(52, 211, 153);
pub const SUCCESS_DIM: Color32 = Color32::from_rgb(16, 66, 53);
pub const WARN: Color32 = Color32::from_rgb(251, 191, 36);
pub const WARN_DIM: Color32 = Color32::from_rgb(74, 58, 17);
pub const DANGER: Color32 = Color32::from_rgb(248, 113, 113);
pub const DANGER_DIM: Color32 = Color32::from_rgb(80, 28, 33);

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), FontData::from_static(bytes).into());
    };
    add("inter", FONT_REGULAR);
    add("inter-medium", FONT_MEDIUM);
    add("inter-semibold", FONT_SEMIBOLD);
    add("inter-bold", FONT_BOLD);
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".into());
    fonts.families.insert(
        FontFamily::Name("medium".into()),
        vec!["inter-medium".into(), "inter".into()],
    );
    fonts.families.insert(
        FontFamily::Name("semibold".into()),
        vec!["inter-semibold".into(), "inter".into()],
    );
    fonts.families.insert(
        FontFamily::Name("bold".into()),
        vec!["inter-bold".into(), "inter-semibold".into(), "inter".into()],
    );
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = Color32::from_rgb(8, 13, 22);
    style.visuals.faint_bg_color = CARD;
    style.visuals.widgets.noninteractive.bg_fill = CARD;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    style.visuals.widgets.inactive.bg_fill = CARD_ALT;
    style.visuals.widgets.inactive.weak_bg_fill = CARD_ALT;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.hovered.bg_fill = CARD_HOVER;
    style.visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.active.bg_fill = ACCENT_DIM;
    style.visuals.widgets.active.weak_bg_fill = ACCENT_DIM;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.selection.bg_fill = ACCENT_DIM;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT_HOVER);
    style.visuals.window_corner_radius = egui::CornerRadius::same(14);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(10);
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };
    style.spacing.item_spacing = Vec2::new(12.0, 10.0);
    style.spacing.button_padding = Vec2::new(16.0, 10.0);
    style.spacing.interact_size = Vec2::new(44.0, 38.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.text_styles.insert(egui::TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
    style.text_styles.insert(egui::TextStyle::Button, FontId::new(15.0, FontFamily::Name("medium".into())));
    style.text_styles.insert(egui::TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
    style.text_styles.insert(egui::TextStyle::Heading, FontId::new(24.0, FontFamily::Name("semibold".into())));
    style.text_styles.insert(egui::TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    ctx.set_style(style);
}

pub fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(14)
        .inner_margin(Margin::same(20))
}

pub fn card_alt() -> Frame {
    Frame::new()
        .fill(CARD_ALT)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(16))
}

pub fn inset() -> Frame {
    Frame::new()
        .fill(Color32::from_rgb(12, 19, 31))
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(29, 42, 63)))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
}

pub fn title(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(32.0, FontFamily::Name("bold".into()))).color(TEXT)
}

pub fn h1(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(24.0, FontFamily::Name("semibold".into()))).color(TEXT)
}

pub fn h2(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(17.5, FontFamily::Name("semibold".into()))).color(TEXT)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(14.0, FontFamily::Proportional)).color(MUTED)
}

pub fn small(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(12.5, FontFamily::Proportional)).color(DIM)
}

pub fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    Frame::new()
        .fill(bg)
        .corner_radius(100)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(FontId::new(12.0, FontFamily::Name("medium".into()))).color(fg));
        });
}

pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).font(FontId::new(15.5, FontFamily::Name("semibold".into()))).color(Color32::WHITE))
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(10)
        .min_size(Vec2::new(0.0, 44.0))
}

pub fn secondary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).font(FontId::new(15.0, FontFamily::Name("medium".into()))).color(TEXT))
        .fill(CARD_ALT)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(10)
        .min_size(Vec2::new(0.0, 42.0))
}

pub fn danger_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).font(FontId::new(15.5, FontFamily::Name("semibold".into()))).color(Color32::WHITE))
        .fill(Color32::from_rgb(185, 51, 63))
        .stroke(Stroke::NONE)
        .corner_radius(10)
        .min_size(Vec2::new(0.0, 44.0))
}

pub fn logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::same((size * 0.28) as u8), ACCENT);
    let inner = rect.shrink(size * 0.19);
    p.circle_filled(inner.center(), size * 0.16, Color32::from_rgb(219, 234, 254));
    p.circle_stroke(inner.center(), size * 0.29, Stroke::new(size * 0.055, Color32::from_rgb(191, 219, 254)));
    p.line_segment(
        [inner.center() + Vec2::new(size * 0.08, -size * 0.22), inner.center() + Vec2::new(size * 0.23, size * 0.12)],
        Stroke::new(size * 0.05, Color32::WHITE),
    );
}

pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), ui.cursor().top(), Stroke::new(1.0_f32, BORDER));
    ui.add_space(13.0);
}

pub fn warning_box(ui: &mut egui::Ui, title: &str, text: &str) {
    Frame::new()
        .fill(WARN_DIM)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(120, 92, 24)))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.label(RichText::new(title).font(FontId::new(14.0, FontFamily::Name("semibold".into()))).color(WARN));
            ui.add_space(3.0);
            ui.label(RichText::new(text).size(13.5).color(Color32::from_rgb(231, 213, 168)));
        });
}

pub fn danger_box(ui: &mut egui::Ui, title: &str, text: &str) {
    Frame::new()
        .fill(DANGER_DIM)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(120, 48, 56)))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.label(RichText::new(title).font(FontId::new(14.0, FontFamily::Name("semibold".into()))).color(DANGER));
            ui.add_space(3.0);
            ui.label(RichText::new(text).size(13.5).color(Color32::from_rgb(241, 197, 200)));
        });
}

pub fn success_box(ui: &mut egui::Ui, title: &str, text: &str) {
    Frame::new()
        .fill(SUCCESS_DIM)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 107, 84)))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.label(RichText::new(title).font(FontId::new(14.0, FontFamily::Name("semibold".into()))).color(SUCCESS));
            ui.add_space(3.0);
            ui.label(RichText::new(text).size(13.5).color(Color32::from_rgb(186, 237, 218)));
        });
}

pub fn danger(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(13.0, FontFamily::Proportional)).color(DANGER)
}

pub fn monospace(text: impl Into<String>) -> RichText {
    RichText::new(text).font(FontId::new(12.5, FontFamily::Monospace)).color(MUTED)
}
