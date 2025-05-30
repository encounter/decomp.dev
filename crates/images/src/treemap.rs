use palette::{Mix, Srgb};
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
    streemap::ordered_pivot_by_middle(rect, items, size_fn, |item, mut rect| {
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

fn rgb(r: u8, g: u8, b: u8) -> Srgb {
    Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

pub fn unit_color(fuzzy_match_percent: f32) -> String {
    let red = rgb(42, 49, 64);
    let green = rgb(0, 200, 0);
    let (r, g, b) = red.mix(green, fuzzy_match_percent / 100.0).into_components();
    format!("#{:02x}{:02x}{:02x}", (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}
