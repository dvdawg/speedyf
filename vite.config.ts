import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';

// Tauri expects a fixed dev port and no auto-clearing of its own logs.
export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/bench/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    target: 'es2022',
    outDir: 'dist',
    sourcemap: false,
    chunkSizeWarningLimit: 900,
  },
  test: {
    // Node by default: almost every test here is over pure logic, and a DOM
    // for those would only be slower. Component tests opt in per file with a
    // `@vitest-environment happy-dom` docblock, so the DOM cost lands only on
    // the handful of files that need one.
    environment: 'node',
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
  },
});
