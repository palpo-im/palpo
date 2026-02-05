//! Configuration state management hook

use dioxus::prelude::*;
use crate::models::WebConfigData;
use crate::services::ConfigService;
use crate::app::AppState;
use wasm_bindgen_futures::spawn_local;

/// Configuration context and methods
#[derive(Clone)]
pub struct ConfigContext {
    pub app_state: Signal<AppState>,
    pub config_service: ConfigService,
}

impl ConfigContext {
    /// Load configuration from server
    pub fn load_config(&self) {
        let config_service = self.config_service.clone();
        let mut app_state = self.app_state;
        
        spawn_local(async move {
            app_state.with_mut(|state| {
                state.is_loading = true;
                state.error = None;
            });
            
            match config_service.get_config().await {
                Ok(config) => {
                    app_state.with_mut(|state| {
                        state.config = Some(config);
                        state.is_loading = false;
                    });
                }
                Err(error) => {
                    app_state.with_mut(|state| {
                        state.error = Some(error.user_message());
                        state.is_loading = false;
                    });
                }
            }
        });
    }

    /// Save configuration to server
    pub fn save_config(&self, config: WebConfigData) {
        let config_service = self.config_service.clone();
        let mut app_state = self.app_state;
        
        spawn_local(async move {
            app_state.with_mut(|state| {
                state.is_loading = true;
                state.error = None;
            });
            
            match config_service.update_config(config.clone()).await {
                Ok(_) => {
                    app_state.with_mut(|state| {
                        state.config = Some(config);
                        state.is_loading = false;
                    });
                }
                Err(error) => {
                    app_state.with_mut(|state| {
                        state.error = Some(error.user_message());
                        state.is_loading = false;
                    });
                }
            }
        });
    }

    /// Validate configuration
    pub fn validate_config(&self, config: WebConfigData) {
        let config_service = self.config_service.clone();
        let mut app_state = self.app_state;
        
        spawn_local(async move {
            match config_service.validate_config(config).await {
                Ok(validation_result) => {
                    if !validation_result.valid {
                        let error_messages: Vec<String> = validation_result.errors
                            .iter()
                            .map(|e| e.message.clone())
                            .collect();
                        app_state.with_mut(|state| {
                            state.error = Some(format!("配置验证失败: {}", error_messages.join(", ")));
                        });
                    } else {
                        app_state.with_mut(|state| {
                            state.error = None;
                        });
                    }
                }
                Err(error) => {
                    app_state.with_mut(|state| {
                        state.error = Some(error.user_message());
                    });
                }
            }
        });
    }

    /// Get current configuration
    pub fn current_config(&self) -> Option<WebConfigData> {
        self.app_state.read().config.clone()
    }

    /// Check if loading
    pub fn is_loading(&self) -> bool {
        self.app_state.read().is_loading
    }

    /// Get current error
    pub fn error(&self) -> Option<String> {
        self.app_state.read().error.clone()
    }

    /// Clear error
    pub fn clear_error(&self) {
        let mut app_state = self.app_state;
        app_state.with_mut(|state| {
            state.error = None;
        });
    }
}

/// Hook for configuration management in Dioxus components
pub fn use_config() -> ConfigContext {
    // Get the app state from context
    let app_state = use_context::<Signal<AppState>>();
    let config_service = ConfigService::default();

    ConfigContext {
        app_state,
        config_service,
    }
}

/// Hook for automatically loading config on component mount
pub fn use_config_loader() -> ConfigContext {
    let config_context = use_config();
    
    // Load config on component mount if not already loaded
    use_effect({
        let config_context = config_context.clone();
        move || {
            if config_context.current_config().is_none() && !config_context.is_loading() {
                config_context.load_config();
            }
        }
    });

    config_context
}