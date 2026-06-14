/** @type {import('tailwindcss').Config} */

// Ported from Sweet Whisper: make a CSS-variable theme color usable with Tailwind's
// `/opacity` modifier while keeping its baked-in alpha when no modifier is given.
const withAlpha = (cssVar) => ({ opacityValue }) =>
  opacityValue === undefined
    ? `var(${cssVar})`
    : `color-mix(in srgb, var(${cssVar}) calc(${opacityValue} * 100%), transparent)`;

export default {
  content: ['./src/**/*.{html,js,svelte,ts}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        'sw-bg': {
          DEFAULT: withAlpha('--sw-bg-primary'),
          secondary: withAlpha('--sw-bg-secondary'),
          card: withAlpha('--sw-bg-card'),
          hover: withAlpha('--sw-bg-hover'),
        },
        'sw-border': {
          DEFAULT: withAlpha('--sw-border'),
          focus: withAlpha('--sw-border-focus'),
        },
        'sw-text': {
          DEFAULT: withAlpha('--sw-text-primary'),
          secondary: withAlpha('--sw-text-secondary'),
          muted: withAlpha('--sw-text-muted'),
          dimmed: withAlpha('--sw-text-dimmed'),
        },
        'sw-accent': {
          DEFAULT: withAlpha('--sw-accent'),
          hover: withAlpha('--sw-accent-hover'),
          glow: withAlpha('--sw-accent-glow'),
        },
      },
      backgroundImage: {
        'gradient-radial': 'radial-gradient(var(--tw-gradient-stops))',
      },
      backdropBlur: { xs: '2px' },
      boxShadow: {
        glow: '0 0 20px rgba(59, 130, 246, 0.15)',
        'glow-sm': '0 0 10px rgba(59, 130, 246, 0.1)',
      },
      animation: {
        'pulse-slow': 'pulse 3s cubic-bezier(0.4, 0, 0.6, 1) infinite',
      },
      spacing: {
        'sw-1': 'var(--sw-space-1)',
        'sw-2': 'var(--sw-space-2)',
        'sw-3': 'var(--sw-space-3)',
        'sw-4': 'var(--sw-space-4)',
        'sw-6': 'var(--sw-space-6)',
        'sw-8': 'var(--sw-space-8)',
      },
      fontSize: {
        'sw-xs': 'var(--sw-text-xs)',
        'sw-sm': 'var(--sw-text-sm)',
        'sw-base': 'var(--sw-text-base)',
      },
      width: { 'sw-sidebar': 'var(--sw-sidebar-width)' },
      height: { 'sw-titlebar': 'var(--sw-titlebar-height)' },
      borderRadius: {
        'sw-sm': 'var(--sw-radius-sm)',
        'sw-md': 'var(--sw-radius-md)',
        'sw-lg': 'var(--sw-radius-lg)',
      },
    },
  },
  plugins: [],
};
