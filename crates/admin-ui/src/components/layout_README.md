# Layout Components

This module provides the core layout components for the Palpo admin interface.

## Components

### AdminLayout

The main layout wrapper that provides the overall structure for admin pages.

**Features:**
- Authentication protection with automatic redirect
- Responsive design with mobile menu support
- Sidebar navigation
- Header with breadcrumbs
- Content area with proper scrolling

**Usage:**
```rust
use crate::components::layout::AdminLayout;

#[component]
fn AdminLayout() -> Element {
    rsx! {
        AdminLayout {}
    }
}
```

### Sidebar

Responsive navigation sidebar with collapsible mobile menu.

**Features:**
- Responsive design (hidden on mobile, visible on desktop)
- Mobile overlay with backdrop
- Active route highlighting
- User profile section with logout
- Smooth transitions and animations

**Props:**
- `show_mobile: bool` - Controls mobile menu visibility
- `on_close: EventHandler<()>` - Callback when mobile menu should close

**Usage:**
```rust
use crate::components::layout::Sidebar;

let mut show_mobile = use_signal(|| false);

rsx! {
    Sidebar {
        show_mobile: show_mobile(),
        on_close: move |_| show_mobile.set(false)
    }
}
```

### Header

Page header with breadcrumb navigation and user actions.

**Features:**
- Mobile menu toggle button
- Page title display
- Breadcrumb navigation
- Session time remaining indicator
- Logout button
- Responsive design

**Props:**
- `on_menu_toggle: EventHandler<()>` - Callback for mobile menu toggle

**Usage:**
```rust
use crate::components::layout::Header;

rsx! {
    Header {
        on_menu_toggle: move |_| {
            // Toggle mobile menu
        }
    }
}
```

### Breadcrumb

Navigation breadcrumb component showing the current page hierarchy.

**Features:**
- Automatic breadcrumb generation based on route
- Clickable parent items
- Current page highlighted
- Responsive text sizing

**Props:**
- `items: Vec<BreadcrumbItem>` - List of breadcrumb items

**Usage:**
```rust
use crate::components::layout::{Breadcrumb, BreadcrumbItem};

let breadcrumbs = vec![
    BreadcrumbItem {
        label: "首页".to_string(),
        route: Some("/admin".to_string()),
    },
    BreadcrumbItem {
        label: "配置管理".to_string(),
        route: None, // Current page, not clickable
    },
];

rsx! {
    Breadcrumb { items: breadcrumbs }
}
```

## Data Structures

### NavItem

Navigation item definition for sidebar menu.

```rust
pub struct NavItem {
    pub id: &'static str,      // Unique identifier
    pub label: &'static str,   // Display label
    pub icon: &'static str,    // Icon (emoji or icon class)
    pub route: String,         // Route path
}
```

### BreadcrumbItem

Breadcrumb item definition.

```rust
pub struct BreadcrumbItem {
    pub label: String,           // Display label
    pub route: Option<String>,   // Route path (None for current page)
}
```

## Responsive Behavior

### Desktop (lg and above)
- Sidebar always visible (w-64)
- No mobile menu button
- Full breadcrumb navigation
- Extended session info

### Mobile (below lg)
- Sidebar hidden by default
- Mobile menu button visible
- Sidebar slides in from left when opened
- Backdrop overlay when menu open
- Compact header layout
- Abbreviated text labels

## Styling

All components use TailwindCSS utility classes for styling:
- **Colors**: Blue for primary actions, gray for neutral elements
- **Spacing**: Consistent padding and margins
- **Shadows**: Subtle shadows for depth
- **Transitions**: Smooth animations for state changes
- **Typography**: Clear hierarchy with appropriate font sizes

## Accessibility

- Semantic HTML elements (nav, header, aside, main)
- ARIA labels for navigation
- Keyboard navigation support
- Focus states for interactive elements
- Screen reader friendly structure

## Integration with Router

The layout components integrate with `dioxus-router`:
- Uses `use_route()` to detect current page
- Uses `Link` component for navigation
- Automatic route-based breadcrumb generation
- Active state highlighting based on route matching

## Authentication Integration

The layout components integrate with the authentication system:
- Checks authentication status via `use_auth()` hook
- Redirects to login if not authenticated
- Displays user information in sidebar
- Shows session time remaining
- Provides logout functionality
