use crate::native::Draw;
use std::{mem::size_of, ptr::null_mut};
use windows::{
    Win32::{
        Foundation::RECT as D2dRect,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::HDC as D2dDc,
        },
    },
    core::{Result, w},
};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT, SIZE},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow},
};

const PIXEL_FORMAT: D2D1_PIXEL_FORMAT = D2D1_PIXEL_FORMAT {
    format: DXGI_FORMAT_B8G8R8A8_UNORM,
    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
};

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
    #[cfg(test)]
    pixels: *mut u8,
}

impl Surface {
    fn new(width: i32, height: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    ..BITMAPINFOHEADER::default()
                },
                ..BITMAPINFO::default()
            };
            let mut pixels = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Self {
                dc,
                bitmap,
                previous,
                width,
                height,
                #[cfg(test)]
                pixels: pixels.cast(),
            })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

pub struct Renderer {
    target: ID2D1DCRenderTarget,
    write: IDWriteFactory,
    surface: Option<Surface>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: PIXEL_FORMAT,
                usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                ..D2D1_RENDER_TARGET_PROPERTIES::default()
            })?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let write = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Self {
                target,
                write,
                surface: None,
            })
        }
    }

    pub fn present(&mut self, hwnd: HWND, bounds: RECT, draw: &Draw, redraw: bool) -> bool {
        let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        let resize = self
            .surface
            .as_ref()
            .is_none_or(|s| (s.width, s.height) != (width, height));
        if resize {
            self.surface = Surface::new(width, height);
        }
        let Some(s) = &self.surface else { return false };
        if (resize || redraw) && self.paint(s, draw).is_err() {
            if let Ok(replacement) = Self::new() {
                *self = replacement;
            }
            return false;
        }
        unsafe {
            let origin = POINT {
                x: bounds.left,
                y: bounds.top,
            };
            let size = SIZE {
                cx: width,
                cy: height,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                null_mut(),
                &origin,
                &size,
                s.dc,
                &POINT::default(),
                0,
                &blend,
                ULW_ALPHA,
            ) != 0
        }
    }

    fn paint(&self, surface: &Surface, draw: &Draw) -> Result<()> {
        let dpi = draw.dpi as f32;
        let (w, h) = (surface.width as f32 / dpi, surface.height as f32 / dpi);
        unsafe {
            self.target.BindDC(
                D2dDc(surface.dc),
                &D2dRect {
                    left: 0,
                    top: 0,
                    right: surface.width,
                    bottom: surface.height,
                },
            )?;
            self.target.SetDpi(dpi * 96., dpi * 96.);
            self.target.BeginDraw();
            // Keep a one-alpha hit surface in Air Mode, matching the original window.
            self.target.Clear(Some(&color(0, 1. / 255.)));
            let result = self.card(w, h, draw);
            let end = self.target.EndDraw(None, None);
            result.and(end)
        }
    }

    fn format(
        &self,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
        alignment: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextFormat> {
        unsafe {
            let format = self.write.CreateTextFormat(
                w!("Segoe UI"),
                None::<&IDWriteFontCollection>,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-US"),
            )?;
            let sign = self.write.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    ..Default::default()
                },
                &sign,
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format.SetTextAlignment(alignment)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            Ok(format)
        }
    }

    fn card(&self, w: f32, h: f32, d: &Draw) -> Result<()> {
        let target = &self.target;
        let radius = d.radius as f32;
        let outer = rounded(rect(0., 0., w, h), radius);
        unsafe {
            let foreground = target.CreateSolidColorBrush(&color(d.text, 1.), None)?;
            let accent = self.brush(d.accent, d.accent_end, 1., d.angle, outer.rect)?;
            let track = target.CreateSolidColorBrush(&color(d.text, 0.18), None)?;
            if !d.air {
                let background =
                    self.brush(d.background, d.end, d.opacity as f32, d.angle, outer.rect)?;
                target.FillRoundedRectangle(&outer, &background);
                if d.border {
                    let white = target.CreateSolidColorBrush(
                        &color(0xffffff, if d.glass { 0.36 } else { 0.26 }),
                        None,
                    )?;
                    target.DrawRoundedRectangle(
                        &rounded(rect(0.5, 0.5, w - 0.5, h - 0.5), (radius - 0.5).max(0.)),
                        &white,
                        1.,
                        None::<&ID2D1StrokeStyle>,
                    );
                }
            }
            if d.badge {
                let centered = self.format(
                    15.,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                )?;
                if let Some(row) = d.rows.first() {
                    if row.friend
                        && let Some(icon) = &row.art
                    {
                        self.image(
                            icon,
                            rect(
                                (w - 15.) / 2.,
                                (h - 15.) / 2.,
                                (w + 15.) / 2.,
                                (h + 15.) / 2.,
                            ),
                            true,
                        )?;
                    } else {
                        let glyph = if row.glyph.is_empty() {
                            row.badge
                        } else {
                            row.glyph
                        };
                        self.text(glyph, &centered, outer.rect, &accent);
                    }
                }
                return Ok(());
            }
            let title = self.format(
                10.5,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
            )?;
            let regular = self.format(
                10.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
            )?;
            let detail = self.format(
                10.,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            )?;
            let centered =
                self.format(12., DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let dense = d.rows.len() > 1;
            let heights: Vec<_> = d
                .rows
                .iter()
                .map(|r| {
                    if r.friend || r.simple {
                        14.
                    } else {
                        let edge: f32 = if r.glyph.is_empty() {
                            0.
                        } else if dense {
                            16.
                        } else {
                            23.
                        };
                        edge.max(if r.progress.is_some() { 17. } else { 14. })
                    }
                })
                .collect();
            let total = heights.iter().sum::<f32>() + (heights.len().saturating_sub(1) as f32) * 2.;
            let factor = ((h - 6.) / total.max(1.)).min(1.);
            let mut y = (h - total * factor) / 2.;
            for (row, row_height) in d.rows.iter().zip(heights) {
                let row_height = row_height * factor;
                let edge: f32 = if row.friend {
                    11.
                } else if dense {
                    16.
                } else {
                    23.
                };
                let edge = edge.min(row_height);
                let show_icon = !row.glyph.is_empty();
                let x = if show_icon { 8. + edge + 7. } else { 8. };
                if show_icon {
                    let top = y + (row_height - edge) / 2.;
                    let icon = rect(8., top, 8. + edge, top + edge);
                    if let Some(art) = &row.art {
                        self.image(art, icon, row.friend)?;
                    } else {
                        if !row.friend {
                            target.FillRoundedRectangle(&rounded(icon, 6.), &track);
                        }
                        self.text(row.glyph, &centered, icon, &accent);
                    }
                }
                let right = w - 8.;
                let detail_width = if row.detail.is_empty() {
                    0.
                } else {
                    let utf16: Vec<_> = row.detail.encode_utf16().collect();
                    let layout = self.write.CreateTextLayout(&utf16, &detail, w, 24.)?;
                    let mut metrics = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut metrics)?;
                    (metrics.width + 7.).min((right - x) * 0.72)
                };
                let text_bottom = y + row_height - if row.progress.is_some() { 3. } else { 0. };
                let label = if row.title.is_empty() {
                    &row.state
                } else {
                    &row.title
                };
                self.text(
                    label,
                    if row.friend || row.simple {
                        &regular
                    } else {
                        &title
                    },
                    rect(x, y, right - detail_width, text_bottom),
                    &foreground,
                );
                if detail_width > 0. {
                    let bounds = rect(right - detail_width, y, right, text_bottom);
                    let ink = rect(right - (detail_width - 7.).max(1.), y, right, text_bottom);
                    let detail_brush = self.brush(d.accent, d.accent_end, 1., d.angle, ink)?;
                    self.text(&row.detail, &detail, bounds, &detail_brush);
                }
                if let Some(p) = row.progress
                    && right > x
                {
                    let top = y + row_height - 2.;
                    target
                        .FillRoundedRectangle(&rounded(rect(x, top, right, top + 2.), 1.), &track);
                    let amount = (right - x) * (p.clamp(0., 100.) as f32) / 100.;
                    if amount > 0. {
                        let filled = rect(x, top, x + amount, top + 2.);
                        let fill = self.brush(d.accent, d.accent_end, 1., d.angle, filled)?;
                        target.FillRoundedRectangle(&rounded(filled, 1.), &fill);
                    }
                }
                y += row_height + 2. * factor;
            }
        }
        Ok(())
    }

    fn brush(
        &self,
        a: u32,
        b: u32,
        opacity: f32,
        angle: f64,
        bounds: D2D_RECT_F,
    ) -> Result<ID2D1Brush> {
        unsafe {
            use windows::core::Interface;
            if a == b {
                return self
                    .target
                    .CreateSolidColorBrush(&color(a, opacity), None)?
                    .cast();
            }
            let stops = self.target.CreateGradientStopCollection(
                &[
                    D2D1_GRADIENT_STOP {
                        position: 0.,
                        color: color(a, opacity),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.,
                        color: color(b, opacity),
                    },
                ],
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )?;
            let (dy, dx) = angle.to_radians().sin_cos();
            let (dx, dy) = (dx as f32, dy as f32);
            let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
            self.target
                .CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: windows_numerics::Vector2 {
                            X: bounds.left + (1. - dx) * w / 2.,
                            Y: bounds.top + (1. - dy) * h / 2.,
                        },
                        endPoint: windows_numerics::Vector2 {
                            X: bounds.left + (1. + dx) * w / 2.,
                            Y: bounds.top + (1. + dy) * h / 2.,
                        },
                    },
                    None,
                    &stops,
                )?
                .cast()
        }
    }

    fn text(
        &self,
        value: &str,
        format: &IDWriteTextFormat,
        bounds: D2D_RECT_F,
        brush: &ID2D1Brush,
    ) {
        if value.is_empty() || bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            return;
        }
        let value: Vec<_> = value.encode_utf16().collect();
        unsafe {
            self.target.DrawText(
                &value,
                format,
                &bounds,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn image(&self, art: &image::RgbaImage, bounds: D2D_RECT_F, circle: bool) -> Result<()> {
        let (w, h) = art.dimensions();
        let pixels: Vec<u8> = art
            .pixels()
            .flat_map(|p| {
                let alpha = u16::from(p[3]);
                let premul = |channel| ((u16::from(channel) * alpha + 127) / 255) as u8;
                [premul(p[2]), premul(p[1]), premul(p[0]), p[3]]
            })
            .collect();
        unsafe {
            let bitmap = self.target.CreateBitmap(
                D2D_SIZE_U {
                    width: w,
                    height: h,
                },
                Some(pixels.as_ptr().cast()),
                w * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: PIXEL_FORMAT,
                    dpiX: 96.,
                    dpiY: 96.,
                },
            )?;
            let edge = (bounds.right - bounds.left).min(bounds.bottom - bounds.top);
            let tiled = self.target.CreateBitmapBrush(&bitmap, None, None)?;
            tiled.SetInterpolationMode(D2D1_BITMAP_INTERPOLATION_MODE_LINEAR);
            tiled.SetTransform(&windows_numerics::Matrix3x2 {
                M11: edge / w as f32,
                M22: edge / h as f32,
                M31: bounds.left,
                M32: bounds.top,
                ..Default::default()
            });
            self.target.FillRoundedRectangle(
                &rounded(bounds, if circle { edge / 2. } else { 6. }),
                &tiled,
            );
        }
        Ok(())
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}
fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}
fn color(bgr: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (bgr & 255) as f32 / 255.,
        g: ((bgr >> 8) & 255) as f32 / 255.,
        b: ((bgr >> 16) & 255) as f32 / 255.,
        a: alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::Row;

    #[test]
    fn system_load_text_spans_both_accent_colors() {
        let (width, height) = (400, 32);
        let surface = Surface::new(width, height).unwrap();
        let renderer = Renderer::new().unwrap();
        let draw = Draw {
            dpi: 1.,
            air: true,
            accent: 0xff0000,
            accent_end: 0xff,
            rows: vec![Row {
                detail: "CPU 17%  RAM 72%".into(),
                ..Row::default()
            }],
            ..Draw::default()
        };
        renderer.paint(&surface, &draw).unwrap();
        unsafe {
            GdiFlush();
        }
        let pixels =
            unsafe { std::slice::from_raw_parts(surface.pixels, (width * height * 4) as usize) };
        let ink: Vec<_> = (0..width as usize)
            .filter_map(|x| {
                (0..height as usize)
                    .map(|y| {
                        let i = (y * width as usize + x) * 4;
                        &pixels[i..i + 4]
                    })
                    .find(|pixel| pixel[3] >= 100)
            })
            .collect();
        assert!(ink.len() > 16);
        assert!(ink[..8].iter().any(|p| p[0] > p[2].saturating_add(50)));
        assert!(
            ink[ink.len() - 8..]
                .iter()
                .any(|p| p[2] > p[0].saturating_add(50))
        );
    }

    #[test]
    fn gradient_draws_distinct_background_and_partial_progress_colors() {
        let surface = Surface::new(180, 32).unwrap();
        let renderer = Renderer::new().unwrap();
        let drawing = Draw {
            dpi: 1.,
            opacity: 0.76,
            glass: true,
            background: 0xff,
            end: 0xff0000,
            accent: 0xff0000,
            accent_end: 0xff,
            rows: vec![Row {
                progress: Some(25.),
                ..Row::default()
            }],
            ..Draw::default()
        };
        renderer.paint(&surface, &drawing).unwrap();
        unsafe {
            GdiFlush();
        }
        let bytes = unsafe { std::slice::from_raw_parts(surface.pixels, 180 * 32 * 4) };
        let pixel = |x: usize, y: usize| &bytes[(y * 180 + x) * 4..(y * 180 + x) * 4 + 4];
        assert!(pixel(5, 8)[2] > pixel(5, 8)[0] && pixel(174, 8)[0] > pixel(174, 8)[2]);
        assert!(pixel(11, 23)[0] > pixel(11, 23)[2] && pixel(45, 23)[2] > pixel(45, 23)[0]);
        assert!((190..=196).contains(&pixel(5, 8)[3]));
    }

    #[test]
    fn air_mode_keeps_correct_icon_colors_and_translucent_glyph_edges() {
        let surface = Surface::new(160, 34).unwrap();
        let renderer = Renderer::new().unwrap();
        let image = image::RgbaImage::from_pixel(48, 48, image::Rgba([212, 110, 31, 255]));
        let draw = Draw {
            dpi: 1.,
            air: true,
            opacity: 0.3,
            text: 0xfff7f1,
            accent: 0xffe054,
            rows: vec![Row {
                title: "Steam Pulse".into(),
                glyph: "▶",
                art: Some(std::sync::Arc::new(image)),
                ..Row::default()
            }],
            ..Draw::default()
        };
        renderer.paint(&surface, &draw).unwrap();
        unsafe {
            GdiFlush();
        }
        let bytes = unsafe { std::slice::from_raw_parts(surface.pixels, 160 * 34 * 4) };
        let pixel = |x: usize, y: usize| &bytes[(y * 160 + x) * 4..(y * 160 + x) * 4 + 4];
        assert_eq!(pixel(0, 0), [0, 0, 0, 1]);
        assert_eq!(pixel(19, 17), [31, 110, 212, 255]);
        let glyphs = (8..27).flat_map(|y| (40..115).map(move |x| pixel(x, y)));
        assert!(glyphs.clone().any(|p| (12..235).contains(&p[3])));
        assert!(
            glyphs
                .filter(|p| p[3] > 12)
                .all(|p| p[0] >= p[1] && p[1] >= p[2])
        );
    }
}
