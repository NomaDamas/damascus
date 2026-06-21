/// Convert an integer in 1..=3999 to a Roman numeral.
pub fn to_roman(mut n: u32) -> String {
    todo!("implement to_roman")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(to_roman(1), "I");
        assert_eq!(to_roman(4), "IV");
        assert_eq!(to_roman(9), "IX");
        assert_eq!(to_roman(14), "XIV");
        assert_eq!(to_roman(40), "XL");
        assert_eq!(to_roman(90), "XC");
        assert_eq!(to_roman(400), "CD");
        assert_eq!(to_roman(944), "CMXLIV");
        assert_eq!(to_roman(2024), "MMXXIV");
        assert_eq!(to_roman(3888), "MMMDCCCLXXXVIII");
    }
}
