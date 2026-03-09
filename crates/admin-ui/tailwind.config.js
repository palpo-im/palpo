/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./src/**/*.{rs,html,css}",
    "./assets/**/*.html",
  ],
  theme: {
    extend: {
      colors: {
        'palpo-primary': '#6366f1',
        'palpo-secondary': '#8b5cf6',
        'palpo-accent': '#06b6d4',
        'palpo-success': '#10b981',
        'palpo-warning': '#f59e0b',
        'palpo-error': '#ef4444',
      }
    },
  },
  plugins: [
    // Note: Using CDN version of Tailwind, plugins need to be added via CDN or npm
    // require('@tailwindcss/forms'),
    // require('@tailwindcss/typography'),
  ],
}