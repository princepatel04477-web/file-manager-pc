/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        ink: '#202124',
        muted: '#6d7278',
        line: '#e7e9ec',
        surface: '#ffffff',
        canvas: '#f6f7f8',
        brand: '#176b58',
        'brand-soft': '#e2f3ed',
      },
      fontFamily: { sans: ['Inter', 'Segoe UI', 'sans-serif'] },
      boxShadow: { card: '0 1px 2px rgba(22, 31, 40, .04), 0 6px 22px rgba(22, 31, 40, .035)' },
    },
  },
  plugins: [],
};
