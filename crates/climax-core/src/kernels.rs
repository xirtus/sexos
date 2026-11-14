//! Deterministic CPU and memory microbenchmarks:
//! ChaCha20/ARX integer ALU, SHA-256 compression rounds, tiled matrix multiply,
//! vectorizable DSP, STREAM memory bandwidth, and Sattolo pointer-chase latency ladder.

use alloc::vec::Vec;

/// Standard STREAM memory benchmark result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamResult {
    /// Number of operations completed.
    pub operations: u64,
    /// Total bytes read and written.
    pub bytes_processed: u64,
    /// Consistency checksum preventing compiler dead-code elimination.
    pub checksum: u64,
}

/// Latency ladder step across cache levels (L1, L2, L3, DRAM).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatencyStep {
    /// Working set size in bytes.
    pub working_set_bytes: usize,
    /// Total pointer hops executed.
    pub hops: usize,
    /// Checksum of visited node indices.
    pub checksum: usize,
}

/// ChaCha20 quarter-round ARX (Add-Rotate-Xor) loop.
///
/// Deterministic pure-integer throughput stress test.
pub fn bench_crypto_arx(iterations: usize, seed: u32) -> (u64, u64) {
    let mut state = [
        0x61707865 ^ seed, 0x3320646e, 0x79622d32, 0x6b206574,
        0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c,
        0x13121110, 0x17161514, 0x1b1a1918, 0x1f1e1d1c,
        0x00000001, seed, 0x00000002, 0x00000003,
    ];

    for _ in 0..iterations {
        // Column rounds
        macro_rules! qr {
            ($a:expr, $b:expr, $c:expr, $d:expr) => {
                state[$a] = state[$a].wrapping_add(state[$b]);
                state[$d] = (state[$d] ^ state[$a]).rotate_left(16);
                state[$c] = state[$c].wrapping_add(state[$d]);
                state[$b] = (state[$b] ^ state[$c]).rotate_left(12);
                state[$a] = state[$a].wrapping_add(state[$b]);
                state[$d] = (state[$d] ^ state[$a]).rotate_left(8);
                state[$c] = state[$c].wrapping_add(state[$d]);
                state[$b] = (state[$b] ^ state[$c]).rotate_left(7);
            };
        }

        qr!(0, 4, 8, 12);
        qr!(1, 5, 9, 13);
        qr!(2, 6, 10, 14);
        qr!(3, 7, 11, 15);

        // Diagonal rounds
        qr!(0, 5, 10, 15);
        qr!(1, 6, 11, 12);
        qr!(2, 7, 8, 13);
        qr!(3, 4, 9, 14);
    }

    // Compute checksum over state
    let mut checksum = 0u64;
    for &val in &state {
        checksum = checksum.wrapping_mul(6364136223846793005).wrapping_add(val as u64);
    }

    // 16 ARX quarter rounds per iteration, 8 ALU ops per QR = 128 ops / iteration
    let ops = (iterations as u64) * 128;
    (ops, checksum)
}

/// Pure-Rust SHA-256 compression rounds.
pub fn bench_sha256(iterations: usize, seed: u32) -> (u64, u64) {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0x0bef9a3f, 0xc67178f2,
    ];

    let mut h = [
        0x6a09e667 ^ seed, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = (seed.wrapping_mul((i as u32) + 1)).rotate_left((i as u32) % 31);
    }

    for _ in 0..iterations {
        // Message schedule expansion
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut h_val = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h_val
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h_val = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_val);

        // Mutate w[0] so consecutive iterations depend on previous output
        w[0] = w[0].wrapping_add(h[0]);
    }

    let mut checksum = 0u64;
    for &val in &h {
        checksum = checksum.wrapping_mul(31).wrapping_add(val as u64);
    }

    let ops = (iterations as u64) * 64 * 10;
    (ops, checksum)
}

/// FMA-free 4x4 matrix multiplication kernel (pure addition & multiplication).
#[inline(always)]
pub fn matmul4x4_f32(a: &[f32; 16], b: &[f32; 16], c: &mut [f32; 16]) {
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                // Explicitly avoid FMA to guarantee bit-identical softfloat and SSE2
                let prod = a[i * 4 + k] * b[k * 4 + j];
                sum += prod;
            }
            c[i * 4 + j] += sum;
        }
    }
}

/// Tiled 64x64 matrix multiplication (FMA-free, SSE2-compatible).
pub fn bench_tiled_gemm_f32(dim: usize, iterations: usize) -> (u64, u64) {
    let size = dim.checked_mul(dim).unwrap_or(0);
    if size == 0 {
        return (0, 0);
    }
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    if a.try_reserve_exact(size).is_err()
        || b.try_reserve_exact(size).is_err()
        || c.try_reserve_exact(size).is_err()
    {
        return (0, 0);
    }
    a.resize(size, 0.0f32);
    b.resize(size, 0.0f32);
    c.resize(size, 0.0f32);

    for i in 0..size {
        a[i] = ((i % 17) as f32) * 0.1;
        b[i] = ((i % 13) as f32) * 0.1;
    }

    let tile_size = 8;
    for _ in 0..iterations {
        for i_tile in (0..dim).step_by(tile_size) {
            let i_max = (i_tile + tile_size).min(dim);
            for j_tile in (0..dim).step_by(tile_size) {
                let j_max = (j_tile + tile_size).min(dim);
                for k_tile in (0..dim).step_by(tile_size) {
                    let k_max = (k_tile + tile_size).min(dim);

                    for i in i_tile..i_max {
                        for k in k_tile..k_max {
                            let a_ik = a[i * dim + k];
                            for j in j_tile..j_max {
                                let prod = a_ik * b[k * dim + j];
                                c[i * dim + j] += prod;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut checksum = 0u64;
    for &val in &c {
        checksum = checksum.wrapping_add(val.to_bits() as u64);
    }

    // 2 * N^3 FLOPs per iteration
    let flops = (iterations as u64) * 2 * (dim as u64) * (dim as u64) * (dim as u64);
    (flops, checksum)
}

/// Vectorizable DSP audio biquad IIR filter stress test.
pub fn bench_dsp_f32(samples: usize, iterations: usize) -> (u64, u64) {
    if samples == 0 {
        return (0, 0);
    }
    let mut signal = Vec::new();
    if signal.try_reserve_exact(samples).is_err() {
        return (0, 0);
    }
    signal.resize(samples, 0.0f32);

    for i in 0..samples {
        signal[i] = if i % 2 == 0 { 0.5 } else { -0.5 };
    }

    // Normalized biquad lowpass coefficients
    let b0 = 0.2929f32;
    let b1 = 0.5858f32;
    let b2 = 0.2929f32;
    let a1 = -0.0000f32;
    let a2 = 0.1716f32;

    for _ in 0..iterations {
        let mut d1 = 0.0f32;
        let mut d2 = 0.0f32;
        for i in 0..samples {
            let x = signal[i];
            let y = b0 * x + d1;
            d1 = b1 * x - a1 * y + d2;
            d2 = b2 * x - a2 * y;
            signal[i] = y;
        }
    }

    let mut checksum = 0u64;
    for &val in &signal {
        checksum = checksum.wrapping_add(val.to_bits() as u64);
    }

    let ops = (iterations as u64) * (samples as u64) * 8;
    (ops, checksum)
}

/// STREAM memory bandwidth benchmark (copy, scale, add, triad) with explicit memory quota.
pub fn bench_stream_with_quota(
    len: usize,
    iterations: usize,
    max_memory_bytes: usize,
) -> StreamResult {
    if len == 0 || max_memory_bytes == 0 {
        return StreamResult {
            operations: 0,
            bytes_processed: 0,
            checksum: 0,
        };
    }

    // 3 arrays of f32: a, b, c. Total bytes = len * 4 * 3 = 12 * len.
    let bytes_per_element = core::mem::size_of::<f32>() * 3;
    let max_len = max_memory_bytes / bytes_per_element;
    let actual_len = len.min(max_len);
    if actual_len == 0 {
        return StreamResult {
            operations: 0,
            bytes_processed: 0,
            checksum: 0,
        };
    }

    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();

    if a.try_reserve_exact(actual_len).is_err()
        || b.try_reserve_exact(actual_len).is_err()
        || c.try_reserve_exact(actual_len).is_err()
    {
        return StreamResult {
            operations: 0,
            bytes_processed: 0,
            checksum: 0,
        };
    }

    a.resize(actual_len, 1.0f32);
    b.resize(actual_len, 2.0f32);
    c.resize(actual_len, 0.0f32);
    let q = 3.0f32;

    for _ in 0..iterations {
        // 1. Copy: c[i] = a[i]
        for i in 0..actual_len {
            c[i] = a[i];
        }
        // 2. Scale: b[i] = q * c[i]
        for i in 0..actual_len {
            b[i] = q * c[i];
        }
        // 3. Add: c[i] = a[i] + b[i]
        for i in 0..actual_len {
            c[i] = a[i] + b[i];
        }
        // 4. Triad: a[i] = b[i] + q * c[i]
        for i in 0..actual_len {
            a[i] = b[i] + q * c[i];
        }
    }

    let mut checksum = 0u64;
    for &val in &a {
        checksum = checksum.wrapping_add(val.to_bits() as u64);
    }

    // Bytes: Copy: 2 reads/writes * len * 4; Scale: 2 * len * 4; Add: 3 * len * 4; Triad: 3 * len * 4
    // Total = 10 * len * 4 bytes per iteration
    let bytes = (iterations as u64) * (actual_len as u64) * 40;
    let ops = (iterations as u64) * (actual_len as u64) * 4;

    StreamResult {
        operations: ops,
        bytes_processed: bytes,
        checksum,
    }
}

/// Standard STREAM memory bandwidth benchmark (defaults to 32 MiB max quota).
pub fn bench_stream(len: usize, iterations: usize) -> StreamResult {
    bench_stream_with_quota(len, iterations, 32 * 1024 * 1024)
}

/// Pseudo-random single-cycle Sattolo permutation generator for cache/TLB latency tests.
pub fn generate_sattolo_cycle(len: usize, seed: u64) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    if len == 1 {
        let mut p = Vec::new();
        if p.try_reserve_exact(1).is_ok() {
            p.push(0);
        }
        return p;
    }

    let mut p = Vec::new();
    if p.try_reserve_exact(len).is_err() {
        return Vec::new();
    }
    p.extend(0..len);

    let mut rng = seed.wrapping_add(0x9e3779b97f4a7c15);

    // Sattolo's algorithm: i goes from len - 1 down to 1; j in 0..i (exclusive of i)
    for i in (1..len).rev() {
        // Fast deterministic 64-bit LCG
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (rng as usize) % i;
        p.swap(i, j);
    }

    p
}

/// Cache & TLB Latency Pointer-Chase Ladder.
///
/// Sweeps across working set sizes (from 4 KiB L1 to 32 MiB DRAM) respecting memory quota.
pub fn bench_pointer_chase_ladder(
    quota_bytes: usize,
    hops_per_step: usize,
) -> Vec<LatencyStep> {
    // Working set sizes: 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 8 MiB, 32 MiB
    let target_sizes = [
        4 * 1024,
        16 * 1024,
        64 * 1024,
        256 * 1024,
        1024 * 1024,
        4 * 1024 * 1024,
        8 * 1024 * 1024,
        32 * 1024 * 1024,
    ];

    let mut results = Vec::new();

    for &size in &target_sizes {
        if size > quota_bytes {
            continue;
        }

        let num_elements = size / core::mem::size_of::<usize>();
        if num_elements < 2 {
            continue;
        }

        let chain = generate_sattolo_cycle(num_elements, 0x1337_cafe_babe);
        if chain.len() < num_elements {
            break;
        }

        // Chase pointers
        let mut cur = 0usize;
        let mut sum = 0usize;

        for _ in 0..hops_per_step {
            cur = chain[cur];
            sum = sum.wrapping_add(cur);
        }

        if results.try_reserve(1).is_err() {
            break;
        }

        results.push(LatencyStep {
            working_set_bytes: size,
            hops: hops_per_step,
            checksum: sum,
        });
    }

    results
}
