//! Map generation kernels.
//!
//! * [`perlin_batch`] evaluates Unciv's (Rosetta-code) Perlin / ridged Perlin octave noise for a
//!   whole map in one call. Kotlin passes the already-projected `x/y/z` sample coordinates
//!   (the cylinder projection uses `sin`/`cos`, which we deliberately keep on the JVM side so the
//!   transcendental functions are evaluated by the exact same implementation as vanilla). The
//!   remaining arithmetic is pure IEEE-754 `+ - * / floor` which Rust evaluates bit-identically
//!   (no fused multiply-add is ever emitted implicitly), so generated maps are identical to vanilla
//!   for the same seed.
//! * [`assign_continents`] replicates `TileMap.assignContinents` (including the
//!   "seed from the middle of the remaining land list" ordering, so continent IDs are identical)
//!   in O(N * continents) with flat arrays instead of O(N^2) list filtering.

const PERMUTATION: [i32; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142,
    8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117,
    35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71,
    134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41,
    55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89,
    18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226,
    250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182,
    189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43,
    172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97,
    228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239,
    107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254,
    138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];

const GRAD3: [[i32; 3]; 16] = [
    [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
    [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
    [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
    [1, 0, -1], [-1, 0, -1], [0, -1, 1], [0, 1, 1],
];

#[inline(always)]
fn p(i: i32) -> i32 {
    PERMUTATION[(i & 255) as usize]
}

#[inline(always)]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline(always)]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

#[inline(always)]
fn grad3(hash: i32, x: f64, y: f64, z: f64) -> f64 {
    let g = &GRAD3[(hash & 15) as usize];
    // Kotlin: x * grad3[h][0] + y * grad3[h][1] + z * grad3[h][2]  (Int -> Double widening)
    x * g[0] as f64 + y * g[1] as f64 + z * g[2] as f64
}

/// Kotlin `floor(x).toInt()`: saturating conversion, NaN -> 0 (Rust `as` has the same semantics).
#[inline(always)]
fn floor_to_int(x: f64) -> i32 {
    x.floor() as i32
}

/// Exact port of `Perlin.noise`.
pub fn noise(x: f64, y: f64, z: f64) -> f64 {
    let xi = floor_to_int(x) & 255;
    let yi = floor_to_int(y) & 255;
    let zi = floor_to_int(z) & 255;
    let xx = x - x.floor();
    let yy = y - y.floor();
    let zz = z - z.floor();
    let u = fade(xx);
    let v = fade(yy);
    let w = fade(zz);
    // p has 512 entries in Kotlin (permutation repeated twice); all indices below are < 512.
    let pp = |i: i32| -> i32 { if i < 256 { PERMUTATION[i as usize] } else { p(i - 256) } };
    let a = pp(xi) + yi;
    let aa = pp(a) + zi;
    let ab = pp(a + 1) + zi;
    let b = pp(xi + 1) + yi;
    let ba = pp(b) + zi;
    let bb = pp(b + 1) + zi;

    lerp(
        w,
        lerp(
            v,
            lerp(u, grad3(pp(aa), xx, yy, zz), grad3(pp(ba), xx - 1.0, yy, zz)),
            lerp(u, grad3(pp(ab), xx, yy - 1.0, zz), grad3(pp(bb), xx - 1.0, yy - 1.0, zz)),
        ),
        lerp(
            v,
            lerp(u, grad3(pp(aa + 1), xx, yy, zz - 1.0), grad3(pp(ba + 1), xx - 1.0, yy, zz - 1.0)),
            lerp(
                u,
                grad3(pp(ab + 1), xx, yy - 1.0, zz - 1.0),
                grad3(pp(bb + 1), xx - 1.0, yy - 1.0, zz - 1.0),
            ),
        ),
    )
}

/// Exact port of `Perlin.noise3d` (`ridged == false`) / `Perlin.ridgedNoise3d` (`ridged == true`).
pub fn octave_noise(x: f64, y: f64, z: f64, n_octaves: i32, persistence: f64, lacunarity: f64, scale: f64, ridged: bool) -> f64 {
    let mut freq = 1.0f64;
    let mut amp = 1.0f64;
    let mut max = 0.0f64;
    let mut total = 0.0f64;
    for _ in 0..n_octaves.max(0) {
        let mut value = noise(x * freq / scale, y * freq / scale, z * freq / scale);
        if ridged {
            value = value.abs();
        }
        total += amp * value;
        max += amp;
        freq *= lacunarity;
        amp *= persistence;
    }
    total / max
}

/// Evaluate octave noise for every sample point. `xs`, `ys`, `zs` must have equal length.
pub fn perlin_batch(xs: &[f64], ys: &[f64], zs: &[f64], n_octaves: i32, persistence: f64, lacunarity: f64, scale: f64, ridged: bool, out: &mut [f64]) {
    let n = xs.len().min(ys.len()).min(zs.len()).min(out.len());
    for i in 0..n {
        out[i] = octave_noise(xs[i], ys[i], zs[i], n_octaves, persistence, lacunarity, scale, ridged);
    }
}

/// Replicates `TileMap.assignContinents(Assign/Reassign)`.
///
/// * `neighbors[t * 6 + k]` = index of the k-th neighbor of tile `t` (or -1), in the same order
///   as `Tile.neighbors` (order is irrelevant for the result, only connectivity matters).
/// * `passable_land[t]` = `tile.isLand && !tile.isImpassible()`.
///
/// Writes the continent id (or -1) for every tile into `out_continent` and returns the
/// continent sizes in id order.
pub fn assign_continents(neighbors: &[i32], passable_land: &[bool], out_continent: &mut [i32]) -> Vec<i32> {
    let n = passable_land.len();
    for c in out_continent.iter_mut() {
        *c = -1;
    }
    // `landTiles` keeps the original tile-list order; vanilla seeds each BFS from the middle element
    // of the *remaining* land list, so we keep a compacted list with identical ordering.
    let mut remaining: Vec<u32> = (0..n as u32).filter(|&t| passable_land[t as usize]).collect();
    let mut sizes = Vec::new();
    let mut queue: Vec<u32> = Vec::with_capacity(n);
    let mut current = 0i32;
    while !remaining.is_empty() {
        let seed = remaining[remaining.len() / 2];
        queue.clear();
        queue.push(seed);
        out_continent[seed as usize] = current;
        let mut head = 0;
        while head < queue.len() {
            let t = queue[head] as usize;
            head += 1;
            for k in 0..6 {
                let nb = neighbors[t * 6 + k];
                if nb < 0 {
                    continue;
                }
                let nb = nb as usize;
                if passable_land[nb] && out_continent[nb] == -1 {
                    out_continent[nb] = current;
                    queue.push(nb as u32);
                }
            }
        }
        sizes.push(queue.len() as i32);
        remaining.retain(|&t| out_continent[t as usize] == -1);
        current += 1;
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_reference_values() {
        // Integer lattice points are always 0 for gradient noise.
        assert_eq!(noise(0.0, 0.0, 0.0), 0.0);
        assert_eq!(noise(3.0, 7.0, 11.0), 0.0);
        // Values must stay in the theoretical range.
        let mut i = 0.0;
        while i < 50.0 {
            let v = noise(i * 0.37, i * 0.11 - 3.0, i * 0.73 + 0.5);
            assert!(v.abs() <= 1.0001, "{v}");
            i += 0.5;
        }
    }

    #[test]
    fn octave_matches_manual_sum() {
        let (x, y, z) = (12.3, -4.56, 7.89);
        let manual = (noise(x / 30.0, y / 30.0, z / 30.0) + 0.5 * noise(x * 2.0 / 30.0, y * 2.0 / 30.0, z * 2.0 / 30.0)) / 1.5;
        assert_eq!(octave_noise(x, y, z, 2, 0.5, 2.0, 30.0, false), manual);
    }

    #[test]
    fn continents_match_reference_ordering() {
        // 1D strip of 9 tiles: L L W L L L W W L  (neighbors = left/right only)
        let land = [true, true, false, true, true, true, false, false, true];
        let n = land.len();
        let mut nb = vec![-1i32; n * 6];
        for t in 0..n {
            if t > 0 { nb[t * 6] = (t - 1) as i32; }
            if t + 1 < n { nb[t * 6 + 1] = (t + 1) as i32; }
        }
        let mut out = vec![0; n];
        let sizes = assign_continents(&nb, &land, &mut out);
        // remaining land list = [0,1,3,4,5,8], middle (index 3) = tile 4 => continent 0 = {3,4,5}
        // remaining = [0,1,8], middle = tile 1 => continent 1 = {0,1}; then tile 8 => continent 2
        assert_eq!(out, vec![1, 1, -1, 0, 0, 0, -1, -1, 2]);
        assert_eq!(sizes, vec![3, 2, 1]);
    }
}
