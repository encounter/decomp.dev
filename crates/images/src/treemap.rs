use palette::{FromColor, Hsl, Mix, Srgb};
use streemap::Rect;

pub fn layout_units<T, S, R>(items: &mut [T], aspect: f32, size_fn: S, mut set_rect_fn: R)
where
    S: Fn(&T) -> f32,
    R: FnMut(&mut T, Rect<f32>),
{
    let rect = if aspect > 1.0 {
        Rect::from_size(1.0, 1.0 / aspect)
    } else {
        Rect::from_size(aspect, 1.0)
    };
    streemap::binary(rect, items, size_fn, |item, mut rect| {
        if aspect > 1.0 {
            rect.y *= aspect;
            rect.h *= aspect;
        } else {
            rect.x /= aspect;
            rect.w /= aspect;
        }
        set_rect_fn(item, rect);
    });
}

pub fn hsl(h: u16, s: u8, l: u8) -> Srgb {
    let hsl = Hsl::new(h as f32, s as f32 / 100.0, l as f32 / 100.0);
    Srgb::from_color(hsl)
}

pub fn color_mix(c1: Srgb, c2: Srgb, percent: f32) -> Srgb { c1.mix(c2, percent) }

pub fn unit_color(fuzzy_match_percent: f32) -> String {
    html_color(if fuzzy_match_percent == 100.0 {
        hsl(120, 100, 39)
    } else {
        let nonmatch = hsl(221, 0, 21);
        let nearmatch = hsl(221, 100, 35);
        nonmatch.mix(nearmatch, fuzzy_match_percent / 100.0)
    })
}

pub fn html_color(c: Srgb) -> String {
    let (r, g, b) = c.into_components();
    format!("#{:02x}{:02x}{:02x}", (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}
