//! Form components for the admin UI

use dioxus::prelude::*;
use super::feedback::ErrorMessage;

/// Props for the Input component
#[derive(Props, Clone, PartialEq)]
pub struct InputProps {
    /// Input label
    pub label: String,
    /// Input type (text, password, email, etc.)
    #[props(default = "text".to_string())]
    pub input_type: String,
    /// Current value
    pub value: String,
    /// Placeholder text
    #[props(default = String::new())]
    pub placeholder: String,
    /// Whether the input is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Whether the input is required
    #[props(default = false)]
    pub required: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when value changes
    pub oninput: EventHandler<String>,
}

/// Text input component with label and validation
#[component]
pub fn Input(props: InputProps) -> Element {
    let has_error = props.error.is_some();
    let input_class = if has_error {
        "form-input error"
    } else {
        "form-input"
    };

    rsx! {
        div { class: "form-group",
            label { class: "form-label",
                "{props.label}"
                if props.required {
                    span { class: "required", " *" }
                }
            }
            input {
                r#type: "{props.input_type}",
                class: "{input_class}",
                value: "{props.value}",
                placeholder: "{props.placeholder}",
                readonly: props.readonly,
                required: props.required,
                oninput: move |evt| props.oninput.call(evt.value().clone())
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the TextArea component
#[derive(Props, Clone, PartialEq)]
pub struct TextAreaProps {
    /// TextArea label
    pub label: String,
    /// Current value
    pub value: String,
    /// Placeholder text
    #[props(default = String::new())]
    pub placeholder: String,
    /// Number of rows
    #[props(default = 4.0)]
    pub rows: f64,
    /// Whether the textarea is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Whether the textarea is required
    #[props(default = false)]
    pub required: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when value changes
    pub oninput: EventHandler<String>,
}

/// TextArea component with label and validation
#[component]
pub fn TextArea(props: TextAreaProps) -> Element {
    let has_error = props.error.is_some();
    let textarea_class = if has_error {
        "form-textarea error"
    } else {
        "form-textarea"
    };

    rsx! {
        div { class: "form-group",
            label { class: "form-label",
                "{props.label}"
                if props.required {
                    span { class: "required", " *" }
                }
            }
            textarea {
                class: "{textarea_class}",
                value: "{props.value}",
                placeholder: "{props.placeholder}",
                rows: props.rows,
                readonly: props.readonly,
                required: props.required,
                oninput: move |evt| props.oninput.call(evt.value().clone())
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the Select component
#[derive(Props, Clone, PartialEq)]
pub struct SelectProps {
    /// Select label
    pub label: String,
    /// Current selected value
    pub value: String,
    /// Available options (value, label)
    pub options: Vec<(String, String)>,
    /// Whether the select is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Whether the select is required
    #[props(default = false)]
    pub required: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when value changes
    pub onchange: EventHandler<String>,
}

/// Select dropdown component with label and validation
#[component]
pub fn Select(props: SelectProps) -> Element {
    let has_error = props.error.is_some();
    let select_class = if has_error {
        "form-select error"
    } else {
        "form-select"
    };

    rsx! {
        div { class: "form-group",
            label { class: "form-label",
                "{props.label}"
                if props.required {
                    span { class: "required", " *" }
                }
            }
            select {
                class: "{select_class}",
                value: "{props.value}",
                disabled: props.readonly,
                required: props.required,
                onchange: move |evt| props.onchange.call(evt.value().clone()),
                for (value, label) in props.options.iter() {
                    option { value: "{value}", "{label}" }
                }
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the Checkbox component
#[derive(Props, Clone, PartialEq)]
pub struct CheckboxProps {
    /// Checkbox label
    pub label: String,
    /// Current checked state
    pub checked: bool,
    /// Whether the checkbox is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when checked state changes
    pub onchange: EventHandler<bool>,
}

/// Checkbox component with label and validation
#[component]
pub fn Checkbox(props: CheckboxProps) -> Element {
    let has_error = props.error.is_some();
    let checkbox_class = if has_error {
        "form-checkbox error"
    } else {
        "form-checkbox"
    };

    rsx! {
        div { class: "form-group checkbox-group",
            label { class: "checkbox-label",
                input {
                    r#type: "checkbox",
                    class: "{checkbox_class}",
                    checked: props.checked,
                    disabled: props.readonly,
                    onchange: move |evt| props.onchange.call(evt.checked())
                }
                span { "{props.label}" }
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the Button component
#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    /// Button text
    pub children: Element,
    /// Button variant (primary, secondary, danger, success)
    #[props(default = "primary".to_string())]
    pub variant: String,
    /// Button size (small, medium, large)
    #[props(default = "medium".to_string())]
    pub size: String,
    /// Whether the button is disabled
    #[props(default = false)]
    pub disabled: bool,
    /// Whether the button is in loading state
    #[props(default = false)]
    pub loading: bool,
    /// Button type (button, submit, reset)
    #[props(default = "button".to_string())]
    pub button_type: String,
    /// Click handler
    #[props(default = EventHandler::default())]
    pub onclick: EventHandler<MouseEvent>,
}

/// Button component with variants and loading state
#[component]
pub fn Button(props: ButtonProps) -> Element {
    let button_class = format!("btn btn-{} btn-{}", props.variant, props.size);
    let is_disabled = props.disabled || props.loading;

    rsx! {
        button {
            r#type: "{props.button_type}",
            class: "{button_class}",
            disabled: is_disabled,
            onclick: move |evt| {
                if !is_disabled {
                    props.onclick.call(evt)
                }
            },
            if props.loading {
                span { class: "btn-spinner" }
            }
            {props.children}
        }
    }
}
