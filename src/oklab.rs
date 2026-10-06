//! Conversions between sRGB colors and OKLab, after Björn Ottosson.

use crate::config::Color;

/// The OKLab coordinates [L, a, b] of the color, ignoring alpha.
pub fn from_color(c: Color) -> [f64; 3] {
    let [r, g, b] = [c.0[0], c.0[1], c.0[2]].map(|v| {
        let x = v as f64 / 255.0;
        if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
    });
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// The color with OKLab coordinates [L, a, b] and the given alpha; channels outside sRGB are
/// clipped.
pub fn to_color([l, a, b]: [f64; 3], alpha: u8) -> Color {
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    let linear = [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    ];
    let [r, g, b] = linear.map(|x| {
        let x = x.clamp(0.0, 1.0);
        let v = if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round() as u8
    });
    Color([r, g, b, alpha])
}
