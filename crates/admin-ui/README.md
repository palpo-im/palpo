# Palpo Admin UI

Modern web administration interface for Palpo Matrix server, built with Dioxus and compiled to WebAssembly.

## Features

- 🚀 Modern Rust + WebAssembly frontend
- 🎨 Responsive design with TailwindCSS
- 🔧 Server configuration management
- 👥 User and room administration
- 🌐 Federation management
- 📊 Media and storage management
- 📝 Audit logging and monitoring

## Development

### Prerequisites

- Rust (latest stable)
- Dioxus CLI: `cargo install dioxus-cli`
- Node.js (for TailwindCSS)

### Getting Started

1. Start development server:
   ```bash
   ./scripts/dev.sh
   ```

2. Build for production:
   ```bash
   ./scripts/build.sh
   ```

### Project Structure

```
src/
├── app.rs          # Main application component
├── components/     # Reusable UI components
├── pages/          # Page components
├── services/       # API services
├── hooks/          # Custom hooks
└── utils/          # Utility functions

assets/
└── tailwind.css    # Styles

scripts/
├── dev.sh          # Development server
└── build.sh        # Production build
```

## Architecture

The admin UI communicates with the Palpo server through RESTful APIs, providing a modern web interface for all administrative tasks.

## License

MIT