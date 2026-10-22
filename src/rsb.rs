use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::Context;
use byteorder::{LE, ReadBytesExt};

pub fn read(filename: &Path) -> anyhow::Result<Rsb> {
    let file = File::open(filename).context("could not open RSB file")?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).context("failed to read RSB file")?;
    let mut buf = Cursor::new(buf);

    let mut rsb = Rsb::default();
    rsb.filename = filename.to_path_buf();

    rsb.version = buf.read_u32::<LE>()?;
    // Only handle Rainbow Six and Rogue Spear
    if rsb.version >= 2 {
        anyhow::bail!("RSB version {} not supported", rsb.version);
    }

    rsb.width = buf.read_u32::<LE>()?;
    rsb.height = buf.read_u32::<LE>()?;
    rsb.palette = if rsb.version == 0 {
        let palette = buf.read_u32::<LE>()?;
        if palette == 0 {
            rsb.bitmask = BitMask::try_new(&mut buf)?;
        } else if palette == 1 {
            // Read the 256 palette colors
            let mut tmp = vec![0u8; 256 * std::mem::size_of::<u32>()];
            let mut colors = Vec::with_capacity(256);
            buf.read_exact(&mut tmp)?;
            for w in tmp.windows(4) {
                let b = w[0];
                let g = w[1];
                let r = w[2];
                let a = w[3];
                colors.push(PaletteColor::new(b, g, r, a));
            }
            rsb.palette_colors = Some(colors);
        } else {
            let path = rsb.filename.display();
            anyhow::bail!("{path}: palette {palette} is unhandled");
        }
        Some(palette)
    } else {
        rsb.bitmask = BitMask::try_new(&mut buf)?;
        None
    };

    let size = (rsb.width * rsb.height) as usize;
    rsb.pixels = Vec::with_capacity(size);
    for _ in 0..size {
        let pixel = if rsb.version == 0 && rsb.palette.is_some_and(|x| x == 1) {
            // Read the palette color index
            let mut tmp = [0u8; 1];
            buf.read_exact(&mut tmp)?;
            Pixel::PaletteColorIndex(tmp[0])
        } else {
            // Read either ARGB or BGRA pixel data
            // TODO: convert to one pixel format?
            let value = buf.read_u16::<LE>()?.into();
            if rsb.bitmask.is_argb() {
                Pixel::Argb(value)
            } else {
                // TODO(simplify?) Rogue Spear RSBs are only 16-bit BGRA
                Pixel::Bgra(value)
            }
        };
        rsb.pixels.push(pixel);
    }

    if rsb.version == 0 && rsb.palette.is_some_and(|x| x == 1) {
        rsb.bitmask = BitMask::try_new(&mut buf)?;

        let mut masked_pixels = Vec::with_capacity(size);
        for _ in 0..size {
            let value = buf.read_u16::<LE>()?.into();
            masked_pixels.push(MaskedPixel(value));
        }
        rsb.masked_pixels = Some(masked_pixels);
    }

    Ok(rsb)
}

#[derive(Clone, Debug, Default)]
pub struct Rsb {
    pub filename: PathBuf,

    /// RSB version
    ///
    /// 0 - 1: games released before Ghost Recon: Rainbow Six and Rogue Spear
    /// 0 - 9: Ghost Recon and Sum of All Fears
    /// 9 - 11: Rainbow Six Lockdown
    pub version: u32,

    /// Texture width
    pub width: u32,

    /// Texture height
    pub height: u32,

    /// Additional 8-bit texture copy and palette. Only when `version == 0`.
    pub palette: Option<u32>,

    /// 8-bit palette of size 256 elements
    pub palette_colors: Option<Vec<PaletteColor>>,

    /// The bits per RGBA for pixel data. See `masked_pixels` as an example
    /// of how this is field is used.
    // TODO: remove this? and convert `masked_pixels` to its resolved form?
    // TODO: same applies when only `pixels` and BGRA order
    pub bitmask: BitMask,

    /// Pixel data of size `width * height`
    pub pixels: Vec<Pixel>,

    /// `width * height` of image data when `version == 0` and `palette == 1`.
    /// The `bitmask` must be used to extract the RGBA data.
    pub masked_pixels: Option<Vec<MaskedPixel>>,
}

impl Rsb {
    /// The `width * height` dimensions of this RSB
    pub fn size(&self) -> usize {
        (self.width * self.height) as _
    }

    /// The image as `width * height` RGBA8 pixels, top row first. Empty if a
    /// palette is missing.
    pub fn to_rgba8(&self) -> Vec<u8> {
        rsb_to_rgba8(self)
    }

    /// As `to_rgba8`, with the pixels of the `key` color (8-bit RGB) clear. As in the original,
    /// the key is cut to the texture's bits per channel (184 160 104 is 0xBD0D in 5/6/5) and
    /// matched exactly. A clear pixel takes the average color of the solid ones around it, so
    /// that filtering does not fringe the edges with the key.
    pub fn to_rgba8_keyed(&self, key: [u8; 3]) -> Vec<u8> {
        let mut out = self.to_rgba8();
        if out.is_empty() {
            return out;
        }
        let bm = &self.bitmask;
        let cut = |k: u8, bits: u32| if (1..=8).contains(&bits) { Some(u32::from(k) >> (8 - bits)) } else { None };
        let clear: Vec<bool> = match &self.palette_colors {
            Some(colors) => self.pixels.iter().map(|px| match px {
                Pixel::PaletteColorIndex(i) => colors.get(*i as usize).is_some_and(|c| [c.r, c.g, c.b] == key),
                _ => false,
            }).collect(),
            None => self.pixels.iter().map(|px| {
                px.r(bm) == cut(key[0], bm.r) && px.g(bm) == cut(key[1], bm.g) && px.b(bm) == cut(key[2], bm.b)
            }).collect(),
        };
        let (w, h) = (self.width as usize, self.height as usize);
        let solid = out.clone();
        for (i, _) in clear.iter().enumerate().filter(|(_, c)| **c) {
            let (x, y) = (i % w, i / w);
            let mut sum = [0u32; 3];
            let mut n = 0;
            // Textures tile, so the neighbors wrap round the edges
            for (dx, dy) in [(w - 1, h - 1), (0, h - 1), (1, h - 1), (w - 1, 0), (1, 0), (w - 1, 1), (0, 1), (1, 1)] {
                let j = (y + dy) % h * w + (x + dx) % w;
                if !clear[j] {
                    for c in 0..3 {
                        sum[c] += u32::from(solid[j * 4 + c]);
                    }
                    n += 1;
                }
            }
            if n > 0 {
                for c in 0..3 {
                    out[i * 4 + c] = (sum[c] / n) as u8;
                }
            }
            out[i * 4 + 3] = 0;
        }
        out
    }
}

fn rsb_to_rgba8(tex: &Rsb) -> Vec<u8> {
    let size = tex.size();
    let mut out = Vec::with_capacity(size * 4);

    if tex.palette_colors.is_some() {
        // Expect palette indices in tex.pixels
        if let Some(colors) = &tex.palette_colors {
            for px in &tex.pixels {
                let idx = match px {
                    Pixel::PaletteColorIndex(i) => *i as usize,
                    _ => 0,
                };
                let c = colors.get(idx).cloned().unwrap_or(PaletteColor { r: 0, g: 0, b: 0, a: 255 });
                out.push(c.r);
                out.push(c.g);
                out.push(c.b);
                out.push(c.a);
            }
        }
        return out;
    }

    // Non-paletted: expand using bitmask
    let bm = &tex.bitmask;
    let rbits = bm.r.max(1);
    let gbits = bm.g.max(1);
    let bbits = bm.b.max(1);
    let abits = bm.a; // alpha may be 0

    for px in &tex.pixels {
        let r = px.r(bm).unwrap_or(0);
        let g = px.g(bm).unwrap_or(0);
        let b = px.b(bm).unwrap_or(0);
        let a = px.a(bm).unwrap_or_else(|| if abits == 0 { 255 } else { 0 });

        out.push(scale_to_u8(r, rbits));
        out.push(scale_to_u8(g, gbits));
        out.push(scale_to_u8(b, bbits));
        // If alpha channel not present, a is already 255
        out.push(if abits == 0 { a as u8 } else { scale_to_u8(a, abits) });
    }

    out
}

fn scale_to_u8(v: u32, bits: u32) -> u8 {
    if bits == 0 {
        return 255;
    }
    let max = (1u32 << bits) - 1;
    (((v * 255) + (max / 2)) / max) as u8
}


impl std::fmt::Display for Rsb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {{\n", self.filename.display())?;
        write!(f, "  version: {}\n", self.version)?;
        if let Some(palette) = &self.palette {
            write!(f, "  palette: {palette}\n")?;
        }
        write!(f, "  size: ({}, {})\n", self.height, self.width)?;
        if let Some(colors) = &self.palette_colors {
            write!(f, "  palette color count: {}\n", colors.len())?;
        }
        write!(f, "  RGBA bits: {}/{}/{}/{}\n",
            self.bitmask.r, self.bitmask.g, self.bitmask.b, self.bitmask.a)?;
        write!(f, "  pixels: {}\n", self.pixels.len())?;
        if let Some(masked) = &self.masked_pixels {
            write!(f, "  masked pixel count: {}\n", masked.len())?;
        }
        write!(f, "}}")
    }
}

/// The color depth bitmask. Use this to figure out the bit sizes of the RGBA
/// channels in pixel data.
#[derive(Clone, Debug, Default)]
pub struct BitMask {
    // TODO yank pub
    pub r: u32,
    pub g: u32,
    pub b: u32,
    pub a: u32,
}

impl BitMask {
    fn try_new(buf: &mut Cursor<Vec<u8>>) -> anyhow::Result<Self> {
        Ok(Self {
            r: buf.read_u32::<LE>()?,
            g: buf.read_u32::<LE>()?,
            b: buf.read_u32::<LE>()?,
            a: buf.read_u32::<LE>()?,
        })
    }

    /// ARGB order is used for pixel data when `bitmask` channel depths sum to
    /// exactly 32-bit; otherwise, BGRA is expected.
    pub fn is_argb(&self) -> bool {
        // TODO(simplify) no RSB assets in Rogue Spear have 32-bit depth
        self.bits() == 32
    }

    /// The bit-depth of this mask
    pub fn bits(&self) -> u32 {
        self.r + self.g + self.b + self.a
    }
}

#[derive(Clone, Debug)]
pub struct PaletteColor {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

impl PaletteColor {
    fn new(b: u8, g: u8, r: u8, a: u8) -> Self {
        Self { b, g, r, a }
    }
}

#[derive(Clone, Debug)]
pub enum Pixel {
    /// When `version` is 0 and `palette` is 1
    PaletteColorIndex(u8),

    /// Alpha, red, blue and green. When `bitmask` channels sum to 32 bits.
    /// Use `bitmask` fields to know how many bits each channel is.
    Argb(u32),

    /// Blue, green, red and alpha. Use `bitmask` fields to know how many bits
    /// each channel is.
    Bgra(u32),
}

// TODO(simplify?): unify pixel layout
impl Pixel {
    pub fn r(&self, bitmask: &BitMask) -> Option<u32> {
        match *self {
            Self::PaletteColorIndex(_) => unimplemented!("not implemented"),
            Self::Argb(bits) => {
                Self::masked(bits, bitmask.r, bitmask.a)
            }
            Self::Bgra(bits) => {
                Self::masked(bits, bitmask.r, bitmask.b + bitmask.g)
            }
        }
    }

    pub fn g(&self, bitmask: &BitMask) -> Option<u32> {
        match *self {
            Self::PaletteColorIndex(_) => unimplemented!("not implemented"),
            Self::Argb(bits) => {
                Self::masked(bits, bitmask.g, bitmask.a + bitmask.r)
            }
            Self::Bgra(bits) => {
                Self::masked(bits, bitmask.g, bitmask.b)
            }
        }
    }

    pub fn b(&self, bitmask: &BitMask) -> Option<u32> {
        match *self {
            Self::PaletteColorIndex(_) => unimplemented!("not implemented"),
            Self::Argb(bits) => {
                Self::masked(bits, bitmask.b, bitmask.a + bitmask.r + bitmask.g)
            }
            Self::Bgra(bits) => {
                Self::masked(bits, bitmask.b, 0)
            }
        }
    }

    pub fn a(&self, bitmask: &BitMask) -> Option<u32> {
        match *self {
            Self::PaletteColorIndex(_) => unimplemented!("not implemented"),
            Self::Argb(bits) => {
                Self::masked(bits, bitmask.a, 0)
            }
            Self::Bgra(bits) => {
                Self::masked(bits, bitmask.a, bitmask.b + bitmask.g + bitmask.r)
            }
        }
    }

    // TODO: make this generic and replace `MaskedPixel::masked`
    fn masked(value: u32, channel: u32, shift: u32) -> Option<u32> {
        if channel > 0 {
            let mask = ((1u32 << channel) - 1) << shift;
            Some((value & mask) >> shift)
        } else {
            None
        }
    }
}

/// Only used when `version == 0` and `palette == 1`. This represents 16-bit
/// pixel data where `BitMask` controls the Red, Blue, Green and Alpha layout.
///
/// Example:
/// `BitMask { r: 5, g: 6, b: 5, a: 0 }` means the `MaskedPixel` data contains
/// red (5 bits), green (6 bits), blue (5 bits) and no alpha bits.
#[derive(Clone, Debug)]
pub struct MaskedPixel(u16);

impl MaskedPixel {
    pub fn r(&self, bitmask: &BitMask) -> Option<u16> {
        self.masked(bitmask.r, 0)
    }

    pub fn g(&self, bitmask: &BitMask) -> Option<u16> {
        self.masked(bitmask.g, bitmask.r)
    }

    pub fn b(&self, bitmask: &BitMask) -> Option<u16> {
        self.masked(bitmask.b, bitmask.g + bitmask.r)
    }

    pub fn a(&self, bitmask: &BitMask) -> Option<u16> {
        self.masked(bitmask.a, bitmask.b + bitmask.g + bitmask.r)
    }

    fn masked(&self, channel: u32, shift: u32) -> Option<u16> {
        if channel > 0 {
            let mask = ((1u16 << channel) - 1) << shift;
            Some((self.0 & mask) >> shift)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn masked_pixel_16bit_extracts_correctly_with_bitmask() {
        // Sample pixel data from 'data/texture/sky_night_rainy_BK.rsb'
        let data = vec![
            0x46, 0x29,
            0x66, 0x29,
            0x45, 0x21,
            0x25, 0x21,
            0x05, 0x21,
        ];

        // Construct a 16-bit bitmask for the pixel data
        let bitmask = BitMask { r: 5, g: 6, b: 5, a: 0 };
        assert_eq!(bitmask.bits(), 16, "this test must use 16-bit pixel depth");
        assert!(!bitmask.is_argb(), "channel data is not 32-bit ARGB");

        // Treat the data as 16-bit chunks
        let size = data.len() / 2;
        let mut buf = Cursor::new(data);

        // Read in the 16-bit pixel data
        let masked_pixels = (0..size)
            .into_iter()
            .map(|_| buf.read_u16::<LE>().unwrap().into())
            .map(MaskedPixel)
            .collect::<Vec<_>>();


        // 0x46, 0x29
        assert_eq!(masked_pixels[0].r(&bitmask), Some(6));
        assert_eq!(masked_pixels[0].g(&bitmask), Some(10));
        assert_eq!(masked_pixels[0].b(&bitmask), Some(5));
        assert_eq!(masked_pixels[0].a(&bitmask), None);

        // 0x66, 0x29
        assert_eq!(masked_pixels[1].r(&bitmask), Some(6));
        assert_eq!(masked_pixels[1].g(&bitmask), Some(11));
        assert_eq!(masked_pixels[1].b(&bitmask), Some(5));
        assert_eq!(masked_pixels[1].a(&bitmask), None);

        // 0x45, 0x21
        assert_eq!(masked_pixels[2].r(&bitmask), Some(5));
        assert_eq!(masked_pixels[2].g(&bitmask), Some(10));
        assert_eq!(masked_pixels[2].b(&bitmask), Some(4));
        assert_eq!(masked_pixels[2].a(&bitmask), None);

        // 0x25, 0x21
        assert_eq!(masked_pixels[3].r(&bitmask), Some(5));
        assert_eq!(masked_pixels[3].g(&bitmask), Some(9));
        assert_eq!(masked_pixels[3].b(&bitmask), Some(4));
        assert_eq!(masked_pixels[3].a(&bitmask), None);

        // 0x05, 0x21,
        assert_eq!(masked_pixels[4].r(&bitmask), Some(5));
        assert_eq!(masked_pixels[4].g(&bitmask), Some(8));
        assert_eq!(masked_pixels[4].b(&bitmask), Some(4));
        assert_eq!(masked_pixels[4].a(&bitmask), None);

        // None of the Rogue Spear game files have version=1, palette=0, and
        // a nonzero alpha channel.
    }

    #[test]
    fn pixel_bgra_extracts_correctly_with_bitmask() {
        // Sample pixel data from 'data/texture/rsw_mp5k.rsb'
        let data = vec![
            0x08, 0x42,
            0x62, 0x10,
            0xa3, 0x10,
            0x04, 0x21,
        ];

        // Construct a 16-bit bitmask for the pixel data
        let bitmask = BitMask { r: 5, g: 6, b: 5, a: 0 };
        assert_eq!(bitmask.bits(), 16, "this test must use 16-bit pixel depth");
        assert!(!bitmask.is_argb(), "channel data is not 32-bit ARGB");

        // Treat the data as 16-bit chunks
        let size = data.len() / 2;
        let mut buf = Cursor::new(data);

        // Read in the 16-bit pixel data
        let pixels = (0..size)
            .into_iter()
            .map(|_| buf.read_u16::<LE>().unwrap().into())
            .map(Pixel::Bgra)
            .collect::<Vec<_>>();

        // 0x08, 0x42
        assert_eq!(pixels[0].b(&bitmask), Some(8));
        assert_eq!(pixels[0].g(&bitmask), Some(16));
        assert_eq!(pixels[0].r(&bitmask), Some(8));
        assert_eq!(pixels[0].a(&bitmask), None);

        // 0x62, 0x10
        assert_eq!(pixels[1].b(&bitmask), Some(2));
        assert_eq!(pixels[1].g(&bitmask), Some(3));
        assert_eq!(pixels[1].r(&bitmask), Some(2));
        assert_eq!(pixels[1].a(&bitmask), None);

        // 0xa3, 0x10
        assert_eq!(pixels[2].b(&bitmask), Some(3));
        assert_eq!(pixels[2].g(&bitmask), Some(5));
        assert_eq!(pixels[2].r(&bitmask), Some(2));
        assert_eq!(pixels[2].a(&bitmask), None);

        // 0xa3, 0x10
        assert_eq!(pixels[3].b(&bitmask), Some(4));
        assert_eq!(pixels[3].g(&bitmask), Some(8));
        assert_eq!(pixels[3].r(&bitmask), Some(4));
        assert_eq!(pixels[3].a(&bitmask), None);
    }

    // rm06's railing (cha_rail_01, keyed 184 160 104 in Rommel.CXP) is about 60% panel color,
    // which the original shows as clear
    #[test]
    fn a_color_key_clears_the_railings_panels() {
        let Ok(tex) = read(Path::new("data/map/rm06/cha_rail_01.rsb")) else { return };
        let rgba = tex.to_rgba8_keyed([184, 160, 104]);
        assert_eq!(rgba.len(), tex.size() * 4);
        let clear = rgba.chunks(4).filter(|p| p[3] == 0).count() as f32 / tex.size() as f32;
        assert!((0.5..0.7).contains(&clear), "{:.0}% clear", clear * 100.0);
        // Any other color keys nothing out
        assert!(tex.to_rgba8_keyed([1, 2, 3]).chunks(4).all(|p| p[3] == 255));
    }
}
