//! Whether a record's field holds its default, so that writing the record leaves it out.

pub(crate) fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

pub(crate) fn is_zero_u32(x: &u32) -> bool {
    *x == 0
}

pub(crate) fn is_one(x: &f64) -> bool {
    *x == 1.0
}

pub(crate) fn is_false(x: &bool) -> bool {
    !*x
}

pub(crate) fn is_default<T: Default + PartialEq>(x: &T) -> bool {
    *x == T::default()
}
