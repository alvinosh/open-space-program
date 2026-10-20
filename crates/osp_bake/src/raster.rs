//! Georeferenced latitude/longitude rasters with mip levels, and sampling by direction.

use rayon::prelude::*;

/// Where a raster sits on the globe. Pixel `(x, y)` has its centre at
/// `lon = lon0 + (x + center) * dlon`, `lat = lat0 - (y + center) * dlat`, with `center`
/// 0.5 for pixel-is-area grids and 0.0 for pixel-is-point grids.
#[derive(Debug, Clone, Copy)]
pub struct Geo {
    pub lon0: f64,
    pub lat0: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub center: f64,
    /// Longitude wraps around (global rasters).
    pub wrap: bool,
}

impl Geo {
    /// A pixel-is-area grid spanning the whole globe.
    pub fn global(width: usize, height: usize) -> Self {
        Self {
            lon0: -180.0,
            lat0: 90.0,
            dlon: 360.0 / width as f64,
            dlat: 180.0 / height as f64,
            center: 0.5,
            wrap: true,
        }
    }
}

pub struct Raster<T> {
    pub width: usize,
    pub height: usize,
    pub data: Vec<T>,
}

impl<T: Copy + Send + Sync> Raster<T> {
    pub fn new(width: usize, height: usize, data: Vec<T>) -> Self {
        assert_eq!(data.len(), width * height);
        Self {
            width,
            height,
            data,
        }
    }

    fn get(&self, x: isize, y: isize, wrap: bool) -> T {
        let x = if wrap {
            x.rem_euclid(self.width as isize)
        } else {
            x.clamp(0, self.width as isize - 1)
        } as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.data[y * self.width + x]
    }

    /// Half-resolution copy, combining each 2×2 block with `combine`.
    fn downsample(&self, combine: &(impl Fn([T; 4]) -> T + Sync)) -> Self {
        let width = self.width.div_ceil(2);
        let height = self.height.div_ceil(2);
        let mut data = Vec::with_capacity(width * height);
        data.par_extend((0..height).into_par_iter().flat_map_iter(|y| {
            (0..width).map(move |x| {
                let (x2, y2) = (2 * x as isize, 2 * y as isize);
                combine([
                    self.get(x2, y2, false),
                    self.get(x2 + 1, y2, false),
                    self.get(x2, y2 + 1, false),
                    self.get(x2 + 1, y2 + 1, false),
                ])
            })
        }));
        Self {
            width,
            height,
            data,
        }
    }
}

/// A raster and its successively halved mip levels.
pub struct Pyramid<T> {
    pub geo: Geo,
    pub levels: Vec<Raster<T>>,
}

impl<T: Copy + Send + Sync> Pyramid<T> {
    pub fn new(geo: Geo, base: Raster<T>, combine: impl Fn([T; 4]) -> T + Sync) -> Self {
        let mut levels = vec![base];
        while levels
            .last()
            .unwrap()
            .width
            .min(levels.last().unwrap().height)
            > 1
        {
            let next = levels.last().unwrap().downsample(&combine);
            levels.push(next);
        }
        Self { geo, levels }
    }

    /// Mip level whose pixels are just finer than `texel_deg`.
    pub fn level_for(&self, texel_deg: f64) -> usize {
        let ratio = texel_deg / self.geo.dlat;
        if ratio <= 1.0 {
            0
        } else {
            (ratio.log2().floor() as usize).min(self.levels.len() - 1)
        }
    }

    /// Continuous pixel coordinates at mip `level`.
    fn coords(&self, level: usize, lat: f64, lon: f64) -> (f64, f64) {
        let g = &self.geo;
        let mut lon = lon;
        if g.wrap {
            lon = (lon - g.lon0).rem_euclid(360.0) + g.lon0;
        }
        let x0 = (lon - g.lon0) / g.dlon - g.center;
        let y0 = (g.lat0 - lat) / g.dlat - g.center;
        let scale = (1u64 << level) as f64;
        ((x0 + 0.5) / scale - 0.5, (y0 + 0.5) / scale - 0.5)
    }

    pub fn nearest(&self, level: usize, lat: f64, lon: f64) -> T {
        let (x, y) = self.coords(level, lat, lon);
        self.levels[level].get(x.round() as isize, y.round() as isize, self.geo.wrap)
    }

    pub fn bilinear<const K: usize>(
        &self,
        level: usize,
        lat: f64,
        lon: f64,
        channels: impl Fn(T) -> [f32; K],
    ) -> [f32; K] {
        let raster = &self.levels[level];
        let (x, y) = self.coords(level, lat, lon);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let wrap = self.geo.wrap;
        let a = channels(raster.get(x0, y0, wrap));
        let b = channels(raster.get(x0 + 1, y0, wrap));
        let c = channels(raster.get(x0, y0 + 1, wrap));
        let d = channels(raster.get(x0 + 1, y0 + 1, wrap));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }
}

pub fn mean_f32(v: [f32; 4]) -> f32 {
    (v[0] + v[1] + v[2] + v[3]) * 0.25
}

pub fn mean_u8(v: [u8; 4]) -> u8 {
    ((v.iter().map(|&x| x as u32).sum::<u32>() + 2) / 4) as u8
}

pub fn mean_rgb(v: [[u8; 3]; 4]) -> [u8; 3] {
    std::array::from_fn(|c| mean_u8([v[0][c], v[1][c], v[2][c], v[3][c]]))
}

/// Most frequent of four class values (the first one on ties).
pub fn mode_u8(v: [u8; 4]) -> u8 {
    let mut best = v[0];
    let mut best_count = 0;
    for &candidate in &v {
        let count = v.iter().filter(|&&x| x == candidate).count();
        if count > best_count {
            best = candidate;
            best_count = count;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_coordinates_and_mips_line_up() {
        // 8×4 grid whose value is the column index: sampling at a pixel centre returns it.
        let base = Raster::new(8, 4, (0..32).map(|k| (k % 8) as f32).collect());
        let pyramid = Pyramid::new(Geo::global(8, 4), base, mean_f32);
        let lon_of = |x: f64| -180.0 + (x + 0.5) * 45.0;
        assert_eq!(pyramid.bilinear(0, 10.0, lon_of(3.0), |v| [v])[0], 3.0);
        // Halfway between columns 3 and 4.
        assert_eq!(pyramid.bilinear(0, 10.0, lon_of(3.5), |v| [v])[0], 3.5);
        // Wraps across the antimeridian.
        assert_eq!(pyramid.bilinear(0, 10.0, lon_of(7.5), |v| [v])[0], 3.5);
        // Mip 1 pixel 1 averages columns 2 and 3 and is centred between them.
        assert_eq!(pyramid.bilinear(1, 10.0, lon_of(2.5), |v| [v])[0], 2.5);
    }

    #[test]
    fn mode_prefers_majority() {
        assert_eq!(mode_u8([3, 5, 5, 1]), 5);
        assert_eq!(mode_u8([3, 5, 1, 2]), 3);
    }
}
