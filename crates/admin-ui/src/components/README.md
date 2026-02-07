# UI Components

This module provides a comprehensive set of reusable UI components for the Palpo admin interface, built with Dioxus.

## Components Overview

### Form Components (`forms.rs`)

Form components with built-in validation and error handling.

#### Input
Text input component with label and validation.

```rust
Input {
    label: "用户名".to_string(),
    value: username(),
    placeholder: "请输入用户名".to_string(),
    required: true,
    error: Some("用户名不能为空".to_string()),
    oninput: move |value| username.set(value)
}
```

**Props:**
- `label`: Input label text
- `input_type`: Input type (text, password, email, etc.) - default: "text"
- `value`: Current value
- `placeholder`: Placeholder text
- `readonly`: Whether input is readonly - default: false
- `required`: Whether input is required - default: false
- `error`: Optional error message to display
- `oninput`: Callback when value changes

#### TextArea
Multi-line text input component.

```rust
TextArea {
    label: "描述".to_string(),
    value: description(),
    rows: 4.0,
    oninput: move |value| description.set(value)
}
```

**Props:**
- `label`: TextArea label
- `value`: Current value
- `placeholder`: Placeholder text
- `rows`: Number of rows - default: 4.0
- `readonly`: Whether readonly - default: false
- `required`: Whether required - default: false
- `error`: Optional error message
- `oninput`: Callback when value changes

#### Select
Dropdown select component.

```rust
Select {
    label: "角色".to_string(),
    value: role(),
    options: vec![
        ("admin".to_string(), "管理员".to_string()),
        ("user".to_string(), "用户".to_string()),
    ],
    onchange: move |value| role.set(value)
}
```

**Props:**
- `label`: Select label
- `value`: Current selected value
- `options`: Vec of (value, label) tuples
- `readonly`: Whether readonly - default: false
- `required`: Whether required - default: false
- `error`: Optional error message
- `onchange`: Callback when selection changes

#### Checkbox
Checkbox component with label.

```rust
Checkbox {
    label: "同意条款".to_string(),
    checked: agreed(),
    onchange: move |checked| agreed.set(checked)
}
```

**Props:**
- `label`: Checkbox label
- `checked`: Current checked state
- `readonly`: Whether readonly - default: false
- `error`: Optional error message
- `onchange`: Callback when checked state changes

#### Button
Button component with variants and loading state.

```rust
Button {
    variant: "primary".to_string(),
    size: "medium".to_string(),
    loading: is_loading(),
    onclick: move |_| handle_submit(),
    "提交"
}
```

**Props:**
- `children`: Button content
- `variant`: Button style (primary, secondary, danger, success) - default: "primary"
- `size`: Button size (small, medium, large) - default: "medium"
- `disabled`: Whether disabled - default: false
- `loading`: Whether in loading state - default: false
- `button_type`: HTML button type (button, submit, reset) - default: "button"
- `onclick`: Click handler

### Feedback Components (`feedback.rs`)

Components for displaying messages, errors, and notifications.

#### ErrorMessage
Display validation or error messages.

```rust
ErrorMessage { message: "用户名不能为空".to_string() }
```

#### SuccessMessage
Display success feedback.

```rust
SuccessMessage { message: "保存成功".to_string() }
```

#### WarningMessage
Display warning messages.

```rust
WarningMessage { message: "密码强度较弱".to_string() }
```

#### InfoMessage
Display informational messages.

```rust
InfoMessage { message: "配置已更新".to_string() }
```

#### ValidationFeedback
Display multiple validation errors and warnings.

```rust
ValidationFeedback {
    errors: vec!["错误1".to_string(), "错误2".to_string()],
    warnings: vec!["警告1".to_string()]
}
```

#### Alert
Prominent alert component with optional title and dismiss button.

```rust
Alert {
    title: Some("重要通知".to_string()),
    message: "系统维护通知".to_string(),
    alert_type: "warning".to_string(),
    dismissible: true,
    ondismiss: move |_| {}
}
```

**Props:**
- `title`: Optional alert title
- `message`: Alert message
- `alert_type`: Alert style (success, error, warning, info) - default: "info"
- `dismissible`: Whether can be dismissed - default: false
- `ondismiss`: Callback when dismissed

#### Toast
Toast notification component.

```rust
Toast {
    message: "操作成功".to_string(),
    toast_type: "success".to_string(),
    visible: show_toast(),
    ondismiss: move |_| show_toast.set(false)
}
```

#### FieldError
Field-specific error display that filters errors by field name.

```rust
FieldError {
    field: "username".to_string(),
    errors: vec![
        ("username".to_string(), "用户名不能为空".to_string()),
        ("password".to_string(), "密码太短".to_string()),
    ]
}
```

### Loading Components (`loading.rs`)

Components for loading states and progress indication.

#### Spinner
Spinning loading indicator.

```rust
Spinner {
    size: "medium".to_string(),
    message: Some("加载中...".to_string())
}
```

**Props:**
- `size`: Spinner size (small, medium, large) - default: "medium"
- `message`: Optional loading message

#### LoadingOverlay
Full-screen loading overlay.

```rust
LoadingOverlay {
    visible: is_loading(),
    message: "处理中...".to_string()
}
```

#### ProgressBar
Progress bar with percentage display.

```rust
ProgressBar {
    value: progress(),
    max: 100.0,
    label: Some("上传进度".to_string()),
    variant: "primary".to_string(),
    show_percentage: true
}
```

**Props:**
- `value`: Current progress value (0-100)
- `max`: Maximum value - default: 100.0
- `show_percentage`: Whether to show percentage - default: true
- `variant`: Progress bar style (primary, success, warning, danger) - default: "primary"
- `label`: Optional label

#### Skeleton
Skeleton loading placeholder.

```rust
Skeleton {
    variant: "text".to_string(),
    lines: 3,
    width: "100%".to_string(),
    height: "1em".to_string()
}
```

**Props:**
- `variant`: Skeleton type (text, circle, rectangle) - default: "text"
- `width`: Width CSS value - default: "100%"
- `height`: Height CSS value - default: "1em"
- `lines`: Number of lines for text variant - default: 1

#### InlineLoader
Inline loading indicator for buttons or small spaces.

```rust
InlineLoader {
    visible: is_saving(),
    text: Some("保存中...".to_string())
}
```

#### ProgressSteps
Multi-step progress indicator.

```rust
ProgressSteps {
    current_step: 1,
    steps: vec![
        "选择配置".to_string(),
        "验证设置".to_string(),
        "保存配置".to_string(),
    ]
}
```

#### LoadingState
Generic loading state wrapper that handles loading, error, and success states.

```rust
LoadingState {
    loading: is_loading(),
    error: error_message(),
    data: user_data(),
    // Render content when data is loaded
    UserProfile { user: user_data().unwrap() }
}
```

## Usage Examples

See `examples.rs` for complete working examples of all components.

### Basic Form Example

```rust
use palpo_admin_ui::components::*;

#[component]
fn LoginForm() -> Element {
    let mut username = use_signal(|| String::new());
    let mut password = use_signal(|| String::new());
    let mut error = use_signal(|| None);

    rsx! {
        div {
            Input {
                label: "用户名".to_string(),
                value: username(),
                error: error(),
                oninput: move |value| username.set(value)
            }
            Input {
                label: "密码".to_string(),
                input_type: "password".to_string(),
                value: password(),
                oninput: move |value| password.set(value)
            }
            Button {
                variant: "primary".to_string(),
                onclick: move |_| {
                    // Handle login
                },
                "登录"
            }
        }
    }
}
```

## Styling

All components use CSS classes for styling. The classes follow a consistent naming convention:

- Form components: `.form-group`, `.form-input`, `.form-label`, etc.
- Feedback components: `.error-message`, `.success-message`, `.alert`, etc.
- Loading components: `.spinner`, `.progress-bar`, `.skeleton`, etc.

Implement your own CSS or use TailwindCSS to style these components according to your design system.

## Requirements Mapping

These components satisfy the following requirements:

- **Requirement 13.4**: Form components with validation and error feedback
- **Requirement 13.5**: Loading indicators and progress feedback
- **Requirement 8.2**: Error message display and validation feedback

## Testing

Unit tests for these components should verify:
- Component rendering with different props
- Event handlers are called correctly
- Validation errors are displayed properly
- Loading states are handled correctly

See the test files for examples of component testing.
