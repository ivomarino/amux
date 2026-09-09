import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/postcss';
import { fileURLToPath } from 'node:url';
export default defineConfig({
  base: '/business/',
  plugins: [react()],
  resolve: { alias: { '@': fileURLToPath(new URL('.', import.meta.url)) } },
  define: { 'import.meta.env.VITE_AMUX_DIRECT': 'true' },
  css: { postcss: { plugins: [tailwindcss()] } },
  build: {
    outDir: '../crates/amux-dashboard/static/business',
    emptyOutDir: true,
  },
});
