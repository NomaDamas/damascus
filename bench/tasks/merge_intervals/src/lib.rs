/// Merge overlapping intervals and return them sorted by start.
/// Intervals are inclusive `(start, end)` with start <= end.
pub fn merge(intervals: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    todo!("implement interval merging")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_overlaps() {
        assert_eq!(merge(vec![(1, 3), (2, 6), (8, 10), (15, 18)]), vec![(1, 6), (8, 10), (15, 18)]);
    }
    #[test]
    fn touching_merge() {
        assert_eq!(merge(vec![(1, 4), (4, 5)]), vec![(1, 5)]);
    }
    #[test]
    fn unsorted_input() {
        assert_eq!(merge(vec![(5, 7), (1, 2), (2, 4)]), vec![(1, 4), (5, 7)]);
    }
    #[test]
    fn empty() {
        assert_eq!(merge(vec![]), Vec::<(i32, i32)>::new());
    }
}
