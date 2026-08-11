// @ts-check
import { defineConfig } from 'astro/config';
import node from '@astrojs/node';

// https://astro.build/config
export default defineConfig({
  output: 'server',
  adapter: node({
    mode: 'standalone'
  }),
  server: {
    port: 4321
  },
  security: {
    checkOrigin: false  // Disable CSRF origin check (safe: we have rate limiting + sameSite cookies)
  },
  vite: {
    server: {
      strictPort: true
    },
    ssr: {
      noExternal: ['exifr', 'photoswipe']
    }
  }
});
