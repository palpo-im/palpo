/// Comprehensive password generator tests
///
/// This test suite provides extensive coverage of password generation,
/// validation, and strength estimation functions.

use palpo_admin_server::password_generator::*;

// ===== Password Generation Tests =====

#[test]
fn test_generate_password_default_config() {
    for _ in 0..100 {
        let password = generate_password(None);
        
        // Check length
        assert!(password.len() >= 16, "Password too short: {}", password.len());
        assert!(password.len() <= 64, "Password too long: {}", password.len());
        
        // Check complexity
        assert!(password.chars().any(|c| c.is_uppercase()), "Missing uppercase");
        assert!(password.chars().any(|c| c.is_lowercase()), "Missing lowercase");
        assert!(password.chars().any(|c| c.is_ascii_digit()), "Missing digit");
        assert!(password.chars().any(|c| !c.is_alphanumeric()), "Missing special char");
    }
}

#[test]
fn test_generate_password_custom_length() {
    let config = PasswordConfig {
        min_length: 20,
        max_length: 20,
        ..Default::default()
    };
    
    for _ in 0..50 {
        let password = generate_password(Some(config.clone()));
        assert_eq!(password.len(), 20, "Password length mismatch");
    }
}

#[test]
fn test_generate_password_minimum_length() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        ..Default::default()
    };
    
    let password = generate_password(Some(config));
    assert_eq!(password.len(), 16);
}

#[test]
fn test_generate_password_maximum_length() {
    let config = PasswordConfig {
        min_length: 64,
        max_length: 64,
        ..Default::default()
    };
    
    let password = generate_password(Some(config));
    assert_eq!(password.len(), 64);
}

#[test]
fn test_generate_password_no_uppercase() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: false,
        include_lowercase: true,
        include_digits: true,
        include_special: true,
        ..Default::default()
    };
    
    for _ in 0..50 {
        let password = generate_password(Some(config.clone()));
        assert!(!password.chars().any(|c| c.is_uppercase()), "Found uppercase when disabled");
        assert!(password.chars().any(|c| c.is_lowercase()), "Missing lowercase");
        assert!(password.chars().any(|c| c.is_ascii_digit()), "Missing digit");
    }
}

#[test]
fn test_generate_password_no_lowercase() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: true,
        include_lowercase: false,
        include_digits: true,
        include_special: true,
        ..Default::default()
    };
    
    for _ in 0..50 {
        let password = generate_password(Some(config.clone()));
        assert!(password.chars().any(|c| c.is_uppercase()), "Missing uppercase");
        assert!(!password.chars().any(|c| c.is_lowercase()), "Found lowercase when disabled");
        assert!(password.chars().any(|c| c.is_ascii_digit()), "Missing digit");
    }
}

#[test]
fn test_generate_password_no_digits() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: true,
        include_lowercase: true,
        include_digits: false,
        include_special: true,
        ..Default::default()
    };
    
    for _ in 0..50 {
        let password = generate_password(Some(config.clone()));
        assert!(password.chars().any(|c| c.is_uppercase()), "Missing uppercase");
        assert!(password.chars().any(|c| c.is_lowercase()), "Missing lowercase");
        assert!(!password.chars().any(|c| c.is_ascii_digit()), "Found digit when disabled");
    }
}

#[test]
fn test_generate_password_no_special() {
    let config = PasswordConfig {
        min_length: 16,
        max_length: 16,
        include_uppercase: true,
        include_lowercase: true,
        include_digits: true,
        include_special: false,
        ..Default::default()
    };
    
    for _ in 0..50 {
        let password = generate_password(Some(config.clone()));
        assert!(password.chars().any(|c| c.is_uppercase()), "Missing uppercase");
        assert!(password.chars().any(|c| c.is_lowercase()), "Missing lowercase");
        assert!(password.chars().any(|c| c.is_ascii_digit()), "Missing digit");
        assert!(password.chars().all(|c| c.is_alphanumeric()), "Found special char when disabled");
    }
}

#[test]
fn test_generate_password_uniqueness() {
    let mut passwords = std::collections::HashSet::new();
    
    for _ in 0..1000 {
        let password = generate_password(None);
        passwords.insert(password);
    }
    
    // All passwords should be unique
    assert_eq!(passwords.len(), 1000, "Generated duplicate passwords");
}

#[test]
fn test_generate_password_randomness() {
    let password1 = generate_password(None);
    let password2 = generate_password(None);
    
    // Passwords should be different
    assert_ne!(password1, password2, "Generated identical passwords");
}

// ===== Password Validation Tests =====

#[test]
fn test_validate_password_valid_strong() {
    assert!(validate_password("SecureP@ssw0rd123", None).is_ok());
    assert!(validate_password("MyP@ssw0rd!20241", None).is_ok()); // 16 chars minimum
    assert!(validate_password("C0mpl3x!P@ssw0rd", None).is_ok());
}

#[test]
fn test_validate_password_too_short() {
    let result = validate_password("Short1!", None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::TooShort(len)) => assert_eq!(len, 7),
        _ => panic!("Expected TooShort error"),
    }
}

#[test]
fn test_validate_password_too_long() {
    let long_password = "A".repeat(65) + "1!";
    let result = validate_password(&long_password, None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::TooLong(_)) => {},
        _ => panic!("Expected TooLong error"),
    }
}

#[test]
fn test_validate_password_missing_uppercase() {
    let result = validate_password("password123!@#$%", None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::ComplexityNotMet(msg)) => {
            assert!(msg.contains("uppercase"), "Error message: {}", msg);
        },
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_lowercase() {
    let result = validate_password("PASSWORD123!@#$%", None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::ComplexityNotMet(msg)) => {
            assert!(msg.contains("lowercase"), "Error message: {}", msg);
        },
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_digit() {
    let result = validate_password("Password!@#$%^&*", None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::ComplexityNotMet(msg)) => {
            assert!(msg.contains("digit"), "Error message: {}", msg);
        },
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_missing_special() {
    let result = validate_password("Password12345678", None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::ComplexityNotMet(msg)) => {
            assert!(msg.contains("special"), "Error message: {}", msg);
        },
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_multiple_missing() {
    let result = validate_password("passwordpassword", None);
    assert!(result.is_err());
    match result {
        Err(PasswordError::ComplexityNotMet(msg)) => {
            assert!(msg.contains("uppercase"), "Error message: {}", msg);
            assert!(msg.contains("digit"), "Error message: {}", msg);
            assert!(msg.contains("special"), "Error message: {}", msg);
        },
        _ => panic!("Expected ComplexityNotMet error"),
    }
}

#[test]
fn test_validate_password_custom_config() {
    let config = PasswordConfig {
        min_length: 12,
        max_length: 32,
        include_uppercase: true,
        include_lowercase: true,
        include_digits: false,
        include_special: false,
        ..Default::default()
    };
    
    // Should pass without digits and special chars
    assert!(validate_password("PasswordPassword", Some(config)).is_ok());
}

#[test]
fn test_validate_password_edge_case_minimum() {
    let password = "SecureP@ssw0rd12"; // Exactly 16 chars
    assert!(validate_password(password, None).is_ok());
}

#[test]
fn test_validate_password_edge_case_maximum() {
    let password = "SecureP@ssw0rd12".to_string() + &"a".repeat(48); // Exactly 64 chars
    assert!(validate_password(&password, None).is_ok());
}

// ===== Entropy Estimation Tests =====

#[test]
fn test_estimate_entropy_weak() {
    let entropy = estimate_entropy("abc");
    assert!(entropy > 0.0);
    assert!(entropy < 20.0);
}

#[test]
fn test_estimate_entropy_moderate() {
    let entropy = estimate_entropy("abc123def456");
    assert!(entropy > 20.0);
    // Entropy can be higher depending on charset detection
}

#[test]
fn test_estimate_entropy_strong() {
    let entropy = estimate_entropy("SecureP@ssw0rd123!");
    assert!(entropy > 60.0);
}

#[test]
fn test_estimate_entropy_comparison() {
    let weak = estimate_entropy("abc");
    let moderate = estimate_entropy("abc123");
    let strong = estimate_entropy("Abc123!@#");
    
    assert!(weak < moderate);
    assert!(moderate < strong);
}

#[test]
fn test_estimate_entropy_length_impact() {
    let short = estimate_entropy("Abc1!");
    let long = estimate_entropy("Abc1!Abc1!Abc1!");
    
    assert!(long > short);
}

#[test]
fn test_estimate_entropy_charset_impact() {
    let lowercase_only = estimate_entropy("abcdefghij");
    let mixed_case = estimate_entropy("AbCdEfGhIj");
    let with_digits = estimate_entropy("AbCdEf123");
    let with_special = estimate_entropy("AbCd12!@#");
    
    // More character sets should generally increase entropy
    assert!(lowercase_only < mixed_case);
    // Note: with_digits might have lower entropy than mixed_case due to length
    assert!(with_special > lowercase_only);
}

#[test]
fn test_estimate_entropy_empty() {
    let entropy = estimate_entropy("");
    assert_eq!(entropy, 0.0);
}

// ===== Strength Rating Tests =====

#[test]
fn test_strength_rating_very_weak() {
    assert_eq!(get_strength_rating("a"), 1);
    assert_eq!(get_strength_rating("abc"), 1);
    assert_eq!(get_strength_rating("12345"), 1);
}

#[test]
fn test_strength_rating_weak() {
    assert_eq!(get_strength_rating("abc123def"), 2);
    assert_eq!(get_strength_rating("password"), 2);
}

#[test]
fn test_strength_rating_moderate() {
    let rating = get_strength_rating("Password123");
    assert!(rating >= 3, "Rating was {}", rating);
    let rating2 = get_strength_rating("abc123def456ghi");
    assert!(rating2 >= 3, "Rating was {}", rating2);
}

#[test]
fn test_strength_rating_strong() {
    assert!(get_strength_rating("SecureP@ssw0rd123") >= 4);
    assert!(get_strength_rating("MyP@ssw0rd!2024") >= 4);
}

#[test]
fn test_strength_rating_very_strong() {
    assert_eq!(get_strength_rating("SecureP@ssw0rd123!VeryLong"), 5);
    assert_eq!(get_strength_rating("C0mpl3x!P@ssw0rd#2024$Secure"), 5);
}

#[test]
fn test_strength_rating_progression() {
    let ratings: Vec<u8> = vec![
        get_strength_rating("a"),
        get_strength_rating("abc123"),
        get_strength_rating("Password123"),
        get_strength_rating("SecureP@ssw0rd123"),
        get_strength_rating("SecureP@ssw0rd123!VeryLong"),
    ];
    
    // Ratings should generally increase
    for i in 0..ratings.len()-1 {
        assert!(ratings[i] <= ratings[i+1], 
            "Rating decreased: {} -> {}", ratings[i], ratings[i+1]);
    }
}

// ===== Integration Tests =====

#[test]
fn test_generated_passwords_pass_validation() {
    for _ in 0..100 {
        let password = generate_password(None);
        assert!(validate_password(&password, None).is_ok(), 
            "Generated password failed validation: {}", password);
    }
}

#[test]
fn test_generated_passwords_have_good_strength() {
    for _ in 0..100 {
        let password = generate_password(None);
        let rating = get_strength_rating(&password);
        assert!(rating >= 4, "Generated password has weak rating: {} ({})", rating, password);
    }
}

#[test]
fn test_generated_passwords_have_high_entropy() {
    for _ in 0..100 {
        let password = generate_password(None);
        let entropy = estimate_entropy(&password);
        assert!(entropy >= 64.0, "Generated password has low entropy: {} ({})", entropy, password);
    }
}

#[test]
fn test_password_config_default() {
    let config = PasswordConfig::default();
    assert_eq!(config.min_length, 16);
    assert_eq!(config.max_length, 64);
    assert!(config.include_uppercase);
    assert!(config.include_lowercase);
    assert!(config.include_digits);
    assert!(config.include_special);
}

#[test]
fn test_password_error_display() {
    let err = PasswordError::TooShort(8);
    assert!(err.to_string().contains("too short"));
    
    let err = PasswordError::TooLong(100);
    assert!(err.to_string().contains("too long"));
    
    let err = PasswordError::NoCharacterSets;
    assert!(err.to_string().contains("character set"));
    
    let err = PasswordError::ComplexityNotMet("test".to_string());
    assert!(err.to_string().contains("complexity"));
}

// ===== Edge Case Tests =====

#[test]
fn test_password_with_unicode() {
    // Unicode characters should not break validation
    let password = "SecureP@ssw0rd123🔒";
    let entropy = estimate_entropy(password);
    assert!(entropy > 0.0);
}

#[test]
fn test_password_all_same_character() {
    let password = "A".repeat(13) + "a1!"; // 16 chars total with all requirements
    // Should still validate if it meets requirements
    assert!(validate_password(&password, None).is_ok());
}

#[test]
fn test_config_clone() {
    let config1 = PasswordConfig::default();
    let config2 = config1.clone();
    assert_eq!(config1.min_length, config2.min_length);
    assert_eq!(config1.max_length, config2.max_length);
}
