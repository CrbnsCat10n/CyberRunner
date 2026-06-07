const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

pub struct PythonRandom {
    state: [u32; N],
    index: usize,
}

impl PythonRandom {
    pub fn new(seed: u64) -> Self {
        let key = seed_key(seed);
        let mut rng = Self {
            state: [0; N],
            index: N,
        };
        rng.init_by_array(&key);
        rng
    }

    pub fn random(&mut self) -> f64 {
        let a = (self.gen_u32() >> 5) as u64;
        let b = (self.gen_u32() >> 6) as u64;
        ((a * 67_108_864 + b) as f64) / 9_007_199_254_740_992.0
    }

    pub fn uniform(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.random()
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..N {
            self.state[i] = 1_812_433_253_u32
                .wrapping_mul(self.state[i - 1] ^ (self.state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        self.index = N;
    }

    fn init_by_array(&mut self, init_key: &[u32]) {
        self.init_genrand(19_650_218);
        let key_length = init_key.len();
        let mut i = 1;
        let mut j = 0;
        for k in (0..N.max(key_length)).rev() {
            let value = (self.state[i]
                ^ ((self.state[i - 1] ^ (self.state[i - 1] >> 30)).wrapping_mul(1_664_525)))
            .wrapping_add(init_key[j])
            .wrapping_add(j as u32);
            self.state[i] = value;
            i += 1;
            j += 1;
            if i >= N {
                self.state[0] = self.state[N - 1];
                i = 1;
            }
            if j >= key_length {
                j = 0;
            }
            let _ = k;
        }
        for _ in (1..N).rev() {
            let value = (self.state[i]
                ^ ((self.state[i - 1] ^ (self.state[i - 1] >> 30)).wrapping_mul(1_566_083_941)))
            .wrapping_sub(i as u32);
            self.state[i] = value;
            i += 1;
            if i >= N {
                self.state[0] = self.state[N - 1];
                i = 1;
            }
        }
        self.state[0] = UPPER_MASK;
    }

    fn gen_u32(&mut self) -> u32 {
        if self.index >= N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    fn twist(&mut self) {
        for kk in 0..(N - M) {
            let y = (self.state[kk] & UPPER_MASK) | (self.state[kk + 1] & LOWER_MASK);
            self.state[kk] = self.state[kk + M] ^ (y >> 1) ^ mag01(y);
        }
        for kk in (N - M)..(N - 1) {
            let y = (self.state[kk] & UPPER_MASK) | (self.state[kk + 1] & LOWER_MASK);
            self.state[kk] = self.state[kk + M - N] ^ (y >> 1) ^ mag01(y);
        }
        let y = (self.state[N - 1] & UPPER_MASK) | (self.state[0] & LOWER_MASK);
        self.state[N - 1] = self.state[M - 1] ^ (y >> 1) ^ mag01(y);
        self.index = 0;
    }
}

fn mag01(value: u32) -> u32 {
    if value & 1 == 0 {
        0
    } else {
        MATRIX_A
    }
}

fn seed_key(seed: u64) -> Vec<u32> {
    if seed == 0 {
        return vec![0];
    }
    let mut value = seed;
    let mut key = Vec::new();
    while value > 0 {
        key.push((value & 0xffff_ffff) as u32);
        value >>= 32;
    }
    key
}

#[cfg(test)]
mod tests {
    use super::PythonRandom;

    #[test]
    fn matches_python_random_for_common_seeds() {
        let expected_20260601 = [
            0.2285368365355147,
            0.5196877203375084,
            0.013412740325089656,
            0.2292213628427009,
            0.8520307240445778,
        ];
        let mut rng = PythonRandom::new(20260601);
        for expected in expected_20260601 {
            assert!((rng.random() - expected).abs() < f64::EPSILON);
        }

        let expected_zero = [
            0.8444218515250481,
            0.7579544029403025,
            0.420571580830845,
            0.25891675029296335,
            0.5112747213686085,
        ];
        let mut rng = PythonRandom::new(0);
        for expected in expected_zero {
            assert!((rng.random() - expected).abs() < f64::EPSILON);
        }
    }
}
