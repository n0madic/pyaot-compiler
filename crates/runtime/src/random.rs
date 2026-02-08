//! random module runtime functions

use crate::object::{ListObj, Obj, TypeTagKind};
use std::cell::RefCell;

// Use rand crate
use rand::prelude::*;
use rand::rngs::StdRng;

thread_local! {
    static RNG: RefCell<StdRng> = RefCell::new(StdRng::from_entropy());
}

/// random.seed(n) - seed the RNG. n=0 means use system entropy (matches Python's seed(None))
#[no_mangle]
pub unsafe extern "C" fn rt_random_seed(n: i64) {
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        if n == 0 {
            *rng = StdRng::from_entropy();
        } else {
            *rng = StdRng::seed_from_u64(n as u64);
        }
    });
}

/// random.random() -> float in [0.0, 1.0)
#[no_mangle]
pub unsafe extern "C" fn rt_random_random() -> f64 {
    RNG.with(|rng| rng.borrow_mut().gen::<f64>())
}

/// random.randint(a, b) -> int in [a, b] (inclusive)
#[no_mangle]
pub unsafe extern "C" fn rt_random_randint(a: i64, b: i64) -> i64 {
    if a > b {
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            b"empty range for randint()" as *const u8,
            "empty range for randint()".len(),
        );
    }
    RNG.with(|rng| rng.borrow_mut().gen_range(a..=b))
}

/// random.uniform(a, b) -> float in [a, b]
#[no_mangle]
pub unsafe extern "C" fn rt_random_uniform(a: f64, b: f64) -> f64 {
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        a + (b - a) * rng.gen::<f64>()
    })
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
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            b"zero step for randrange()" as *const u8,
            "zero step for randrange()".len(),
        );
    }

    let n = if actual_step > 0 {
        if actual_start >= actual_stop {
            0
        } else {
            (actual_stop - actual_start + actual_step - 1) / actual_step
        }
    } else {
        if actual_start <= actual_stop {
            0
        } else {
            (actual_start - actual_stop - actual_step - 1) / (-actual_step)
        }
    };

    if n <= 0 {
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            b"empty range for randrange()" as *const u8,
            "empty range for randrange()".len(),
        );
    }

    let idx = RNG.with(|rng| rng.borrow_mut().gen_range(0..n));
    actual_start + idx * actual_step
}

/// random.choice(seq) - return random element from a list
/// seq must be a *mut Obj (ListObj)
#[no_mangle]
pub unsafe extern "C" fn rt_random_choice(seq: *mut Obj) -> *mut Obj {
    crate::debug_assert_type_tag!(seq, TypeTagKind::List, "rt_random_choice");
    let list = seq as *mut ListObj;
    let len = (*list).len;
    if len == 0 {
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::IndexError.tag(),
            b"Cannot choose from an empty sequence" as *const u8,
            "Cannot choose from an empty sequence".len(),
        );
    }
    let idx = RNG.with(|rng| rng.borrow_mut().gen_range(0..len));
    *(*list).data.add(idx)
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
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        let data = (*list).data;
        for i in (1..len).rev() {
            let j = rng.gen_range(0..=i);
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
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            b"Sample larger than population" as *const u8,
            "Sample larger than population".len(),
        );
    }

    // Create a working copy of indices
    let mut indices: Vec<usize> = (0..len).collect();
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        // Fisher-Yates partial shuffle for first k elements
        for i in 0..k {
            let j = rng.gen_range(i..len);
            indices.swap(i, j);
        }
    });

    // Build result list
    let result = crate::list::rt_make_list(k as i64, crate::object::ELEM_HEAP_OBJ);
    let result_list = result as *mut ListObj;
    for i in 0..k {
        let elem = *(*list).data.add(indices[i]);
        *(*result_list).data.add(i) = elem;
    }
    (*result_list).len = k;
    result
}
