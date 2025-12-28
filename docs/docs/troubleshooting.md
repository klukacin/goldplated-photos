# Troubleshooting

Common issues and their solutions.

---

## Thumbnail Issues

### Thumbnails Not Generating

**Symptoms:** Broken image icons, 500 errors on thumbnail requests

**Solutions:**

1. Check `.meta/thumbnails/` directory exists and is writable:
   ```bash
   mkdir -p .meta/thumbnails
   chmod 755 .meta/thumbnails
   ```

2. Verify Sharp is installed:
   ```bash
   npm ls sharp
   ```

3. Rebuild Sharp for your platform:
   ```bash
   npm rebuild sharp
   ```

4. Check server logs for specific errors

### Thumbnails Not Updating

**Symptoms:** Old thumbnails still showing after replacing photos

**Solution:** Delete cached thumbnails:
```bash
rm -rf .meta/thumbnails
```

Thumbnails regenerate automatically on next request.

### Wrong Orientation

**Symptoms:** Photos display rotated incorrectly

**Solutions:**

1. Verify the original has EXIF orientation data
2. Delete the cached thumbnail to regenerate:
   ```bash
   rm .meta/thumbnails/*/HASH.jpg
   ```
3. Ensure Sharp's `.rotate()` is called without parameters

---

## Password Protection Issues

### Password Not Working

**Symptoms:** Correct password rejected, can't access album

**Solutions:**

1. Check the `album-access` cookie in browser DevTools:
   - Open DevTools → Application → Cookies
   - Look for `album-access` cookie
   - Delete it and try again

2. Verify password in `index.md` (case-sensitive)

3. Check for rate limiting (wait 15 minutes or restart server)

### "Too Many Attempts" Error

**Symptoms:** Rate limited after failed attempts

**Solutions:**

1. Wait 15 minutes for cooldown
2. Or restart the server (rate limit is in-memory)
3. Adjust limits in `src/lib/rate-limit.ts`:
   ```typescript
   const MAX_ATTEMPTS = 10;
   const LOCKOUT_DURATION = 15 * 60 * 1000;
   ```

### Album Content Visible Without Password

**Symptoms:** Protected album shows photos without authentication

**Solutions:**

1. Verify `prerender = false` in `[...path].astro`:
   ```typescript
   export const prerender = false;
   ```

2. Clear browser cache and cookies

3. Ensure access verification runs before content fetch

### Download Album Failing on Protected Content

**Symptoms:** 401 errors when downloading protected albums

**Solution:** Ensure `X-Album-Token` header is included in the request. Check browser DevTools Network tab for the download request.

---

## Display Issues

### EXIF Not Showing

**Symptoms:** Info overlay is empty or shows "No EXIF data"

**Causes:**

- Photo lacks EXIF data (stripped during editing)
- Unsupported EXIF format

**Solutions:**

1. Check console for errors from exifr library
2. Verify EXIF exists with: `exiftool photo.jpg`
3. Re-export photo with EXIF preserved

### EXIF Not Visible in Fullscreen

**Symptoms:** EXIF overlay hidden behind browser UI in fullscreen

**Solution:** This is handled automatically. The overlay should appear inside PhotoSwipe's container. If not, check for CSS conflicts.

### Navigation Jumping

**Symptoms:** Arrow key navigation skips photos unexpectedly

**Solution:** Position-based navigation uses 80px tolerance for row detection. Adjust in PhotoGrid's `getNextIndex()` function if needed.

### Sort Not Persisting

**Symptoms:** Sort preference resets on page refresh

**Solution:** Check localStorage for `photoGallery_sortOption` key:
```javascript
localStorage.getItem('photoGallery_sortOption')
```

---

## Page Issues

### /home Page Not Rendering

**Symptoms:** Home page shows error or blank

**Solution:** Ensure prerender is set correctly:
```typescript
// src/pages/home.astro
export const prerender = true;
```

### Album Not Appearing

**Symptoms:** Album exists but doesn't show in listings

**Solutions:**

1. Verify `index.md` exists in the album folder
2. Check for YAML syntax errors in frontmatter
3. Ensure `hidden: false` (or remove the field)
4. Restart the dev server

### Broken Thumbnails / "Album Not Found"

**Symptoms:** Thumbnails fail to load, console shows path errors

**Cause:** Dots in folder names after numbers

**Example:**
- `16.album-name` → Astro normalizes to `16album-name`
- File system lookup fails

**Solution:** Use dashes instead of dots:
- `16-album-name`

---

## Server Issues

### Port Already in Use

**Symptoms:** "Port 4321 is already in use" error

**Solutions:**

1. Dev server tries 4321, 4322, 4323 automatically
2. Find and kill the process:
   ```bash
   lsof -i :4321
   kill -9 PID
   ```
3. Or specify a different port:
   ```bash
   PORT=4000 npm run dev
   ```

### Server Won't Start

**Symptoms:** Immediate crash on startup

**Solutions:**

1. Check Node.js version:
   ```bash
   node --version  # Should be 18+
   ```

2. Verify build completed:
   ```bash
   ls dist/server/
   ```

3. Check for missing dependencies:
   ```bash
   npm install
   ```

---

## Styling Issues

### Fonts Not Loading

**Symptoms:** Fallback fonts displayed instead of Google Fonts

**Solutions:**

1. Check Network tab for Google Fonts requests
2. Verify preconnect hints in Layout.astro:
   ```html
   <link rel="preconnect" href="https://fonts.googleapis.com">
   <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
   ```
3. Check for ad blockers blocking fonts

### Focus Styles Not Visible

**Symptoms:** No visible focus indicator on keyboard navigation

**Solutions:**

1. Ensure `:focus-visible` isn't overridden
2. Check for `outline: none` on elements
3. Verify `--color-focus` CSS variable exists

### Slider Not Auto-playing

**Symptoms:** Hero slider doesn't advance automatically

**Cause:** User has `prefers-reduced-motion: reduce` enabled

**Solution:** This is intentional behavior respecting accessibility preferences. Slider can still be navigated manually.

---

## Development Issues

### Content Collection Errors

**Symptoms:** Build fails with content collection validation errors

**Solutions:**

1. Check frontmatter YAML syntax in `index.md` files
2. Verify required fields are present (at minimum: `title`)
3. Remove invalid or extra fields

### TypeScript Errors

**Symptoms:** Type errors during build

**Solutions:**

1. Run type check:
   ```bash
   npx astro check
   ```
2. Update Astro and dependencies:
   ```bash
   npm update
   ```

---

## Getting Help

If you're still stuck:

1. Check the [GitHub Issues](https://github.com/klukacin/goldplated-photos/issues)
2. Search for similar problems
3. Open a new issue with:
   - Node.js version
   - Browser and version
   - Steps to reproduce
   - Error messages from console

---

## Related

- [Installation](getting-started/installation.md) - Setup guide
- [Configuration](getting-started/configuration.md) - All options
- [Architecture](reference/architecture.md) - Technical details
