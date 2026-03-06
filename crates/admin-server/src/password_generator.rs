/// Password Generator - Secure password generation utilities
///
/// This module provides secure password generation functions for user management.
/// All passwords are generated using a cryptographically secure random number generator.
///
/// # Features
///
/// - Configurable password length (default 16 characters)
/// - Enforces minimum complexity requirements
/// - Uses cryptographically secure random number generation
/// - Provides password strength estimation

use rand::Rng;
use std::error::Error;
use std::fmt;

/// Password generation configuration
#[derive(Debug, Clone)]
pub struct PasswordConfig {
    /// Minimum password length (default: 16)
    pub min_length: usize,
    /// Maximum password length (default: 64)
    pub max_length: usize,
    /// Include uppercase letters (default: true)
    pub include_uppercase: bool,
    /// Include lowercase letters (default: true)
    pub include_lowercase: bool,
    /// Include digits (default: true)
    pub include_digits: bool,
    /// Include special characters (default: true)
    pub include_special: bool,
    /// Special characters to include
    pub special_chars: &'static str,
}

impl Default for PasswordConfig {
    fn default() -> Self {
        Self {
            min_length: 16,
            max_length: 64,
            include_uppercase: true,
            include_lowercase: true,
            include_digits: true,
            include_special: true,
            special_chars: "!@#$%^&*()_+-=[]{}|;:,.<>?",
        }
    }
}

/// Password generation errors
#[derive(Debug)]
pub enum PasswordError {
    /// Password too short
    TooShort(usize),
    /// Password too long
    TooLong(usize),
    /// No character sets enabled
    NoCharacterSets,
    /// Password does not meet complexity requirements
    ComplexityNotMet(String),
}

impl fmt::Display for PasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasswordError::TooShort(len) => write!(f, "Password too short: {} characters (minimum {})", len, 16),
            PasswordError::TooLong(len) => write!(f, "Password too long: {} characters (maximum {})", len, 64),
            PasswordError::NoCharacterSets => write!(f, "At least one character set must be enabled"),
            PasswordError::ComplexityNotMet(msg) => write!(f, "Password complexity not met: {}", msg),
        }
    }
}

impl Error for PasswordError {}

/// Character sets for password generation
const UPPERCASE_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWERCASE_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGIT_CHARS: &[u8] = b"0123456789";

/// Generate a secure random password
///
/// # Arguments
///
/// * `config` - Password configuration (uses defaults if None)
///
/// # Returns
///
/// A randomly generated password string
///
/// # Examples
///
/// ```
/// use palpo_admin_server::password_generator::{generate_password, PasswordConfig};
///
/// // Use default config
/// let password = generate_password(None);
/// assert!(password.len() >= 16);
///
/// // Use custom config
/// let config = PasswordConfig {
///     min_length: 20,
///     max_length: 32,
///     ..Default::default()
/// };
/// let password = generate_password(Some(config));
/// assert!(password.len() >= 20 && password.len() <= 32);
/// ```
pub fn generate_password(config: Option<PasswordConfig>) -> String {
    let config = config.unwrap_or_default();
    let length = rand::thread_rng().gen_range(config.min_length..=config.max_length);
    generate_password_with_length(length, &config)
}

/// Generate a password with a specific length
fn generate_password_with_length(length: usize, config: &PasswordConfig) -> String {
    let mut rng = rand::thread_rng();
    let mut password = String::with_capacity(length);
    let mut char_sets: Vec<&[u8]> = Vec::new();

    if config.include_uppercase {
        char_sets.push(UPPERCASE_CHARS);
        password.push(UPPERCASE_CHARS[rng.gen_range(0..UPPERCASE_CHARS.len())] as char);
    }
    if config.include_lowercase {
        char_sets.push(LOWERCASE_CHARS);
        password.push(LOWERCASE_CHARS[rng.gen_range(0..LOWERCASE_CHARS.len())] as char);
    }
    if config.include_digits {
        char_sets.push(DIGIT_CHARS);
        password.push(DIGIT_CHARS[rng.gen_range(0..DIGIT_CHARS.len())] as char);
    }
    if config.include_special {
        let special: Vec<char> = config.special_chars.chars().collect();
        password.push(special[rng.gen_range(0..special.len())]);
    }

    // Fill remaining length with random characters from all enabled sets
    let all_chars: Vec<u8> = char_sets.iter().flat_map(|s| s.to_vec()).collect();
    for _ in password.len()..length {
        password.push(all_chars[rng.gen_range(0..all_chars.len())] as char);
    }

    // Shuffle the password to avoid predictable patterns
    let mut chars: Vec<char> = password.chars().collect();
    for i in (1..chars.len()).rev() {
        let j = rng.gen_range(0..=i);
        chars.swap(i, j);
    }

    chars.into_iter().collect()
}

/// Validate password strength
///
/// # Arguments
///
/// * `password` - Password to validate
/// * `config` - Password configuration
///
/// # Returns
///
/// `Ok(())` if password meets requirements, or an error
///
/// # Examples
///
/// ```
/// use palpo_admin_server::password_generator::validate_password;
///
/// assert!(validate_password("SecureP@ssw0rd123").is_ok());
/// assert!(validate_password("short").is_err());
/// ```
pub fn validate_password(password: &str, config: Option<PasswordConfig>) -> Result<(), PasswordError> {
    let config = config.unwrap_or_default();

    // Check length
    if password.len() < config.min_length {
        return Err(PasswordError::TooShort(password.len()));
    }
    if password.len() > config.max_length {
        return Err(PasswordError::TooLong(password.len()));
    }

    // Check at least one character set is enabled
    if !config.include_uppercase
        && !config.include_lowercase
        && !config.include_digits
        && !config.include_special
    {
        return Err(PasswordError::NoCharacterSets);
    }

    // Validate character requirements
    let mut has_uppercase = false;
    let mut has_lowercase = false;
    let mut has_digit = false;
    let mut has_special = false;

    for c in password.chars() {
        if c.is_ascii_uppercase() {
            has_uppercase = true;
        } else if c.is_ascii_lowercase() {
            has_lowercase = true;
        } else if c.is_ascii_digit() {
            has_digit = true;
        } else if config.special_chars.contains(c) {
            has_special = true;
        }
    }

    let mut errors = Vec::new();
    if config.include_uppercase && !has_uppercase {
        errors.push("uppercase letter");
    }
    if config.include_lowercase && !has_lowercase {
        errors.push("lowercase letter");
    }
    if config.include_digits && !has_digit {
        errors.push("digit");
    }
    if config.include_special && !has_special {
        errors.push("special character");
    }

    if !errors.is_empty() {
        return Err(PasswordError::ComplexityNotMet(format!(
            "missing: {}",
            errors.join(", ")
        )));
    }

    Ok(())
}

/// Estimate password entropy
///
/// Returns the estimated entropy in bits based on character set size and password length.
///
/// # Examples
///
/// ```
/// use palpo_admin_server::password_generator::estimate_entropy;
///
/// let entropy = estimate_entropy("SecureP@ssw0rd123");
/// assert!(entropy > 50.0);
/// ```
pub fn estimate_entropy(password: &str) -> f64 {
    let mut charset_size = 0;

    if password.chars().any(|c| c.is_ascii_uppercase()) {
        charset_size += 26;
    }
    if password.chars().any(|c| c.is_ascii_lowercase()) {
        charset_size += 26;
    }
    if password.chars().any(|c| c.is_ascii_digit()) {
        charset_size += 10;
    }
    if password.chars().any(|c| !c.is_ascii_alphanumeric()) {
        charset_size += 32; // Approximate special chars
    }

    if charset_size == 0 {
        return 0.0;
    }

    (password.len() as f64) * (charset_size as f64).log2()
}

/// Get password strength rating
///
/// Returns a rating from 0-5 based on entropy and complexity.
///
/// # Examples
///
/// ```
/// use palpo_admin_server::password_generator::get_strength_rating;
///
/// assert!(get_strength_rating("weak") < 3);
/// assert!(get_strength_rating("SecureP@ssw0rd123!") >= 4);
/// ```
pub fn get_strength_rating(password: &str) -> u8 {
    let entropy = estimate_entropy(password);

    match entropy {
        e if e < 32.0 => 1,  // Very weak
        e if e < 48.0 => 2,  // Weak
        e if e < 64.0 => 3,  // Moderate
        e if e < 80.0 => 4,  // Strong
        _ => 5,              // Very strong
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_password_default() {
        let password = generate_password(None);
        assert!(password.len() >= 16);
        assert!(password.chars().any(|c| c.is_uppercase()));
        assert!(password.chars().any(|c| c.is_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_generate_password_custom_length() {
        let config = PasswordConfig {
            min_length: 24,
            max_length: 24,
            ..Default::default()
        };
        let password = generate_password(Some(config));
        assert_eq!(password.len(), 24);
    }

    #[test]
    fn test_validate_password_valid() {
        assert!(validate_password("SecureP@ssw0rd123", None).is_ok());
    }

    #[test]
    fn test_validate_password_too_short() {
        assert!(validate_password("short", None).is_err());
    }

    #[test]
    fn test_validate_password_missing_uppercase() {
        let config = PasswordConfig {
            include_uppercase: true,
            include_lowercase: true,
            include_digits: true,
            include_special: true,
            ..Default::default()
        };
        assert!(validate_password("password123!", Some(config)).is_err());
    }

    #[test]
    fn test_validate_password_missing_digit() {
        let config = PasswordConfig {
            include_uppercase: true,
            include_lowercase: true,
            include_digits: true,
            include_special: true,
            ..Default::default()
        };
        assert!(validate_password("Password!@#$%", Some(config)).is_err());
    }

    #[test]
    fn test_entropy_calculation() {
        let entropy = estimate_entropy("abc");
        assert!(entropy > 0.0);

        let strong_entropy = estimate_entropy("SecureP@ssw0rd123!");
        assert!(strong_entropy > entropy);
    }

    #[test]
    fn test_strength_rating() {
        assert_eq!(get_strength_rating("a"), 1);
        assert_eq!(get_strength_rating("abc123"), 1); // entropy ~31, which is < 32
        assert_eq!(get_strength_rating("abc123def"), 2); // entropy ~46, which is 32-48
        assert!(get_strength_rating("SecureP@ssw0rd123!") >= 4);
    }
}