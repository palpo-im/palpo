//! Loading indicators and progress components

use dioxus::prelude::*;

/// Props for the Spinner component
#[derive(Props, Clone, PartialEq)]
pub struct SpinnerProps {
    /// Spinner size (small, medium, large)
    #[props(default = "medium".to_string())]
    pub size: String,
    /// Optional loading message
    #[props(default = None)]
    pub message: Option<String>,
}

/// Spinner loading indicator
#[component]
pub fn Spinner(props: SpinnerProps) -> Element {
    let spinner_class = format!("spinner spinner-{}", props.size);

    rsx! {
        div { class: "spinner-container",
            div { class: "{spinner_class}" }
            if let Some(message) = &props.message {
                div { class: "spinner-message", "{message}" }
            }
        }
    }
}

/// Props for the LoadingOverlay component
#[derive(Props, Clone, PartialEq)]
pub struct LoadingOverlayProps {
    /// Whether the overlay is visible
    pub visible: bool,
    /// Loading message
    #[props(default = "加载中...".to_string())]
    pub message: String,
}

/// Full-screen loading overlay
#[component]
pub fn LoadingOverlay(props: LoadingOverlayProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        div { class: "loading-overlay",
            div { class: "loading-overlay-content",
                Spinner { size: "large".to_string(), message: Some(props.message.clone()) }
            }
        }
    }
}

/// Props for the ProgressBar component
#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    /// Current progress value (0-100)
    pub value: f32,
    /// Maximum value (default 100)
    #[props(default = 100.0)]
    pub max: f32,
    /// Whether to show percentage text
    #[props(default = true)]
    pub show_percentage: bool,
    /// Progress bar variant (primary, success, warning, danger)
    #[props(default = "primary".to_string())]
    pub variant: String,
    /// Optional label
    #[props(default = None)]
    pub label: Option<String>,
}

/// Progress bar component
#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    let percentage = (props.value / props.max * 100.0).min(100.0).max(0.0);
    let progress_class = format!("progress-bar progress-bar-{}", props.variant);

    rsx! {
        div { class: "progress-container",
            if let Some(label) = &props.label {
                div { class: "progress-label", "{label}" }
            }
            div { class: "progress",
                div {
                    class: "{progress_class}",
                    style: "width: {percentage}%",
                    if props.show_percentage {
                        span { class: "progress-text", "{percentage:.0}%" }
                    }
                }
            }
        }
    }
}

/// Props for the Skeleton component
#[derive(Props, Clone, PartialEq)]
pub struct SkeletonProps {
    /// Skeleton variant (text, circle, rectangle)
    #[props(default = "text".to_string())]
    pub variant: String,
    /// Width (CSS value)
    #[props(default = "100%".to_string())]
    pub width: String,
    /// Height (CSS value)
    #[props(default = "1em".to_string())]
    pub height: String,
    /// Number of lines (for text variant)
    #[props(default = 1)]
    pub lines: usize,
}

/// Skeleton loading placeholder
#[component]
pub fn Skeleton(props: SkeletonProps) -> Element {
    let skeleton_class = format!("skeleton skeleton-{}", props.variant);

    if props.variant == "text" && props.lines > 1 {
        rsx! {
            div { class: "skeleton-text-container",
                for _ in 0..props.lines {
                    div {
                        class: "{skeleton_class}",
                        style: "width: {props.width}; height: {props.height}"
                    }
                }
            }
        }
    } else {
        rsx! {
            div {
                class: "{skeleton_class}",
                style: "width: {props.width}; height: {props.height}"
            }
        }
    }
}

/// Props for the LoadingState component
#[derive(Props, Clone, PartialEq)]
pub struct LoadingStateProps<T: Clone + PartialEq + 'static> {
    /// Loading state
    pub loading: bool,
    /// Error message if any
    pub error: Option<String>,
    /// Data if loaded
    pub data: Option<T>,
    /// Render function for loaded data
    pub children: Element,
}

/// Generic loading state component that handles loading, error, and success states
#[component]
pub fn LoadingState<T: Clone + PartialEq + 'static>(props: LoadingStateProps<T>) -> Element {
    if props.loading {
        return rsx! {
            div { class: "loading-state",
                Spinner { message: Some("加载中...".to_string()) }
            }
        };
    }

    if let Some(error) = &props.error {
        return rsx! {
            div { class: "error-state",
                div { class: "error-message",
                    span { class: "error-icon", "⚠" }
                    span { "{error}" }
                }
            }
        };
    }

    if props.data.is_none() {
        return rsx! {
            div { class: "empty-state",
                "暂无数据"
            }
        };
    }

    rsx! {
        {props.children}
    }
}

/// Props for the InlineLoader component
#[derive(Props, Clone, PartialEq)]
pub struct InlineLoaderProps {
    /// Whether the loader is visible
    pub visible: bool,
    /// Optional loading text
    #[props(default = None)]
    pub text: Option<String>,
}

/// Inline loading indicator for buttons or small spaces
#[component]
pub fn InlineLoader(props: InlineLoaderProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    rsx! {
        span { class: "inline-loader",
            span { class: "inline-spinner" }
            if let Some(text) = &props.text {
                span { class: "inline-loader-text", "{text}" }
            }
        }
    }
}

/// Props for the ProgressSteps component
#[derive(Props, Clone, PartialEq)]
pub struct ProgressStepsProps {
    /// Current step (0-indexed)
    pub current_step: usize,
    /// List of step labels
    pub steps: Vec<String>,
}

/// Progress steps indicator for multi-step processes
#[component]
pub fn ProgressSteps(props: ProgressStepsProps) -> Element {
    rsx! {
        div { class: "progress-steps",
            for (index, step) in props.steps.iter().enumerate() {
                div {
                    class: if index < props.current_step {
                        "progress-step completed"
                    } else if index == props.current_step {
                        "progress-step active"
                    } else {
                        "progress-step"
                    },
                    div { class: "step-number", "{index + 1}" }
                    div { class: "step-label", "{step}" }
                    if index < props.steps.len() - 1 {
                        div { class: "step-connector" }
                    }
                }
            }
        }
    }
}
