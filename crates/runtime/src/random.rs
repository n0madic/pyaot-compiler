//! random module runtime functions
//!
//! Uses Mersenne Twister (MT19937) to match CPython's `random` module exactly.
//! Seeding with the same value produces identical sequences to CPython.

use crate::object::{ListObj, Obj, TypeTagKind};
use std::cell::RefCell;

// ==================== Mersenne Twister (MT19937) ====================

const MT_N: usize = 624;
const MT_M: usize = 397;
const MATRIX_A: u32 = 0x9908b0df;
const UPPER_MASK: u32 = 0x80000000;
const LOWER_MASK: u32 = 0x7fffffff;

/// CPython-compatible Mersenne Twister RNG
struct MersenneTwister {
    mt: [u32; MT_N],
    mti: usize,
    /// Cached second value from Box-Muller gauss (matches CPython's _gauss_next)
    gauss_next: f64,
    /// Whether gauss_next holds a valid cached value
    gauss_has_next: bool,
}

impl MersenneTwister {
    /// Initialize from a single u32 seed (CPython's init_genrand)
    fn init_genrand(seed: u32) -> Self {
        let mut mt = [0u32; MT_N];
        mt[0] = seed;
        for i in 1..MT_N {
            mt[i] = 1812433253u32
                .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        MersenneTwister {
            mt,
            mti: MT_N + 1,
            gauss_next: 0.0,
            gauss_has_next: false,
        }
    }

    /// Initialize from key array (CPython's init_by_array)
    fn init_by_array(key: &[u32]) -> Self {
        let mut state = Self::init_genrand(19650218);
        let mt = &mut state.mt;

        let mut i = 1usize;
        let mut j = 0usize;
        let k = MT_N.max(key.len());

        for _ in 0..k {
            mt[i] = (mt[i] ^ ((mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1664525)))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= MT_N {
                mt[0] = mt[MT_N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }

        for _ in 0..(MT_N - 1) {
            mt[i] = (mt[i] ^ ((mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1566083941)))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= MT_N {
                mt[0] = mt[MT_N - 1];
                i = 1;
            }
        }
        mt[0] = 0x80000000; // MSB is 1; assuring non-zero initial array
        state
    }

    /// Seed from a Python integer (matches CPython's random_seed for int args)
    fn seed_from_int(n: i64) -> Self {
        let n_abs = (n.unsigned_abs()) as u128;
        if n_abs == 0 {
            // seed(0) → init_by_array(&[0])
            return Self::init_by_array(&[0]);
        }

        // Convert to little-endian u32 words (matching CPython's _PyLong_AsByteArray)
        let mut key = Vec::new();
        let mut val = n_abs;
        while val > 0 {
            key.push((val & 0xFFFF_FFFF) as u32);
            val >>= 32;
        }

        // Strip trailing zeros (CPython behavior)
        while key.len() > 1 && *key.last().unwrap() == 0 {
            key.pop();
        }

        Self::init_by_array(&key)
    }

    /// Generate a random u32 (CPython's genrand_uint32)
    fn genrand_uint32(&mut self) -> u32 {
        let mag01 = [0u32, MATRIX_A];

        if self.mti >= MT_N {
            // Generate MT_N words at one time
            for kk in 0..(MT_N - MT_M) {
                let y = (self.mt[kk] & UPPER_MASK) | (self.mt[kk + 1] & LOWER_MASK);
                self.mt[kk] = self.mt[kk + MT_M] ^ (y >> 1) ^ mag01[(y & 1) as usize];
            }
            for kk in (MT_N - MT_M)..(MT_N - 1) {
                let y = (self.mt[kk] & UPPER_MASK) | (self.mt[kk + 1] & LOWER_MASK);
                self.mt[kk] = self.mt[kk + MT_M - MT_N] ^ (y >> 1) ^ mag01[(y & 1) as usize];
            }
            let y = (self.mt[MT_N - 1] & UPPER_MASK) | (self.mt[0] & LOWER_MASK);
            self.mt[MT_N - 1] = self.mt[MT_M - 1] ^ (y >> 1) ^ mag01[(y & 1) as usize];
            self.mti = 0;
        }

        let mut y = self.mt[self.mti];
        self.mti += 1;

        // Tempering
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;

        y
    }

    /// Generate float in [0.0, 1.0) with 53-bit precision (CPython's genrand_res53)
    fn random(&mut self) -> f64 {
        let a = (self.genrand_uint32() >> 5) as f64; // 27 bits
        let b = (self.genrand_uint32() >> 6) as f64; // 26 bits
        (a * 67108864.0 + b) * (1.0 / 9007199254740992.0)
    }

    /// Generate random bits (CPython's getrandbits, limited to 64 bits)
    fn getrandbits(&mut self, num_bits: u32) -> u64 {
        if num_bits == 0 {
            return 0;
        }
        if num_bits <= 32 {
            return (self.genrand_uint32() >> (32 - num_bits)) as u64;
        }

        // For > 32 bits: generate words LSB-first (matching CPython)
        let words = ((num_bits as usize) - 1) / 32 + 1;
        let mut result = 0u64;
        let mut remaining = num_bits;
        for word_idx in 0..words {
            let r = self.genrand_uint32();
            let val = if remaining < 32 {
                r >> (32 - remaining)
            } else {
                r
            };
            result |= (val as u64) << (word_idx * 32);
            remaining = remaining.saturating_sub(32);
        }
        result
    }

    /// Generate random integer in [0, n) using getrandbits (CPython's _randbelow_with_getrandbits)
    fn randbelow(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let k = 64 - ((n as u64).leading_zeros()); // bit_length of n
        let mut r = self.getrandbits(k);
        while r >= n as u64 {
            r = self.getrandbits(k);
        }
        r as usize
    }

    /// Gaussian distribution with caching (matches CPython's random.gauss exactly)
    fn gauss(&mut self, mu: f64, sigma: f64) -> f64 {
        if self.gauss_has_next {
            let z = self.gauss_next;
            self.gauss_has_next = false;
            return mu + sigma * z;
        }
        // Box-Muller transform (matching CPython's parameter order)
        let x2pi = self.random() * std::f64::consts::TAU;
        let g2rad = (-2.0 * (1.0 - self.random()).ln()).sqrt();
        let z = x2pi.cos() * g2rad;
        self.gauss_next = x2pi.sin() * g2rad;
        self.gauss_has_next = true;
        mu + sigma * z
    }

    /// Create from system entropy (for default initialization)
    fn from_entropy() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);
        // Mix with process ID for additional entropy so two processes starting
        // at the same nanosecond still get different sequences.
        let pid = std::process::id() as u64;
        let seed = seed ^ pid ^ (pid << 32);
        let key = [seed as u32, (seed >> 32) as u32];
        Self::init_by_array(&key)
    }
}

// ==================== Thread-local RNG ====================

thread_local! {
    static RNG: RefCell<MersenneTwister> = RefCell::new(MersenneTwister::from_entropy());
}

// ==================== Public runtime functions ====================

/// random.seed(n, arg_count) - seed the RNG.
///
/// `arg_count` is the number of arguments the Python caller supplied:
/// - `arg_count == 0`: seed() or seed(None) — use system entropy
/// - `arg_count == 1`: seed(n) — seed deterministically with `n`
///
/// Using an explicit arg_count avoids the former i64::MIN sentinel, which
/// made it impossible to seed with i64::MIN as an actual value.
#[no_mangle]
pub unsafe extern "C" fn rt_random_seed(n: i64, arg_count: i64) {
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        if arg_count == 0 {
            // seed() or seed(None) — use system entropy
            *rng = MersenneTwister::from_entropy();
        } else {
            *rng = MersenneTwister::seed_from_int(n);
        }
    });
}

/// random.random() -> float in [0.0, 1.0)
#[no_mangle]
pub unsafe extern "C" fn rt_random_random() -> f64 {
    RNG.with(|rng| rng.borrow_mut().random())
}

/// random.randint(a, b) -> int in [a, b] (inclusive)
#[no_mangle]
pub unsafe extern "C" fn rt_random_randint(a: i64, b: i64) -> i64 {
    if a > b {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "empty range for randint()"
        );
    }
    // randint(a, b) = randrange(a, b+1) = a + randbelow(b - a + 1)
    // Use i128 arithmetic to avoid overflow when a and b span i64's full range.
    let width = match (b as i128 - a as i128).checked_add(1) {
        Some(w) if w > 0 && w <= usize::MAX as i128 => w as usize,
        _ => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::OverflowError,
                "randint range too large"
            );
        }
    };
    RNG.with(|rng| a + rng.borrow_mut().randbelow(width) as i64)
}

/// random.uniform(a, b) -> float in [a, b]
#[no_mangle]
pub unsafe extern "C" fn rt_random_uniform(a: f64, b: f64) -> f64 {
    RNG.with(|rng| a + (b - a) * rng.borrow_mut().random())
}

/// random.randrange(start, stop, step) - like range() but returns random element
/// stop=i64::MIN means single-arg form (0..start)
/// step=0 means not provided (default 1)
#[no_mangle]
pub unsafe extern "C" fn rt_random_randrange(start: i64, stop: i64, step: i64) -> i64 {
    let (actual_start, actual_stop, actual_step) = if stop == i64::MIN && step == 0 {
        // Single arg: randrange(stop) -> range(0, stop)
        (0i64, start, 1i64)
    } else if step == 0 {
        // Two args: randrange(start, stop) -> range(start, stop)
        (start, stop, 1i64)
    } else {
        (start, stop, step)
    };

    if actual_step == 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "zero step for randrange()"
        );
    }

    let n = if actual_step > 0 {
        if actual_start >= actual_stop {
            0
        } else {
            (actual_stop - actual_start + actual_step - 1) / actual_step
        }
    } else if actual_start <= actual_stop {
        0
    } else {
        (actual_start - actual_stop - actual_step - 1) / (-actual_step)
    };

    if n <= 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "empty range for randrange()"
        );
    }

    let idx = RNG.with(|rng| rng.borrow_mut().randbelow(n as usize));
    actual_start + idx as i64 * actual_step
}

/// random.gauss(mu, sigma) -> float from Gaussian distribution
/// Uses Box-Muller transform with caching (matches CPython exactly)
#[no_mangle]
pub unsafe extern "C" fn rt_random_gauss(mu: f64, sigma: f64) -> f64 {
    RNG.with(|rng| rng.borrow_mut().gauss(mu, sigma))
}

/// random.choices(population, weights, k) - weighted random sampling with replacement
/// population and weights are *mut Obj (ListObj), k is count
/// Preserves the population's elem_tag in the result list.
#[no_mangle]
pub unsafe extern "C" fn rt_random_choices(
    population: *mut Obj,
    weights: *mut Obj,
    k: i64,
) -> *mut Obj {
    crate::debug_assert_type_tag!(population, TypeTagKind::List, "rt_random_choices");
    let pop_list = population as *mut ListObj;
    let pop_len = (*pop_list).len;
    let pop_elem_tag = (*pop_list).elem_tag;

    if pop_len == 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "Cannot choose from an empty population"
        );
    }

    let k = k as usize;

    // Build cumulative weights
    let mut cum_weights: Vec<f64> = Vec::with_capacity(pop_len);
    if !weights.is_null() {
        crate::debug_assert_type_tag!(weights, TypeTagKind::List, "rt_random_choices weights");
        let weights_list = weights as *mut ListObj;
        let mut total = 0.0f64;
        for i in 0..pop_len {
            let w_obj = crate::list::list_slot_raw(weights_list, i);
            let w = crate::boxing::rt_unbox_float(w_obj);
            total += w;
            cum_weights.push(total);
        }
    } else {
        // No weights: use floor(random() * n) like CPython
        let result = crate::list::rt_make_list(k as i64, pop_elem_tag);
        let result_list = result as *mut ListObj;
        RNG.with(|rng| {
            let mut rng = rng.borrow_mut();
            let n = pop_len as f64;
            for i in 0..k {
                let idx = (rng.random() * n).floor() as usize;
                *(*result_list).data.add(i) = *(*pop_list).data.add(idx);
            }
        });
        (*result_list).len = k;
        return result;
    }

    let total = *cum_weights
        .last()
        .expect("cumulative weights must be non-empty");

    // Build result list with same elem_tag as population
    let result = crate::list::rt_make_list(k as i64, pop_elem_tag);
    let result_list = result as *mut ListObj;

    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let hi = pop_len - 1; // Match CPython: bisect(cum_weights, r, 0, hi)
        for i in 0..k {
            let r = rng.random() * total;
            // bisect_right in cum_weights[0..=hi]
            let mut lo = 0usize;
            let mut hi_bs = hi;
            while lo < hi_bs {
                let mid = (lo + hi_bs) / 2;
                if r < cum_weights[mid] {
                    hi_bs = mid;
                } else {
                    lo = mid + 1;
                }
            }
            *(*result_list).data.add(i) = *(*pop_list).data.add(lo);
        }
    });

    (*result_list).len = k;
    result
}

/// random.choice(seq) - return random element from a list
/// seq must be a *mut Obj (ListObj)
#[no_mangle]
pub unsafe extern "C" fn rt_random_choice(seq: *mut Obj) -> *mut Obj {
    crate::debug_assert_type_tag!(seq, TypeTagKind::List, "rt_random_choice");
    let list = seq as *mut ListObj;
    let len = (*list).len;
    if len == 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::IndexError,
            "Cannot choose from an empty sequence"
        );
    }
    // CPython: choice(seq) = seq[randbelow(len)]
    let idx = RNG.with(|rng| rng.borrow_mut().randbelow(len));
    crate::list::list_slot_raw(list, idx)
}

/// random.shuffle(seq) - shuffle list in-place using Fisher-Yates
/// seq must be a *mut Obj (ListObj)
#[no_mangle]
pub unsafe extern "C" fn rt_random_shuffle(seq: *mut Obj) {
    crate::debug_assert_type_tag!(seq, TypeTagKind::List, "rt_random_shuffle");
    let list = seq as *mut ListObj;
    let len = (*list).len;
    if len <= 1 {
        return;
    }
    // CPython: for i in reversed(range(1, len)): j = randbelow(i+1); swap(i, j)
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let data = (*list).data;
        for i in (1..len).rev() {
            let j = rng.randbelow(i + 1);
            if i != j {
                let tmp = *data.add(i);
                *data.add(i) = *data.add(j);
                *data.add(j) = tmp;
            }
        }
    });
}

/// random.sample(population, k) - return k unique random elements from population
/// population must be a *mut Obj (ListObj)
#[no_mangle]
pub unsafe extern "C" fn rt_random_sample(population: *mut Obj, k: i64) -> *mut Obj {
    crate::debug_assert_type_tag!(population, TypeTagKind::List, "rt_random_sample");
    let list = population as *mut ListObj;
    let len = (*list).len;
    let k = k as usize;

    if k > len {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "Sample larger than population"
        );
    }

    // CPython's sample uses a selection-based approach for small k
    // We use partial Fisher-Yates on index copy (same result distribution)
    let mut indices: Vec<usize> = (0..len).collect();
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        for i in 0..k {
            let j = i + rng.randbelow(len - i);
            indices.swap(i, j);
        }
    });

    // Build result list preserving population's elem_tag
    let pop_elem_tag = (*list).elem_tag;
    let result = crate::list::rt_make_list(k as i64, pop_elem_tag);
    let result_list = result as *mut ListObj;
    for (i, &src_idx) in indices.iter().take(k).enumerate() {
        let elem = *(*list).data.add(src_idx);
        *(*result_list).data.add(i) = elem;
    }
    (*result_list).len = k;
    result
}
