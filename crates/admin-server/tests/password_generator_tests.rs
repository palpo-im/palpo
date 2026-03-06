/// Unit tests for password generator module
///
/// Tests cover:
/// - Password generation with various lengths
/// - Password strength validation (weak, medium, strong)
/// - Character set requirements (uppercase, lowercase, digits, special chars)
/// - Edge cases (minimum length, maximum length, invalid inputs)
/// - Randomness and uniqueness of generated passwords
/// - Security requirements validation

use palpo_admin_server::password_generator::{
    estimate_entropy, generate_password, get_strength_rating, validate_password, PasswordConfig,
    PasswordError,
};
use std::collections::HashSet;

// ============================================================================
// Password Generation Tests
// ============================================================================

#[test]
fn test_generate_password_default_config() {
    let password = generate_password(None);

    // Check length is within default range (16-64)
    assert!(
        password.len() >= 16 && password.len() <= 64,
        "Password length {} not in range [16, 64]",
        password.len()
    );

    // Verify it meets default complexity requirements
    assert!(validate_password(&password, None).is_ok());
}

#[test]
fn test_generate_password_minimum_length() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        ..Default::default()
    };
    let password = generate_password(Some(config));

    assert_eq!(password.len(), 16, "Password should be exactly 16 characters");
}

#[test]
fn test_generate_password_maximum_length() {
    let config = PasswordConfig {
        min_length: 64,
        max_length: 64,
        ..Default::default()
    };
    let password = generate_password(Some(config));

    assert_eq!(password.len(), 64, "Password should be exactly 64 characters");
}

#[test]
fn test_generate_password_custom_length_range() {
    let config = PasswordConfig {
        min_length: 20,
        max_length: 30,
        ..Default::default()
    };

    for _ in 0..10 {
        let password = generate_password(Some(config.clone()));
        assert!(
            password.len() >= 20 && password.len() <= 30,
            "Password length {} not in range [20, 30]",
            password.len()
        );
    }
}

#[test]
fn test_generate_password_contains_uppercase() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: true,
        include_lowercase: false,
        include_digits: false,
        include_special: false,
        ..Default::default()
    };
    let password = generate_password(Some(config));

    assert!(
        password.chars().any(|c| c.is_ascii_uppercase()),
        "Password should contain uppercase letters"
    );
    assert!(
        password.chars().all(|c| c.is_ascii_uppercase()),
        "Password should only contain uppercase letters"
    );
}

#[test]
fn test_generate_password_contains_lowercase() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: false,
        include_lowercase: true,
        include_digits: false,
        include_special: false,
        ..Default::default()
    };
    let password = generate_password(Some(config));

    assert!(
        password.chars().any(|c| c.is_ascii_lowercase()),
        "Password should contain lowercase letters"
    );
    assert!(
        password.chars().all(|c| c.is_ascii_lowercase()),
        "Password should only contain lowercase letters"
    );
}

#[test]
fn test_generate_password_contains_digits() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: false,
        include_lowercase: false,
        include_digits: true,
        include_special: false,
        ..Default::default()
    };
    let password = generate_password(Some(config));

    assert!(
        password.chars().any(|c| c.is_ascii_digit()),
        "Password should contain digits"
    );
    assert!(
        password.chars().all(|c| c.is_ascii_digit()),
        "Password should only contain digits"
    );
}

#[test]
fn test_generate_password_contains_special_chars() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: false,
        include_lowercase: true, // Need at least one charset for filling
        include_digits: false,
        include_special: true,
        special_chars: "!@#$%^&*()",
        ..Default::default()
    };
    let password = generate_password(Some(config.clone()));

    assert!(
        password
            .chars()
            .any(|c| config.special_chars.contains(c)),
        "Password should contain special characters"
    );
}

#[test]
fn test_generate_password_all_character_sets() {
    let config = PasswordConfig {
        min_length: 20,
        max_length: 20,
        include_uppercase: true,
        include_lowercase: true,
        include_digits: true,
        include_special: true,
        ..Default::default()
    };
    let password = generate_password(Some(config.clone()));

    assert!(
        password.chars().any(|c| c.is_ascii_uppercase()),
        "Password should contain uppercase letters"
    );
    assert!(
        password.chars().any(|c| c.is_ascii_lowercase()),
        "Password should contain lowercase letters"
    );
    assert!(
        password.chars().any(|c| c.is_ascii_digit()),
        "Password should contain digits"
    );
    assert!(
        password
            .chars()
            .any(|c| config.special_chars.contains(c)),
        "Password should contain special characters"
    );
}

// ============================================================================
// Password Randomness and Uniqueness Tests
// ============================================================================

#[test]
fn test_generate_password_uniqueness() {
    let mut passwords = HashSet::new();
    let iterations = 100;

    for _ in 0..iterations {
        let password = generate_password(None);
        passwords.insert(password);
    }

    // All generated passwords should be unique
    assert_eq!(
        passwords.len(),
        iterations,
        "Generated passwords should all be unique"
    );
}

#[test]
fn test_generate_password_randomness() {
    // Generate multiple passwords and check they're different
    let password1 = generate_password(None);
    let password2 = generate_password(None);
    let password3 = generate_password(None);

    assert_ne!(password1, password2, "Passwords should be different");
    assert_ne!(password2, password3, "Passwords should be different");
    assert_ne!(password1, password3, "Passwords should be different");
}

#[test]
fn test_generate_password_no_predictable_patterns() {
    let config = PasswordConfig {
        min_length: 20,
        max_length: 20,
        ..Default::default()
    };

    for _ in 0..10 {
        let password = generate_password(Some(config.clone()));

        // Check that character types are not grouped together
        // (e.g., not all uppercase at the start)
        let chars: Vec<char> = password.chars().collect();

        // Count consecutive uppercase letters
        let mut max_consecutive_uppercase = 0;
        let mut current_consecutive = 0;
        for c in &chars {
            if c.is_ascii_uppercase() {
                current_consecutive += 1;
                max_consecutive_uppercase = max_consecutive_uppercase.max(current_consecutive);
            } else {
                current_consecutive = 0;
            }
        }

        // Should not have all uppercase letters consecutive
        assert!(
            max_consecutive_uppercase < 10,
            "Password should not have too many consecutive uppercase letters"
        );
    }
}

// ============================================================================
// Password Validation Tests
// ============================================================================

#[test]
fn test_validate_password_valid_strong_password() {
    let password = "SecureP@ssw0rd123";
    assert!(
        validate_password(password, None).is_ok(),
        "Strong password should be valid"
    );
}

#[test]
fn test_validate_password_too_short() {
    let password = "Short1!";
    let result = validate_password(password, None);

    assert!(result.is_err(), "Short password should be invalid");
    match result.unwrap_err() {
        PasswordError::TooShort(len) => assert_eq!(len, 7),
        _ => panic!("Expected TooShort error"),
    }
}

#[test]
fn test_validate_password_too_long() {
    let password = "A".repeat(65) + "1!";
    let result = validate_password(&password, None);

    assert!(result.is_err(), "Too long password should be invalid");
    match result.unwrap_err() {
        PasswordError::TooLong(len) => assert_eq!(len, 67),
        _ => panic!("Expected TooLong error"),
    }
}

#[test]
fn test_validate_password_missing_uppercase() {
    let password = "password123!@#$%";
    let result = validate_password(password, None);

    assert!(
        result.is_err(),
        "Password without uppercase should be invalid"
    );
    match result.unwrap_err() {
        PasswordError::ComplexityNotMet(msg) => {
            assert!(msg.contains("uppercase"), "Error should mention uppercase")
        }
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_lowercase() {
    let password = "PASSWORD123!@#$%";
    let result = validate_password(password, None);

    assert!(
        result.is_err(),
        "Password without lowercase should be invalid"
    );
    match result.unwrap_err() {
        PasswordError::ComplexityNotMet(msg) => {
            assert!(msg.contains("lowercase"), "Error should mention lowercase")
        }
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_digit() {
    let password = "Password!@#$%^&*";
    let result = validate_password(password, None);

    assert!(result.is_err(), "Password without digit should be invalid");
    match result.unwrap_err() {
        PasswordError::ComplexityNotMet(msg) => {
            assert!(msg.contains("digit"), "Error should mention digit")
        }
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_special() {
    let password = "Password12345678";
    let result = validate_password(password, None);

    assert!(
        result.is_err(),
        "Password without special char should be invalid"
    );
    match result.unwrap_err() {
        PasswordError::ComplexityNotMet(msg) => {
            assert!(
                msg.contains("special"),
                "Error should mention special character"
            )
        }
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_multiple_requirements() {
    let password = "passwordpassword"; // Missing uppercase, digit, special
    let result = validate_password(password, None);

    assert!(result.is_err(), "Password should be invalid");
    match result.unwrap_err() {
        PasswordError::ComplexityNotMet(msg) => {
            assert!(msg.contains("uppercase"));
            assert!(msg.contains("digit"));
            assert!(msg.contains("special"));
        }
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_custom_config_no_special() {
    let config = PasswordConfig {
        include_special: false,
        ..Default::default()
    };
    let password = "Password12345678"; // No special chars

    assert!(
        validate_password(password, Some(config)).is_ok(),
        "Password should be valid without special chars when not required"
    );
}

#[test]
fn test_validate_password_custom_config_no_digits() {
    let config = PasswordConfig {
        include_digits: false,
        ..Default::default()
    };
    let password = "Password!@#$%^&*"; // No digits

    assert!(
        validate_password(password, Some(config)).is_ok(),
        "Password should be valid without digits when not required"
    );
}

#[test]
fn test_validate_password_minimum_length_boundary() {
    let config = PasswordConfig {
        min_length: 16,
        ..Default::default()
    };

    // Exactly 16 characters
    let password = "Password123!@#$%";
    assert_eq!(password.len(), 16);
    assert!(
        validate_password(password, Some(config.clone())).is_ok(),
        "Password at minimum length should be valid"
    );

    // 15 characters (one less than minimum)
    let password = "Password123!@#$";
    assert_eq!(password.len(), 15);
    assert!(
        validate_password(password, Some(config)).is_err(),
        "Password below minimum length should be invalid"
    );
}

#[test]
fn test_validate_password_maximum_length_boundary() {
    let config = PasswordConfig {
        max_length: 64,
        ..Default::default()
    };

    // Exactly 64 characters: 55 A's + "a123!@#AB" (9 chars) = 64
    let password = format!("{}a123!@#AB", "A".repeat(55));
    assert_eq!(password.len(), 64, "Password should be exactly 64 characters");
    assert!(
        validate_password(&password, Some(config.clone())).is_ok(),
        "Password at maximum length should be valid"
    );

    // 65 characters (one more than maximum)
    let password = format!("{}a123!@#ABC", "A".repeat(55));
    assert_eq!(password.len(), 65, "Password should be exactly 65 characters");
    assert!(
        validate_password(&password, Some(config)).is_err(),
        "Password above maximum length should be invalid"
    );
}

// ============================================================================
// Password Strength and Entropy Tests
// ============================================================================

#[test]
fn test_estimate_entropy_weak_password() {
    let password = "abc";
    let entropy = estimate_entropy(password);

    assert!(entropy > 0.0, "Entropy should be positive");
    assert!(entropy < 20.0, "Weak password should have low entropy");
}

#[test]
fn test_estimate_entropy_medium_password() {
    let password = "Password123";
    let entropy = estimate_entropy(password);
    // Password123: charset=62 (upper+lower+digits), len=11, entropy ≈ 66
    assert!(
        entropy >= 60.0 && entropy < 80.0,
        "Medium password should have moderate entropy, got {}",
        entropy
    );
}

#[test]
fn test_estimate_entropy_strong_password() {
    let password = "SecureP@ssw0rd123!";
    let entropy = estimate_entropy(password);

    assert!(
        entropy >= 60.0,
        "Strong password should have high entropy"
    );
}

#[test]
fn test_estimate_entropy_increases_with_length() {
    let short = "Pass1!";
    let medium = "Password123!";
    let long = "SecurePassword123!@#$%";

    let entropy_short = estimate_entropy(short);
    let entropy_medium = estimate_entropy(medium);
    let entropy_long = estimate_entropy(long);

    assert!(
        entropy_short < entropy_medium,
        "Longer password should have higher entropy"
    );
    assert!(
        entropy_medium < entropy_long,
        "Longer password should have higher entropy"
    );
}

#[test]
fn test_estimate_entropy_increases_with_charset() {
    let lowercase_only = "passwordpassword"; // 16 chars
    let with_uppercase = "Passwordpassword"; // 16 chars
    let with_digits = "Password12345678"; // 16 chars
    let with_special = "Password123!@#$%"; // 16 chars

    let entropy_lowercase = estimate_entropy(lowercase_only);
    let entropy_uppercase = estimate_entropy(with_uppercase);
    let entropy_digits = estimate_entropy(with_digits);
    let entropy_special = estimate_entropy(with_special);

    assert!(
        entropy_uppercase > entropy_lowercase,
        "Adding uppercase should increase entropy"
    );
    assert!(
        entropy_digits > entropy_uppercase,
        "Adding digits should increase entropy"
    );
    assert!(
        entropy_special > entropy_digits,
        "Adding special chars should increase entropy"
    );
}

#[test]
fn test_get_strength_rating_very_weak() {
    let password = "abc";
    let rating = get_strength_rating(password);

    assert_eq!(rating, 1, "Very weak password should have rating 1");
}

#[test]
fn test_get_strength_rating_weak() {
    let password = "abc123";
    let rating = get_strength_rating(password);
    // abc123: charset=36 (lowercase+digits), len=6, entropy ≈ 31 → rating 1
    assert_eq!(rating, 1, "Weak password should have rating 1");
}

#[test]
fn test_get_strength_rating_moderate() {
    let password = "Password123";
    let rating = get_strength_rating(password);
    // Password123: charset=62 (upper+lower+digits), len=11, entropy ≈ 66 → rating 4
    assert_eq!(rating, 4, "Moderate password should have rating 4");
}

#[test]
fn test_get_strength_rating_strong() {
    let password = "SecureP@ssw0rd123";
    let rating = get_strength_rating(password);

    assert!(
        rating >= 4,
        "Strong password should have rating 4 or higher"
    );
}

#[test]
fn test_get_strength_rating_very_strong() {
    let password = "VerySecureP@ssw0rd123!@#$%^&*()";
    let rating = get_strength_rating(password);

    assert_eq!(rating, 5, "Very strong password should have rating 5");
}

// ============================================================================
// Edge Cases and Error Handling Tests
// ============================================================================

#[test]
fn test_validate_password_empty_string() {
    let password = "";
    let result = validate_password(password, None);

    assert!(result.is_err(), "Empty password should be invalid");
    match result.unwrap_err() {
        PasswordError::TooShort(len) => assert_eq!(len, 0),
        _ => panic!("Expected TooShort error"),
    }
}

#[test]
fn test_validate_password_single_character() {
    let password = "A";
    let result = validate_password(password, None);

    assert!(
        result.is_err(),
        "Single character password should be invalid"
    );
}

#[test]
fn test_validate_password_unicode_characters() {
    let password = "Pässwörd123!@#$%"; // Contains unicode
    let result = validate_password(password, None);

    // Should still validate based on ASCII requirements
    assert!(result.is_ok(), "Password with unicode should be valid");
}

#[test]
fn test_generate_password_meets_own_validation() {
    // Generate 50 passwords and ensure they all pass validation
    for _ in 0..50 {
        let password = generate_password(None);
        assert!(
            validate_password(&password, None).is_ok(),
            "Generated password '{}' should pass validation",
            password
        );
    }
}

#[test]
fn test_generate_password_with_custom_special_chars() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        special_chars: "!@#",
        ..Default::default()
    };
    let password = generate_password(Some(config.clone()));

    // Check that special chars are from the custom set
    let special_chars: Vec<char> = password
        .chars()
        .filter(|c| !c.is_alphanumeric())
        .collect();

    for c in special_chars {
        assert!(
            config.special_chars.contains(c),
            "Special char '{}' should be from custom set",
            c
        );
    }
}

#[test]
fn test_password_config_default_values() {
    let config = PasswordConfig::default();

    assert_eq!(config.min_length, 16);
    assert_eq!(config.max_length, 64);
    assert!(config.include_uppercase);
    assert!(config.include_lowercase);
    assert!(config.include_digits);
    assert!(config.include_special);
    assert!(!config.special_chars.is_empty());
}

// ============================================================================
// Security Requirements Tests
// ============================================================================

#[test]
fn test_generated_password_meets_security_requirements() {
    // Test that generated passwords meet all security requirements from spec
    for _ in 0..20 {
        let password = generate_password(None);

        // Requirement 2.2: Length should be 16 characters (or more)
        assert!(password.len() >= 16);

        // Requirement 2.3: Should contain all character types
        assert!(password.chars().any(|c| c.is_ascii_uppercase()));
        assert!(password.chars().any(|c| c.is_ascii_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
        assert!(password.chars().any(|c| !c.is_alphanumeric()));

        // Should have high entropy (strong password)
        let entropy = estimate_entropy(&password);
        assert!(entropy >= 64.0, "Generated password should have high entropy");

        // Should have strong rating
        let rating = get_strength_rating(&password);
        assert!(rating >= 4, "Generated password should be strong");
    }
}

#[test]
fn test_password_validation_enforces_complexity() {
    // Test various weak passwords that should fail validation
    let weak_passwords = vec![
        "password",           // No uppercase, digits, special
        "PASSWORD",           // No lowercase, digits, special
        "12345678",           // No letters, special
        "Password",           // No digits, special
        "Password123",        // No special
        "password123!",       // No uppercase
        "PASSWORD123!",       // No lowercase
        "Pass1!",             // Too short
    ];

    for password in weak_passwords {
        assert!(
            validate_password(password, None).is_err(),
            "Weak password '{}' should fail validation",
            password
        );
    }
}

#[test]
fn test_cryptographic_randomness_distribution() {
    // Generate many passwords and check character distribution
    let mut char_counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    let iterations = 1000;

    for _ in 0..iterations {
        let password = generate_password(None);
        for c in password.chars() {
            *char_counts.entry(c).or_insert(0) += 1;
        }
    }

    // Should have a good distribution of different characters
    assert!(
        char_counts.len() > 50,
        "Should use a wide variety of characters"
    );
}
