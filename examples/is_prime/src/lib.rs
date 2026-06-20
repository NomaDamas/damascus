/// Returns true if `n` is a prime number.
///
/// This starts unimplemented on purpose: run Damascus to make the tests pass.
pub fn is_prime(n: u64) -> bool {
    todo!("implement is_prime")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_primes() {
        for p in [2u64, 3, 5, 7, 11, 13, 97] {
            assert!(is_prime(p), "{p} should be prime");
        }
    }

    #[test]
    fn non_primes() {
        for n in [0u64, 1, 4, 6, 9, 100] {
            assert!(!is_prime(n), "{n} should not be prime");
        }
    }
}
