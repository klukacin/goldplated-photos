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
      strictPort: true,
      watch: {
        // Git worktrees live under .claude/worktrees, *inside* the repo, and
        // desktop/target holds Rust build artifacts — hundreds of thousands of
        // files between them. Watched, they exhaust the process's file
        // descriptors and the dev server dies mid-session with a wall of
        // `EMFILE: too many open files`, which reads like a broken page rather
        // than a watcher that ran out of room. Nothing under either path is
        // served, so nothing is lost by not watching them.
        ignored: ['**/.claude/**', '**/desktop/target/**', '**/.meta/**']
      }
    },
    ssr: {
      noExternal: ['exifr', 'photoswipe']
    }
  }
});
