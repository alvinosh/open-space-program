//! Tile pack files: one file per layer holding compressed tiles and an index.
//!
//! Layout: `MAGIC`, then the tile blobs back to back, then the index (`count` entries of
//! key bits `u64`, offset `u64`, length `u32`), then a footer with the index offset `u64`,
//! the entry count `u32` and the encoding `u8`. All integers are little endian.
//!
//! Encodings:
//! - [`Encoding::Height`]: `f32` samples quantised to `u16` steps above a per-tile minimum,
//!   coded as zigzag residuals from a gradient predictor, split into byte planes, deflated.
//! - [`Encoding::Rgb`]: baseline JPEG.
//! - [`Encoding::Bytes`]: `channels` planes of `u8`, deflated.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Mutex,
};

use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};

use crate::cube::{TILE_SAMPLES, TileKey};

const MAGIC: &[u8; 8] = b"PHYPACK1";
const FOOTER_LEN: u64 = 13;
const SAMPLES: usize = TILE_SAMPLES * TILE_SAMPLES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Height,
    Rgb,
    Bytes { channels: u8 },
}

impl Encoding {
    fn to_u8(self) -> u8 {
        match self {
            Encoding::Height => 1,
            Encoding::Rgb => 2,
            Encoding::Bytes { channels } => 0x10 | channels,
        }
    }

    fn from_u8(v: u8) -> io::Result<Self> {
        Ok(match v {
            1 => Encoding::Height,
            2 => Encoding::Rgb,
            v if v & 0xf0 == 0x10 => Encoding::Bytes { channels: v & 0x0f },
            _ => return Err(invalid("unknown tile encoding")),
        })
    }
}

/// Decoded tile samples, `TILE_SAMPLES²` per channel, row-major (`j * TILE_SAMPLES + i`).
#[derive(Debug, Clone, PartialEq)]
pub enum TileData {
    Height(Vec<f32>),
    Rgb(Vec<[u8; 3]>),
    /// Planar: channel `c` occupies `data[c * SAMPLES..(c + 1) * SAMPLES]`.
    Bytes {
        channels: u8,
        data: Vec<u8>,
    },
}

impl TileData {
    pub fn heights(&self) -> &[f32] {
        match self {
            TileData::Height(h) => h,
            _ => &[],
        }
    }

    pub fn rgb(&self) -> &[[u8; 3]] {
        match self {
            TileData::Rgb(c) => c,
            _ => &[],
        }
    }

    pub fn channel(&self, channel: usize) -> &[u8] {
        match self {
            TileData::Bytes { data, .. } => &data[channel * SAMPLES..(channel + 1) * SAMPLES],
            _ => &[],
        }
    }

    fn encoding(&self) -> Encoding {
        match self {
            TileData::Height(_) => Encoding::Height,
            TileData::Rgb(_) => Encoding::Rgb,
            TileData::Bytes { channels, .. } => Encoding::Bytes {
                channels: *channels,
            },
        }
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
        match self {
            TileData::Height(h) => encode_height(h, 0.05),
            TileData::Rgb(c) => {
                assert_eq!(c.len(), SAMPLES);
                let mut out = Vec::new();
                let raw: Vec<u8> = c.iter().flatten().copied().collect();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
                    .encode(
                        &raw,
                        TILE_SAMPLES as u32,
                        TILE_SAMPLES as u32,
                        image::ExtendedColorType::Rgb8,
                    )
                    .map_err(io::Error::other)?;
                Ok(out)
            }
            TileData::Bytes { channels, data } => {
                assert_eq!(data.len(), SAMPLES * *channels as usize);
                deflate(data)
            }
        }
    }

    pub fn decode(encoding: Encoding, blob: &[u8]) -> io::Result<Self> {
        match encoding {
            Encoding::Height => {
                if blob.len() < 8 {
                    return Err(invalid("short height tile"));
                }
                let min = f32::from_le_bytes(blob[0..4].try_into().unwrap());
                let step = f32::from_le_bytes(blob[4..8].try_into().unwrap());
                let planes = inflate(&blob[8..], SAMPLES * 2)?;
                let (lo, hi) = planes.split_at(SAMPLES);
                let mut q = vec![0u16; SAMPLES];
                for k in 0..SAMPLES {
                    let zigzag = u16::from_le_bytes([lo[k], hi[k]]);
                    let residual = (zigzag >> 1) ^ (zigzag & 1).wrapping_neg();
                    q[k] = predict(&q, k).wrapping_add(residual);
                }
                Ok(TileData::Height(
                    q.into_iter().map(|v| min + v as f32 * step).collect(),
                ))
            }
            Encoding::Rgb => {
                let image = image::load_from_memory_with_format(blob, image::ImageFormat::Jpeg)
                    .map_err(io::Error::other)?
                    .into_rgb8();
                if image.width() as usize != TILE_SAMPLES || image.height() as usize != TILE_SAMPLES
                {
                    return Err(invalid("colour tile has the wrong size"));
                }
                Ok(TileData::Rgb(
                    image.pixels().map(|p| p.0).collect::<Vec<[u8; 3]>>(),
                ))
            }
            Encoding::Bytes { channels } => Ok(TileData::Bytes {
                channels,
                data: inflate(blob, SAMPLES * channels as usize)?,
            }),
        }
    }
}

/// Gradient predictor (left + up - up-left) over already decoded samples, in wrapping
/// `u16` arithmetic so encoder and decoder agree exactly.
fn predict(q: &[u16], k: usize) -> u16 {
    let (i, j) = (k % TILE_SAMPLES, k / TILE_SAMPLES);
    match (i, j) {
        (0, 0) => 0,
        (_, 0) => q[k - 1],
        (0, _) => q[k - TILE_SAMPLES],
        _ => q[k - 1]
            .wrapping_add(q[k - TILE_SAMPLES])
            .wrapping_sub(q[k - TILE_SAMPLES - 1]),
    }
}

/// Encode heights quantised to `precision` metres (or finer, if the tile's range needs
/// fewer than 65536 steps at that precision there is no loss beyond it).
pub fn encode_height(heights: &[f32], precision: f32) -> io::Result<Vec<u8>> {
    assert_eq!(heights.len(), SAMPLES);
    let (min, max) = heights
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let step = ((max - min) / 65535.0).max(precision).max(1e-4);
    let q: Vec<u16> = heights
        .iter()
        .map(|&v| ((v - min) / step).round().clamp(0.0, 65535.0) as u16)
        .collect();
    let mut lo = Vec::with_capacity(SAMPLES);
    let mut hi = Vec::with_capacity(SAMPLES);
    for k in 0..SAMPLES {
        let residual = q[k].wrapping_sub(predict(&q, k)) as i16;
        let zigzag = ((residual << 1) ^ (residual >> 15)) as u16;
        let [l, h] = zigzag.to_le_bytes();
        lo.push(l);
        hi.push(h);
    }
    let mut out = Vec::with_capacity(SAMPLES);
    out.extend(min.to_le_bytes());
    out.extend(step.to_le_bytes());
    out.extend(deflate(&[lo, hi].concat())?);
    Ok(out)
}

fn deflate(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(7));
    encoder.write_all(bytes)?;
    encoder.finish()
}

fn inflate(bytes: &[u8], expected: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    DeflateDecoder::new(bytes).read_to_end(&mut out)?;
    if out.len() != expected {
        return Err(invalid("tile has the wrong size"));
    }
    Ok(out)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Writes a pack. Tiles may be added in any order; [`PackWriter::finish`] writes the index.
pub struct PackWriter {
    file: BufWriter<File>,
    offset: u64,
    encoding: Option<Encoding>,
    index: Vec<(u64, u64, u32)>,
}

impl PackWriter {
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut file = BufWriter::new(File::create(path)?);
        file.write_all(MAGIC)?;
        Ok(Self {
            file,
            offset: MAGIC.len() as u64,
            encoding: None,
            index: Vec::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Add an already encoded tile (see [`TileData::encode`]), so encoding can run in
    /// parallel.
    pub fn write_encoded(
        &mut self,
        key: TileKey,
        encoding: Encoding,
        blob: &[u8],
    ) -> io::Result<()> {
        if *self.encoding.get_or_insert(encoding) != encoding {
            return Err(invalid("mixed encodings in one pack"));
        }
        self.file.write_all(blob)?;
        self.index
            .push((key.to_bits(), self.offset, blob.len() as u32));
        self.offset += blob.len() as u64;
        Ok(())
    }

    pub fn write(&mut self, key: TileKey, tile: &TileData) -> io::Result<()> {
        self.write_encoded(key, tile.encoding(), &tile.encode()?)
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.index.sort_unstable();
        let index_offset = self.offset;
        for (key, offset, len) in &self.index {
            self.file.write_all(&key.to_le_bytes())?;
            self.file.write_all(&offset.to_le_bytes())?;
            self.file.write_all(&len.to_le_bytes())?;
        }
        self.file.write_all(&index_offset.to_le_bytes())?;
        self.file
            .write_all(&(self.index.len() as u32).to_le_bytes())?;
        self.file
            .write_all(&[self.encoding.unwrap_or(Encoding::Height).to_u8()])?;
        self.file.flush()
    }
}

/// Random access to the tiles of a pack. Safe to share between threads.
pub struct PackReader {
    file: Mutex<File>,
    encoding: Encoding,
    index: HashMap<TileKey, (u64, u32)>,
    max_level: u8,
}

impl PackReader {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid("not a tile pack"));
        }
        file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
        let mut footer = [0u8; FOOTER_LEN as usize];
        file.read_exact(&mut footer)?;
        let index_offset = u64::from_le_bytes(footer[0..8].try_into().unwrap());
        let count = u32::from_le_bytes(footer[8..12].try_into().unwrap()) as usize;
        let encoding = Encoding::from_u8(footer[12])?;

        file.seek(SeekFrom::Start(index_offset))?;
        let mut raw = vec![0u8; count * 20];
        file.read_exact(&mut raw)?;
        let mut index = HashMap::with_capacity(count);
        let mut max_level = 0;
        for entry in raw.chunks_exact(20) {
            let key = TileKey::from_bits(u64::from_le_bytes(entry[0..8].try_into().unwrap()));
            let offset = u64::from_le_bytes(entry[8..16].try_into().unwrap());
            let len = u32::from_le_bytes(entry[16..20].try_into().unwrap());
            max_level = max_level.max(key.level);
            index.insert(key, (offset, len));
        }
        Ok(Self {
            file: Mutex::new(file),
            encoding,
            index,
            max_level,
        })
    }

    pub fn contains(&self, key: TileKey) -> bool {
        self.index.contains_key(&key)
    }

    pub fn max_level(&self) -> u8 {
        self.max_level
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn read(&self, key: TileKey) -> io::Result<Option<TileData>> {
        let Some(&(offset, len)) = self.index.get(&key) else {
            return Ok(None);
        };
        let mut blob = vec![0u8; len as usize];
        {
            let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut blob)?;
        }
        TileData::decode(self.encoding, &blob).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("osp_pack_{}_{name}", std::process::id()))
    }

    #[test]
    fn height_round_trip_is_within_quantisation() {
        let heights: Vec<f32> = (0..SAMPLES)
            .map(|k| ((k % 129) as f32 * 0.37).sin() * 3000.0 - 1200.0 + (k / 129) as f32)
            .collect();
        let tile = TileData::Height(heights.clone());
        let decoded = TileData::decode(Encoding::Height, &tile.encode().unwrap()).unwrap();
        let max_error = heights
            .iter()
            .zip(decoded.heights())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(max_error < 0.06, "{max_error}");
        // Coarser precision is honoured.
        let coarse =
            TileData::decode(Encoding::Height, &encode_height(&heights, 1.0).unwrap()).unwrap();
        let max_error = heights
            .iter()
            .zip(coarse.heights())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(max_error <= 0.5 + 1e-3, "{max_error}");
    }

    #[test]
    fn pack_round_trip() {
        let path = temp_path("bytes");
        let a = TileKey::root(2);
        let b = TileKey::containing(4, 0.2, 0.9, 7);
        let tile = |v: u8| TileData::Bytes {
            channels: 2,
            data: (0..SAMPLES * 2)
                .map(|k| (k as u8).wrapping_mul(v))
                .collect(),
        };
        let mut writer = PackWriter::create(&path).unwrap();
        writer.write(b, &tile(3)).unwrap();
        writer.write(a, &tile(5)).unwrap();
        writer.finish().unwrap();

        let reader = PackReader::open(&path).unwrap();
        assert_eq!(reader.len(), 2);
        assert_eq!(reader.max_level(), 7);
        assert_eq!(reader.read(a).unwrap().unwrap(), tile(5));
        assert_eq!(reader.read(b).unwrap().unwrap(), tile(3));
        assert!(reader.read(TileKey::root(0)).unwrap().is_none());
        std::fs::remove_file(path).ok();
    }
}
