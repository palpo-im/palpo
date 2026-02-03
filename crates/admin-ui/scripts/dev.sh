#!/bin/bash
# Development script for Palpo Admin UI

set -e

echo "Starting Palpo Admin UI development server..."

# Install Dioxus CLI if not present
if ! command -v dx &> /dev/null; then
    echo "Installing Dioxus CLI..."
    cargo install dioxus-cli
fi

# Start development server with hot reload
dx serve --hot-reload