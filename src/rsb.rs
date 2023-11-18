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
    b: u8,
    g: u8,
    r: u8,
    a: u8,
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
}
