//! Feedback components for error messages, validation, and notifications

use dioxus::prelude::*;

/// Props for the ErrorMessage component
#[derive(Props, Clone, PartialEq)]
pub struct ErrorMessageProps {
    /// Error message to display
    pub message: String,
}

/// Error message component for displaying validation errors
#[component]
pub fn ErrorMessage(props: ErrorMessageProps) -> Element {
    rsx! {
        div { class: "error-message",
            span { class: "error-icon", "⚠" }
            span { class: "error-text", "{props.message}" }
        }
    }
}

/// Props for the SuccessMessage component
#[derive(Props, Clone, PartialEq)]
pub struct SuccessMessageProps {
    /// Success message to display
    pub message: String,
}

/// Success message component for displaying success feedback
#[component]
pub fn SuccessMessage(props: SuccessMessageProps) -> Element {
    rsx! {
        div { class: "success-message",
            span { class: "success-icon", "✓" }
            span { class: "success-text", "{props.message}" }
        }
    }
}

/// Props for the WarningMessage component
#[derive(Props, Clone, PartialEq)]
pub struct WarningMessageProps {
    /// Warning message to display
    pub message: String,
}

/// Warning message component for displaying warnings
#[component]
pub fn WarningMessage(props: WarningMessageProps) -> Element {
    rsx! {
        div { class: "warning-message",
            span { class: "warning-icon", "⚠" }
            span { class: "warning-text", "{props.message}" }
        }
    }
}

/// Props for the InfoMessage component
#[derive(Props, Clone, PartialEq)]
pub struct InfoMessageProps {
    /// Info message to display
    pub message: String,
}

/// Info message component for displaying informational messages
#[component]
pub fn InfoMessage(props: InfoMessageProps) -> Element {
    rsx! {
        div { class: "info-message",
            span { class: "info-icon", "ℹ" }
            span { class: "info-text", "{props.message}" }
        }
    }
}

/// Props for the ValidationFeedback component
#[derive(Props, Clone, PartialEq)]
pub struct ValidationFeedbackProps {
    /// List of validation errors
    #[props(default = vec![])]
    pub errors: Vec<String>,
    /// List of validation warnings
    #[props(default = vec![])]
    pub warnings: Vec<String>,
}

/// Validation feedback component for displaying multiple errors and warnings
#[component]
pub fn ValidationFeedback(props: ValidationFeedbackProps) -> Element {
    let has_errors = !props.errors.is_empty();
    let has_warnings = !props.warnings.is_empty();

    if !has_errors && !has_warnings {
        return rsx! {};
    }

    rsx! {
        div { class: "validation-feedback",
            if has_errors {
                div { class: "validation-errors",
                    for error in props.errors.iter() {
                        ErrorMessage { message: error.clone() }
                    }
                }
            }
            if has_warnings {
                div { class: "validation-warnings",
                    for warning in props.warnings.iter() {
                        WarningMessage { message: warning.clone() }
                    }
                }
            }
        }
    }
}

/// Props for the Toast component
#[derive(Props, Clone, PartialEq)]
pub struct ToastProps {
    /// Toast message
    pub message: String,
    /// Toast type (success, error, warning, info)
    #[props(default = "info".to_string())]
    pub toast_type: String,
    /// Whether the toast is visible
    #[props(default = true)]
    pub visible: bool,
    /// Callback when toast is dismissed
    #[props(default = EventHandler::default())]
    pub ondismiss: EventHandler<()>,
}

/// Toast notification component
#[component]
pub fn Toast(props: ToastProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let toast_class = format!("toast toast-{}", props.toast_type);

    rsx! {
        div { class: "{toast_class}",
            span { class: "toast-message", "{props.message}" }
            button {
                class: "toast-close",
                onclick: move |_| props.ondismiss.call(()),
                "×"
            }
        }
    }
}

/// Props for the Alert component
#[derive(Props, Clone, PartialEq)]
pub struct AlertProps {
    /// Alert title
    #[props(default = None)]
    pub title: Option<String>,
    /// Alert message
    pub message: String,
    /// Alert type (success, error, warning, info)
    #[props(default = "info".to_string())]
    pub alert_type: String,
    /// Whether the alert can be dismissed
    #[props(default = false)]
    pub dismissible: bool,
    /// Callback when alert is dismissed
    #[props(default = EventHandler::default())]
    pub ondismiss: EventHandler<()>,
}

/// Alert component for displaying prominent messages
#[component]
pub fn Alert(props: AlertProps) -> Element {
    let alert_class = format!("alert alert-{}", props.alert_type);

    rsx! {
        div { class: "{alert_class}",
            if let Some(title) = &props.title {
                div { class: "alert-title", "{title}" }
            }
            div { class: "alert-message", "{props.message}" }
            if props.dismissible {
                button {
                    class: "alert-close",
                    onclick: move |_| props.ondismiss.call(()),
                    "×"
                }
            }
        }
    }
}

/// Props for the FieldError component
#[derive(Props, Clone, PartialEq)]
pub struct FieldErrorProps {
    /// Field name
    pub field: String,
    /// List of all validation errors
    pub errors: Vec<(String, String)>,
}

/// Field-specific error component that filters errors by field name
#[component]
pub fn FieldError(props: FieldErrorProps) -> Element {
    let field_errors: Vec<String> = props
        .errors
        .iter()
        .filter(|(field, _)| field == &props.field)
        .map(|(_, message)| message.clone())
        .collect();

    if field_errors.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "field-errors",
            for error in field_errors.iter() {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}
