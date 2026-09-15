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
    // Astro's CSRF check compares the Origin header to the request URL, and
    // behind the reverse proxy the app sees http://127.0.0.1:4321 — every
    // form post would fail. What actually stands in for it: the access cookie
    // is sameSite=strict (never sent cross-site), API routes that need a
    // token read a custom header (blocked by CORS without a preflight we
    // never grant), and the password limiter is keyed per IP *and* album so
    // a cross-site form spamming /api/unlock cannot lock a visitor out.
    checkOrigin: false
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
