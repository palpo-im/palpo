/// Session Manager for Web UI Admin Authentication
///
/// Manages authenticated sessions for Web UI admin users. Sessions are stored
/// in-memory using RwLock for thread-safe concurrent access. Each session has
/// a cryptographically secure random token and an expiration timestamp.
///
/// **Key Features:**
/// - Thread-safe in-memory session storage
/// - Cryptographically secure token generation
/// - Automatic expiration checking
/// - Session cleanup for expired sessions
/// - 24-hour session duration by default

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use tokio::sync::RwLock;

use super::types::{AdminError, SessionToken};

/// Internal session data stored in memory
///
/// Contains the username and expiration timestamp for each active session.
#[derive(Debug, Clone)]
struct SessionData {
    /// Username associated with this session (always "admin" for Web UI)
    username: String,
    /// Timestamp when this session expires
    expires_at: DateTime<Utc>,
}

/// Session Manager for handling authenticated sessions
///
/// Provides thread-safe session management with in-memory storage.
/// Sessions are identified by cryptographically random tokens and
/// automatically expire after a configurable duration.
///
/// # Example
///
/// ```rust,ignore
/// let manager = SessionManager::new();
///
/// // Create a new session
/// let token = manager.create_session("admin").await?;
///
/// // Validate the session
/// let username = manager.validate_session(&token.token).await?;
///
/// // Invalidate the session (logout)
/// manager.invalidate_session(&token.token).await?;
/// ```
pub struct SessionManager {
    /// In-memory storage of active sessions
    /// Key: session token, Value: session data
    sessions: Arc<RwLock<HashMap<String, SessionData>>>,
}

impl SessionManager {
    /// Default session duration in hours
    const SESSION_DURATION_HOURS: i64 = 24;

    /// Length of the session token in bytes (32 bytes = 64 hex characters)
    const TOKEN_LENGTH: usize = 32;

    /// Creates a new SessionManager instance
    ///
    /// Initializes an empty in-memory session store.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Creates a new session for the given username
    ///
    /// Generates a cryptographically secure random token and stores the session
    /// with an expiration timestamp 24 hours in the future.
    ///
    /// # Arguments
    ///
    /// * `username` - The username to associate with this session (typically "admin")
    ///
    /// # Returns
    ///
    /// Returns a `SessionToken` containing the token string and expiration timestamp.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let token = manager.create_session("admin").await?;
    /// println!("Session token: {}", token.token);
    /// println!("Expires at: {}", token.expires_at);
    /// ```
    pub async fn create_session(&self, username: &str) -> Result<SessionToken, AdminError> {
        let token = Self::generate_token();
        let expires_at = Utc::now() + Duration::hours(Self::SESSION_DURATION_HOURS);

        let mut sessions = self.sessions.write().await;
        sessions.insert(
            token.clone(),
            SessionData {
                username: username.to_string(),
                expires_at,
            },
        );

        Ok(SessionToken { token, expires_at })
    }

    /// Validates a session token and returns the associated username
    ///
    /// Checks if the token exists and has not expired. If the token is expired,
    /// it is automatically removed from the session store.
    ///
    /// # Arguments
    ///
    /// * `token` - The session token to validate
    ///
    /// # Returns
    ///
    /// Returns the username associated with the session if valid.
    ///
    /// # Errors
    ///
    /// * `AdminError::InvalidSessionToken` - Token does not exist
    /// * `AdminError::SessionExpired` - Token has expired
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// match manager.validate_session(&token).await {
    ///     Ok(username) => println!("Valid session for: {}", username),
    ///     Err(AdminError::SessionExpired) => println!("Session expired"),
    ///     Err(AdminError::InvalidSessionToken) => println!("Invalid token"),
    ///     Err(e) => println!("Error: {}", e),
    /// }
    /// ```
    pub async fn validate_session(&self, token: &str) -> Result<String, AdminError> {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get(token) {
            if session.expires_at > Utc::now() {
                return Ok(session.username.clone());
            } else {
                // Remove expired session
                sessions.remove(token);
                return Err(AdminError::SessionExpired);
            }
        }

        Err(AdminError::InvalidSessionToken)
    }

    /// Invalidates a session (logout)
    ///
    /// Removes the session from the in-memory store, effectively logging out
    /// the user. This operation is idempotent - calling it multiple times with
    /// the same token has no additional effect.
    ///
    /// # Arguments
    ///
    /// * `token` - The session token to invalidate
    ///
    /// # Returns
    ///
    /// Always returns `Ok(())`, even if the token doesn't exist.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// manager.invalidate_session(&token).await?;
    /// println!("User logged out successfully");
    /// ```
    pub async fn invalidate_session(&self, token: &str) -> Result<(), AdminError> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(token);
        Ok(())
    }

    /// Cleans up expired sessions from memory
    ///
    /// Removes all sessions that have passed their expiration timestamp.
    /// This method should be called periodically to prevent memory leaks
    /// from accumulating expired sessions.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Run cleanup periodically
    /// tokio::spawn(async move {
    ///     let mut interval = tokio::time::interval(Duration::hours(1).to_std().unwrap());
    ///     loop {
    ///         interval.tick().await;
    ///         manager.cleanup_expired_sessions().await;
    ///     }
    /// });
    /// ```
    pub async fn cleanup_expired_sessions(&self) {
        let mut sessions = self.sessions.write().await;
        let now = Utc::now();
        sessions.retain(|_, session| session.expires_at > now);
    }

    /// Generates a cryptographically secure random token
    ///
    /// Creates a random byte array and encodes it as a hexadecimal string.
    /// Uses the system's cryptographically secure random number generator.
    ///
    /// # Returns
    ///
    /// A 64-character hexadecimal string (32 bytes of random data)
    fn generate_token() -> String {
        let mut rng = rand::thread_rng();
        let bytes: Vec<u8> = (0..Self::TOKEN_LENGTH).map(|_| rng.r#gen()).collect();
        hex::encode(bytes)
    }

    /// Returns the number of active sessions (for testing/monitoring)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let count = manager.session_count().await;
    /// println!("Active sessions: {}", count);
    /// ```
    #[cfg(test)]
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_session() {
        let manager = SessionManager::new();
        let result = manager.create_session("admin").await;

        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.token.is_empty());
        assert!(token.expires_at > Utc::now());
    }

    #[tokio::test]
    async fn test_validate_session() {
        let manager = SessionManager::new();
        let token = manager.create_session("admin").await.unwrap();

        let result = manager.validate_session(&token.token).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "admin");
    }

    #[tokio::test]
    async fn test_validate_invalid_token() {
        let manager = SessionManager::new();
        let result = manager.validate_session("invalid_token").await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AdminError::InvalidSessionToken));
    }

    #[tokio::test]
    async fn test_invalidate_session() {
        let manager = SessionManager::new();
        let token = manager.create_session("admin").await.unwrap();

        // Session should be valid
        assert!(manager.validate_session(&token.token).await.is_ok());

        // Invalidate the session
        manager.invalidate_session(&token.token).await.unwrap();

        // Session should now be invalid
        let result = manager.validate_session(&token.token).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AdminError::InvalidSessionToken));
    }

    #[tokio::test]
    async fn test_session_expiration() {
        let manager = SessionManager::new();
        let token_str = SessionManager::generate_token();

        // Create an expired session manually
        {
            let mut sessions = manager.sessions.write().await;
            sessions.insert(
                token_str.clone(),
                SessionData {
                    username: "admin".to_string(),
                    expires_at: Utc::now() - Duration::hours(1), // Expired 1 hour ago
                },
            );
        }

        // Validation should fail with SessionExpired
        let result = manager.validate_session(&token_str).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AdminError::SessionExpired));

        // Session should be removed after validation attempt
        assert_eq!(manager.session_count().await, 0);
    }

    #[tokio::test]
    async fn test_cleanup_expired_sessions() {
        let manager = SessionManager::new();

        // Create a valid session
        let valid_token = manager.create_session("admin").await.unwrap();

        // Create an expired session manually
        let expired_token = SessionManager::generate_token();
        {
            let mut sessions = manager.sessions.write().await;
            sessions.insert(
                expired_token.clone(),
                SessionData {
                    username: "admin".to_string(),
                    expires_at: Utc::now() - Duration::hours(1),
                },
            );
        }

        // Should have 2 sessions
        assert_eq!(manager.session_count().await, 2);

        // Cleanup expired sessions
        manager.cleanup_expired_sessions().await;

        // Should have 1 session left (the valid one)
        assert_eq!(manager.session_count().await, 1);

        // Valid session should still work
        assert!(manager.validate_session(&valid_token.token).await.is_ok());
    }

    #[tokio::test]
    async fn test_token_uniqueness() {
        let token1 = SessionManager::generate_token();
        let token2 = SessionManager::generate_token();

        assert_ne!(token1, token2);
        assert_eq!(token1.len(), 64); // 32 bytes = 64 hex chars
        assert_eq!(token2.len(), 64);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let manager = Arc::new(SessionManager::new());
        let mut handles = vec![];

        // Spawn multiple tasks creating sessions concurrently
        for i in 0..10 {
            let manager_clone = Arc::clone(&manager);
            let handle = tokio::spawn(async move {
                manager_clone
                    .create_session(&format!("user{}", i))
                    .await
                    .unwrap()
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        let tokens: Vec<SessionToken> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // All tokens should be unique
        let unique_tokens: std::collections::HashSet<_> =
            tokens.iter().map(|t| &t.token).collect();
        assert_eq!(unique_tokens.len(), 10);

        // All sessions should be valid
        assert_eq!(manager.session_count().await, 10);
    }

    #[tokio::test]
    async fn test_idempotent_invalidation() {
        let manager = SessionManager::new();
        let token = manager.create_session("admin").await.unwrap();

        // Invalidate once
        assert!(manager.invalidate_session(&token.token).await.is_ok());

        // Invalidate again - should still succeed
        assert!(manager.invalidate_session(&token.token).await.is_ok());

        // Invalidate non-existent token - should still succeed
        assert!(manager.invalidate_session("nonexistent").await.is_ok());
    }
}
