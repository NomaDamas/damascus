/// Return the median of `xs`. For an even number of elements it is the average
/// of the two middle values. `xs` is assumed non-empty.
///
/// NOTE: this implementation has a bug — the tests below fail. Fix the body.
pub fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = v.len() / 2;
    // BUG: ignores the even-length case
    v[mid]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_length() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
    }
    #[test]
    fn even_length() {
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
    }
    #[test]
    fn even_unsorted() {
        assert_eq!(median(&[10.0, 2.0, 8.0, 4.0]), 6.0);
    }
}
