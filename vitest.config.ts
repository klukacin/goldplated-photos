import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    exclude: [
      '**/node_modules/**',
      '**/dist/**',
      // Git worktrees live under .claude/worktrees, inside the repo. Without
      // this, `npm test` in the main checkout also runs every test file in
      // every worktree — so a run reports another branch's failures as this
      // branch's, and passes or fails for reasons that are not in your tree.
      '.claude/**',
    ],
  },
});
