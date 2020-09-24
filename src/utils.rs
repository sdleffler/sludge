use std::cmp::Ordering;

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
