pub mod stack;

#[cfg(test)]
mod tests {
    use super::stack::Stack;

    #[test]
    fn push_pop_order() {
        let mut s = Stack::new();
        s.push(1);
        s.push(2);
        s.push(3);
        assert_eq!(s.pop(), Some(3));
        assert_eq!(s.pop(), Some(2));
        assert_eq!(s.pop(), Some(1));
        assert_eq!(s.pop(), None);
    }

    #[test]
    fn len_tracks() {
        let mut s: Stack<&str> = Stack::new();
        assert!(s.is_empty());
        s.push("a");
        s.push("b");
        assert_eq!(s.len(), 2);
        s.pop();
        assert_eq!(s.len(), 1);
    }
}
