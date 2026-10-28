use crate::{Error, KeyName};

pub fn validate<K: KeyName + ?Sized>(s: &str) -> Result<(), Error> {
    let colon_idx = s.find(':').ok_or(Error::MissingColon)?;
    if colon_idx == 0 {
        return Err(Error::Empty);
    }
    K::validate(&s[colon_idx + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    impl KeyName for str {
        fn validate(s: &str) -> Result<(), Error> {
            if s == "key" {
                Ok(())
            } else {
                Err(Error::InvalidCharacters)
            }
        }
    }

    #[test]
    fn validates_the_actual_suffix_after_long_algorithms() {
        for algorithm in [
            "a".repeat(255),
            "a".repeat(256),
            "a".repeat(257),
            "é".repeat(129),
        ] {
            assert_eq!(validate::<str>(&format!("{algorithm}:key")), Ok(()));
            assert_eq!(
                validate::<str>(&format!("{algorithm}:bad")),
                Err(Error::InvalidCharacters)
            );
        }
    }

    #[test]
    fn distinguishes_empty_algorithm_from_missing_separator() {
        assert_eq!(validate::<str>(":key"), Err(Error::Empty));
        assert_eq!(validate::<str>("key"), Err(Error::MissingColon));
    }
}
