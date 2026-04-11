//! Runtime operations (arithmetic, comparisons, printing, etc.)

mod arithmetic;
mod comparison;
mod printing;

// Re-export all public functions

pub use arithmetic::{
    rt_add_float, rt_add_int, rt_div_float, rt_div_int, rt_mod_int, rt_mul_float, rt_mul_int,
    rt_obj_add, rt_obj_div, rt_obj_floordiv, rt_obj_mod, rt_obj_mul, rt_obj_pow, rt_obj_sub,
    rt_sub_float, rt_sub_int, rt_true_div_int,
};

pub use comparison::{
    rt_any_getitem, rt_is_truthy, rt_obj_contains, rt_obj_eq, rt_obj_gt, rt_obj_gte, rt_obj_lt,
    rt_obj_lte,
};

pub use printing::{
    rt_flush_stdout, rt_print_bool_value, rt_print_float_value, rt_print_int_value,
    rt_print_newline, rt_print_none_value, rt_print_obj, rt_print_sep, rt_print_str_value,
};

#[cfg(test)]
mod tests {
    use super::arithmetic::*;
    use super::comparison::*;

    fn init_runtime() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::gc::init();
    }

    #[test]
    fn test_add_int_basic() {
        assert_eq!(rt_add_int(1, 2), 3);
        assert_eq!(rt_add_int(0, 0), 0);
        assert_eq!(rt_add_int(-5, 3), -2);
        assert_eq!(rt_add_int(i64::MAX - 1, 1), i64::MAX);
    }

    #[test]
    fn test_sub_int_basic() {
        assert_eq!(rt_sub_int(5, 3), 2);
        assert_eq!(rt_sub_int(0, 0), 0);
        assert_eq!(rt_sub_int(i64::MIN + 1, 1), i64::MIN);
    }

    #[test]
    fn test_mul_int_basic() {
        assert_eq!(rt_mul_int(3, 4), 12);
        assert_eq!(rt_mul_int(0, i64::MAX), 0);
        assert_eq!(rt_mul_int(-3, 5), -15);
    }

    #[test]
    fn test_div_int_floor() {
        // Python floor division: -7 // 2 = -4 (not -3)
        assert_eq!(rt_div_int(7, 2), 3);
        assert_eq!(rt_div_int(-7, 2), -4);
        assert_eq!(rt_div_int(7, -2), -4);
        assert_eq!(rt_div_int(-7, -2), 3);
        assert_eq!(rt_div_int(6, 3), 2);
    }

    #[test]
    fn test_mod_int() {
        assert_eq!(rt_mod_int(7, 3), 1);
        assert_eq!(rt_mod_int(-7, 3), 2); // Python: -7 % 3 = 2
        assert_eq!(rt_mod_int(7, -3), -2); // Python: 7 % -3 = -2
    }

    #[test]
    fn test_float_arithmetic() {
        assert_eq!(rt_add_float(1.5, 2.5), 4.0);
        assert_eq!(rt_sub_float(5.0, 3.0), 2.0);
        assert_eq!(rt_mul_float(2.0, 3.0), 6.0);
        assert_eq!(rt_div_float(7.0, 2.0), 3.5);
    }

    #[test]
    fn test_true_div_int() {
        assert_eq!(rt_true_div_int(7, 2), 3.5);
        assert_eq!(rt_true_div_int(6, 3), 2.0);
    }

    #[test]
    fn test_is_truthy_int() {
        init_runtime();
        let zero = crate::boxing::rt_box_int(0);
        assert_eq!(rt_is_truthy(zero), 0);
        let one = crate::boxing::rt_box_int(1);
        assert_eq!(rt_is_truthy(one), 1);
        let neg = crate::boxing::rt_box_int(-1);
        assert_eq!(rt_is_truthy(neg), 1);
    }
}
