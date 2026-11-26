use base64::{Engine, engine::general_purpose::STANDARD};
use image::{
    ColorType, ImageEncoder, RgbImage, RgbaImage,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
    sync::LazyLock,
};
use steam_pulse::model::Preferences;
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::{
        GetClientRect, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    },
};

#[derive(PartialEq, Clone, Copy)]
pub struct Options {
    depth: f32,
    blur: i32,
    dispersion: f32,
    light: f32,
    shine: f32,
    radius: f32,
}
impl Options {
    pub fn settings(p: &Preferences, prefix: &str, dpi: f64) -> Self {
        let read = |key| p.number(&format!("{prefix}{key}"));
        Self {
            depth: read("GlassDepth") as f32,
            blur: (read("GlassBlur") * dpi).round() as i32,
            dispersion: read("GlassDispersion") as f32,
            light: read("GlassLight").to_radians() as f32,
            shine: read("Shine") as f32,
            radius: ((if prefix == "Menu" {
                p.number("MenuRadius")
            } else {
                20.
            }) * dpi) as f32,
        }
    }
}

pub fn bounds(hwnd: HWND) -> Option<RECT> {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        if GetClientRect(hwnd, &mut client) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
            return None;
        }
    }
    (client.right > 0 && client.bottom > 0).then_some(RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + client.right,
        bottom: origin.y + client.bottom,
    })
}

pub fn exclude(hwnd: HWND, on: bool) -> bool {
    unsafe {
        SetWindowDisplayAffinity(hwnd, if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE }) != 0
    }
}

pub fn png(image: &RgbaImage) -> Option<String> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            image,
            image.width(),
            image.height(),
            ColorType::Rgba8.into(),
        )
        .ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[derive(Default)]
pub struct Cache {
    previous: Option<(RgbImage, Options)>,
}

impl Cache {
    pub fn frame(&mut self, rect: RECT, mut options: Options) -> Option<String> {
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let ratio = (640. / w.max(h) as f32).min(1.);
        let (sw, sh) = (
            (w as f32 * ratio).round() as i32,
            (h as f32 * ratio).round() as i32,
        );
        if sw <= 0 || sh <= 0 {
            return None;
        }
        let rgb = desktop(rect, sw, sh)?;
        options.blur = (options.blur as f32 * ratio).round() as i32;
        options.radius *= ratio;
        if self
            .previous
            .as_ref()
            .is_some_and(|(input, p)| *p == options && *input == rgb)
        {
            return None;
        }
        let out = filter(rgb.clone(), options);
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, 76)
            .encode(&out, sw as u32, sh as u32, ColorType::Rgb8.into())
            .ok()?;
        self.previous = Some((rgb, options));
        Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(data)))
    }
}

fn desktop(rect: RECT, w: i32, h: i32) -> Option<RgbImage> {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(screen);
        let mut pixels = null_mut();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                ..zeroed()
            },
            ..zeroed()
        };
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if !dc.is_null() && !bmp.is_null() {
            let old = SelectObject(dc, bmp);
            SetStretchBltMode(dc, HALFTONE);
            if StretchBlt(
                dc,
                0,
                0,
                w,
                h,
                screen,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SRCCOPY | CAPTUREBLT,
            ) != 0
            {
                GdiFlush();
                let bytes = std::slice::from_raw_parts(pixels.cast::<u8>(), (w * h * 4) as usize);
                for bgr in bytes.as_chunks::<4>().0 {
                    data.extend([bgr[2], bgr[1], bgr[0]]);
                }
            }
            SelectObject(dc, old);
        }
        if !bmp.is_null() {
            DeleteObject(bmp);
        }
        if !dc.is_null() {
            DeleteDC(dc);
        }
        ReleaseDC(null_mut(), screen);
    }
    RgbImage::from_raw(w as u32, h as u32, data)
}

fn filter(rgb: RgbImage, p: Options) -> RgbImage {
    static PROFILE: LazyLock<[f32; 256]> = LazyLock::new(|| {
        let profile = |u: f32| {
            let slope = u.powi(3) / (1. - u.powi(4)).max(1e-9).powf(0.75);
            let incident = slope.atan();
            (incident - (incident.sin() / 1.5).asin()).tan()
        };
        let max = profile(1.);
        std::array::from_fn(|i| profile(1. - i as f32 / 255.) / max)
    });
    let (w, h) = (rgb.width() as i32, rgb.height() as i32);
    let output = if p.blur > 0 {
        let mut mid = vec![0; rgb.len()];
        let mut soft = vec![0; rgb.len()];
        blur(&rgb, &mut mid, w, h, p.blur, false);
        blur(&mid, &mut soft, w, h, p.blur, true);
        blur(&soft, &mut mid, w, h, p.blur, false);
        blur(&mid, &mut soft, w, h, p.blur, true);
        soft
    } else {
        rgb.into_raw()
    };
    let short = w.min(h) as f32;
    let bevel = short * 0.3;
    let radius = p.radius.max(bevel).min(short * 0.5);
    let base = 0.35 * bevel * p.depth;
    let spread = (if short < 80. { 5f32 } else { 10. })
        .min(p.dispersion.max(0.) * bevel * 0.5 * (p.depth.abs() * 4.).min(1.));
    let spectrum = if spread <= 0.5 { 0. } else { spread * 0.27 };
    let (half_w, half_h) = (w as f32 / 2., h as f32 / 2.);
    let (light_x, light_y) = (p.light.sin(), -p.light.cos());
    let mut result = output.clone();
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x as f32 + 0.5 - half_w, y as f32 + 0.5 - half_h);
            let (qx, qy) = (cx.abs() - (half_w - radius), cy.abs() - (half_h - radius));
            let (distance, nx, ny) = if qx > 0. && qy > 0. {
                let length = qx.hypot(qy);
                (radius - length, qx / length, qy / length)
            } else if qx > qy {
                (radius - qx, 1., 0.)
            } else {
                (radius - qy, 0., 1.)
            };
            if distance >= bevel {
                continue;
            }
            let (nx, ny) = (nx * cx.signum(), ny * cy.signum());
            let cell = (distance / bevel).clamp(0., 1.) * 255.;
            let i = cell as usize;
            let power = mix(PROFILE[i], PROFILE[(i + 1).min(255)], cell - i as f32);
            let facing = nx * light_x + ny * light_y;
            let edge = (1. - distance / (bevel * 0.26).max(2.)).clamp(0., 1.);
            let glare = p.shine * edge * (0.11 + 0.41 * facing.max(0.) + 0.23 * (-facing).max(0.));
            for ch in 0..3 {
                let offset = base + spectrum * (ch as f32 - 1.);
                let value = sample(
                    &output,
                    w,
                    h,
                    x as f32 - nx * power * offset,
                    y as f32 - ny * power * offset,
                    ch,
                );
                result[((y * w + x) * 3) as usize + ch] =
                    mix(value, 255., glare).clamp(0., 255.) as u8;
            }
        }
    }
    RgbImage::from_raw(w as u32, h as u32, result).expect("matching image dimensions")
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn sample(src: &[u8], w: i32, h: i32, x: f32, y: f32, ch: usize) -> f32 {
    let x = x.clamp(0., (w - 1) as f32);
    let y = y.clamp(0., (h - 1) as f32);
    let (px, py) = (x as i32, y as i32);
    let (sx, sy) = ((px + 1).min(w - 1), (py + 1).min(h - 1));
    let at = |x, y| src[((y * w + x) * 3) as usize + ch] as f32;
    mix(
        mix(at(px, py), at(sx, py), x - px as f32),
        mix(at(px, sy), at(sx, sy), x - px as f32),
        y - py as f32,
    )
}

fn blur(src: &[u8], out: &mut [u8], w: i32, h: i32, r: i32, vertical: bool) {
    let (lines, length, stride, line_stride) = if vertical {
        (w, h, w * 3, 3)
    } else {
        (h, w, 3, w * 3)
    };
    for line in 0..lines {
        let index = |i: i32| (line * line_stride + i.clamp(0, length - 1) * stride) as usize;
        let mut sum = [0i32; 3];
        for i in -r..=r {
            for ch in 0..3 {
                sum[ch] += i32::from(src[index(i) + ch]);
            }
        }
        for i in 0..length {
            let (at, add, remove) = (index(i), index(i + r + 1), index(i - r));
            for ch in 0..3 {
                out[at + ch] = (sum[ch] / (2 * r + 1)) as u8;
                sum[ch] += i32::from(src[add + ch]) - i32::from(src[remove + ch]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn flat_glass_is_identity_and_shine_is_local_to_the_edge() {
        let image = RgbImage::from_pixel(80, 60, Rgb([40, 60, 80]));
        let options = Options {
            depth: 0.,
            blur: 3,
            dispersion: 0.,
            light: 0.,
            shine: 0.,
            radius: 10.,
        };
        assert_eq!(filter(image.clone(), options), image);
        let shiny = filter(
            image.clone(),
            Options {
                shine: 1.,
                ..options
            },
        );
        assert!(shiny.get_pixel(40, 0)[0] > image.get_pixel(40, 0)[0]);
        assert_eq!(shiny.get_pixel(40, 30), image.get_pixel(40, 30));
    }
}
