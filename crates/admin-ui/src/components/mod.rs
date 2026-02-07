//! UI Components module

pub mod forms;
pub mod feedback;
pub mod loading;
pub mod examples;
pub mod layout;
pub mod layout_example;

// Re-export commonly used components
pub use forms::{Button, Checkbox, Input, Select, TextArea};
pub use feedback::{
    Alert, ErrorMessage, FieldError, InfoMessage, SuccessMessage, Toast, ValidationFeedback,
    WarningMessage,
};
pub use loading::{
    InlineLoader, LoadingOverlay, LoadingState, ProgressBar, ProgressSteps, Skeleton, Spinner,
};
pub use layout::{AdminLayout, Breadcrumb, BreadcrumbItem, Header, NavItem, Sidebar};