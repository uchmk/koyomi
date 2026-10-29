//! The app's icon: a calendar page on a dark tile, its header band cyan,
//! six days below it as dots, and today's day gold (the day a run is on).
//! Drawn here at any size so no image file is carried; build.rs includes
//! this file too, for the .exe's icon, so it uses nothing of the crate.

type Rgb = [u8; 3];

/// The tile's colour and its rim (as minato's: `#16181d` and `#2c3039`).
const TILE: Rgb = [0x16, 0x18, 0x1d];
const RIM: Rgb = [0x2c, 0x30, 0x39];
/// The page, its header band, the days and today.
const PAGE: Rgb = [0xe4, 0xe7, 0xee];
const CYAN: Rgb = [0x6f, 0xd0, 0xd0];
const DAY: Rgb = [0x8a, 0x90, 0xa0];
const GOLD: Rgb = [0xe8, 0xc8, 0x7a];

/// The page in the 16-unit box: its corners, the header band's bottom.
const PAGE_MIN: [f32; 2] = [3.0, 3.2];
const PAGE_MAX: [f32; 2] = [13.0, 13.2];
const HEADER_BOTTOM: f32 = 6.0;
/// The days, three to a row; today is the last row's middle one.
const DAYS: [[f32; 2]; 6] = [[5.4, 8.3], [8.0, 8.3], [10.6, 8.3], [5.4, 11.0], [8.0, 11.0], [10.6, 11.0]];
const TODAY: usize = 4;

/// The icon as RGBA, `size` × `size`.
pub fn pixels(size: u32) -> Vec<u8> {
    let n = size as usize;
    let unit = size as f32 / 16.0;
    // Bigger days at small sizes, so the grid survives 16 px.
    let boost = if size <= 16 {
        1.35
    } else if size <= 32 {
        1.15
    } else {
        1.0
    };
    let day_r = 0.85 * unit * boost;
    let today_r = 1.15 * unit * boost;
    let radius = 3.4 * unit;
    let page_r = 1.3 * unit;
    let (pmin, pmax) = (PAGE_MIN.map(|v| v * unit), PAGE_MAX.map(|v| v * unit));
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let at = [x as f32 + 0.5, y as f32 + 0.5];
            let tile = rounded(at, [0.0, 0.0], [size as f32, size as f32], radius);
            if tile <= 0.0 {
                continue;
            }
            let mut rgb = TILE.map(f32::from);
            // The rim: a half-unit band inside the edge.
            let inside = rounded(at, [0.25 * unit; 2], [size as f32 - 0.25 * unit; 2], radius - 0.25 * unit);
            blend(&mut rgb, RIM, 1.0 - inside);
            let page = rounded(at, pmin, pmax, page_r);
            blend(&mut rgb, PAGE, page);
            // The header: the page's top, down to HEADER_BOTTOM.
            let band = (HEADER_BOTTOM * unit - at[1] + 0.5).clamp(0.0, 1.0);
            blend(&mut rgb, CYAN, page * band);
            for (i, d) in DAYS.iter().enumerate() {
                let c = [d[0] * unit, d[1] * unit];
                let dist = ((at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2)).sqrt();
                let (r, color) = if i == TODAY { (today_r, GOLD) } else { (day_r, DAY) };
                blend(&mut rgb, color, (r - dist + 0.5).clamp(0.0, 1.0));
            }
            let i = (y * n + x) * 4;
            out[i..i + 3].copy_from_slice(&rgb.map(|v| v.round() as u8));
            out[i + 3] = (tile * 255.0).round() as u8;
        }
    }
    out
}

fn blend(rgb: &mut [f32; 3], color: Rgb, cover: f32) {
    for k in 0..3 {
        rgb[k] += (f32::from(color[k]) - rgb[k]) * cover;
    }
}

/// How much of the pixel at `p` is inside the rounded box `min`..`max`
/// with corners of radius `r`, 0 to 1.
fn rounded(p: [f32; 2], min: [f32; 2], max: [f32; 2], r: f32) -> f32 {
    let q = |k: usize| (p[k] - (min[k] + max[k]) / 2.0).abs() - ((max[k] - min[k]) / 2.0 - r);
    let (qx, qy) = (q(0), q(1));
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
    (0.5 - outside).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_has_the_page_its_header_the_days_and_today() {
        let px = pixels(128);
        assert_eq!(px.len(), 128 * 128 * 4);
        let at = |ux: f32, uy: f32| {
            let (x, y) = ((ux * 8.0) as usize, (uy * 8.0) as usize);
            &px[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4]
        };
        assert_eq!(at(0.1, 0.1)[3], 0, "the corner is cut round");
        assert_eq!(at(1.5, 8.0), &[0x16, 0x18, 0x1d, 255], "the tile beside the page");
        assert_eq!(at(8.0, 7.2), &[0xe4, 0xe7, 0xee, 255], "the page between the header and the days");
        let head = at(8.0, 4.5);
        assert!(head[1] > 0xb0 && head[0] < 0x90, "the cyan header: {head:?}");
        let day = at(5.4, 8.3);
        assert!(day[0] < 0xa0 && day[2] > 0x90, "a day: {day:?}");
        let today = at(8.0, 11.0);
        assert!(today[0] > 0xc0 && today[2] < 0x90, "today, gold: {today:?}");
        assert_eq!(pixels(16).len(), 16 * 16 * 4);
    }
}
