import type { Config } from 'tailwindcss';

const config: Config = {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // UI chrome tokens — sourced from src/styles/theme-dark.css.
        // Hard-coded hex values are forbidden in components; reference
        // these aliases or `var(--token)` directly.
        bg: {
          primary: 'var(--bg-primary)',
          secondary: 'var(--bg-secondary)',
          tertiary: 'var(--bg-tertiary)',
          active: 'var(--bg-active)',
          overlay: 'var(--bg-overlay)',
        },
        border: {
          subtle: 'var(--border-subtle)',
          strong: 'var(--border-strong)',
          focus: 'var(--border-focus)',
        },
        text: {
          primary: 'var(--text-primary)',
          secondary: 'var(--text-secondary)',
          disabled: 'var(--text-disabled)',
          'on-accent': 'var(--text-on-accent)',
          link: 'var(--text-link)',
          'link-hover': 'var(--text-link-hover)',
        },
        accent: {
          DEFAULT: 'var(--accent)',
          hover: 'var(--accent-hover)',
          active: 'var(--accent-active)',
        },
        pane: {
          border: 'var(--pane-border)',
          'border-active': 'var(--pane-border-active)',
        },
        status: {
          ok: 'var(--status-ok)',
          info: 'var(--status-info)',
          warn: 'var(--status-warn)',
          error: 'var(--status-error)',
        },
      },
    },
  },
  plugins: [],
};

export default config;
