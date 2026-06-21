/// Returns the FizzBuzz sequence for 1..=n.
/// Multiples of 3 -> "Fizz", of 5 -> "Buzz", of 15 -> "FizzBuzz",
/// otherwise the number as a string.
pub fn fizzbuzz(n: u32) -> Vec<String> {
    todo!("implement fizzbuzz")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_fifteen() {
        let got = fizzbuzz(15);
        let want = [
            "1", "2", "Fizz", "4", "Buzz", "Fizz", "7", "8", "Fizz", "Buzz", "11", "Fizz", "13",
            "14", "FizzBuzz",
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn length_matches() {
        assert_eq!(fizzbuzz(100).len(), 100);
    }
}
