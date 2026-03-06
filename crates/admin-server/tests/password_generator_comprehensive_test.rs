/// Property-based tests for password generation
///
/// This test suite verifies essential properties of password generation.
/// Unit tests for specific password generation logic are in the source files.

use palpo_admin_server::password_generator::*;

// ===== Property-Based Tests =====

#[test]
fn test_password_generation_uniqueness() {
    let mut passwords = std::collections::HashSet::new();
    
    for _ in 0..1000 {
        let password = generate_password(None);
        passwords.insert(password);
    }
    
    // All passwords should be unique
    assert_eq!(passwords.len(), 1000, "Generated duplicate passwords");
}

#[test]
fn test_password_complexity_requirements() {
    for _ in 0..100 {
        let password = generate_password(None);
        
        // Verify complexity requirements
        assert!(password.chars().any(|c| c.is_uppercase()), "Missing uppercase");
        assert!(password.chars().any(|c| c.is_lowercase()), "Missing lowercase");
        assert!(password.chars().any(|c| c.is_ascii_digit()), "Missing digit");
        assert!(password.chars().any(|c| !c.is_alphanumeric()), "Missing special char");
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
