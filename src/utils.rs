use std::cmp::Ordering;

/// Imagine macro parameters, but more like those Russian dolls.
///
/// Calls m!(A, B, C), m!(A, B), m!(B), and m!() for i.e. (m, A, B, C)
/// where m is any macro, for any number of parameters.
macro_rules! smaller_tuples_too {
    ($m: ident, $ty: ident) => {
        $m!{$ty}
        $m!{}
    };
    ($m: ident, $ty: ident, $($tt: ident),*) => {
        $m!{$ty, $($tt),*}
        smaller_tuples_too!{$m, $($tt),*}
    };
}

pub fn cmp_reversed<T>(lhs: &T, rhs: &T) -> Ordering
where
    T: Ord,
{
    lhs.cmp(rhs).reverse()
}

pub fn partial_cmp_reversed<T, U>(lhs: &T, rhs: &U) -> Option<Ordering>
where
    T: PartialOrd<U>,
{
    lhs.partial_cmp(rhs).map(Ordering::reverse)
}
