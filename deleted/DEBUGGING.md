# Debugging Guide

## Testing Password Protection

### Password for Maria's Birthday Album
- **URL**: http://localhost:4322/2025/Birthdays/Marias-Birthday
- **Password**: `maria2025`

### How to Test

1. Navigate to: http://localhost:4322/2025/Birthdays/Marias-Birthday
2. You should see a password protection screen
3. Enter password: `maria2025`
4. Click "Unlock Album"
5. The page should reload and show the photos

### Common Issues

#### Password Form Not Submitting
- **Check browser console** (F12 → Console tab) for JavaScript errors
- Look for errors like:
  - `Failed to fetch`
  - `Unexpected token`
  - Module import errors

#### API Route Not Responding
Test the API directly:
```bash
curl -X POST http://localhost:4322/api/check-password \
  -H "Content-Type: application/json" \
  -d '{"albumPath":"2025/Birthdays/Marias-Birthday","password":"maria2025"}'
```

Expected response: `{"success":true}`

#### Password Accepted But Photos Not Showing
- Check browser console for errors
- Check if sessionStorage is working:
  1. Open browser DevTools (F12)
  2. Go to Application tab → Session Storage
  3. Look for key: `album-password-2025/Birthdays/Marias-Birthday`

#### Photos Not Loading (General)
1. **Check if photos exist**:
   ```bash
   ls -la src/content/albums/2025/Birthdays/Marias-Birthday/
   ```

2. **Check photo file extensions**:
   - Supported: `.jpg`, `.jpeg`, `.png`, `.gif`, `.webp`, `.heic`, `.heif`
   - Files must be lowercase extension

3. **Check browser Network tab**:
   - Look for failed requests to `/albums/.../*.jpg`
   - 404 errors mean file path is wrong

## Checking for Errors

### Browser Console Errors
1. Open browser (Chrome/Firefox/Safari)
2. Press F12 or right-click → Inspect
3. Go to Console tab
4. Look for red error messages

Common errors and fixes:

**Error**: `PhotoSwipeLightbox is not defined`
- **Fix**: The PhotoSwipe module didn't load properly
- Check if `node_modules/photoswipe` exists

**Error**: `Failed to fetch /api/check-password`
- **Fix**: API route not accessible
- Make sure dev server is running
- Check that API routes have `export const prerender = false`

**Error**: `Cannot read property 'classList' of null`
- **Fix**: DOM element not found
- Check that the HTML structure matches what JavaScript expects

### Server Console Errors
Check the terminal where you ran `npm run dev` for errors like:
- TypeScript errors
- Import errors
- File not found errors

### Network Tab Debugging
1. Open DevTools → Network tab
2. Reload the page
3. Look for:
   - Red items (failed requests)
   - Status codes (404 = not found, 500 = server error)
   - Request/Response details

## Manual Testing Steps

### 1. Test Homepage
```
URL: http://localhost:4322/
Expected: Shows "2025" album
```

### 2. Test Album Navigation
```
URL: http://localhost:4322/2025/Birthdays
Expected: Shows "Johns-Birthday" and "Marias-Birthday" albums
```

### 3. Test Password Protection
```
URL: http://localhost:4322/2025/Birthdays/Marias-Birthday
Expected: Shows password form
Action: Enter "maria2025"
Expected: Shows 10 photos after reload
```

### 4. Test Photo Display
```
URL: http://localhost:4322/2025/Birthdays/Marias-Birthday (after unlocking)
Expected: Grid of photos
Action: Click a photo
Expected: Opens lightbox viewer
```

### 5. Test EXIF Info
```
Action: Click "Info" button under a photo
Expected: Modal shows camera info, settings, etc.
Note: Some photos may not have EXIF data
```

### 6. Test Download
```
Action: Click "Download" under a photo
Expected: Downloads the photo
Action: Click "Download Album" button
Expected: Downloads ZIP file with all photos
```

## Current Setup

- Dev server: http://localhost:4322/
- Photos added to: `src/content/albums/2025/Birthdays/Marias-Birthday/`
- Photo count: 10 JPG files
- Password protection: Enabled with password "maria2025"

## What To Report

When reporting issues, please include:

1. **What you did**: "I clicked X, then Y"
2. **What you expected**: "I expected to see Z"
3. **What actually happened**: "Instead I saw error ABC"
4. **Browser console errors**: Copy/paste any red errors
5. **Screenshots**: If visual issue

## Quick Fixes

### Clear Session Storage
If password protection is stuck:
```javascript
// Open browser console and run:
sessionStorage.clear()
// Then refresh the page
```

### Force Refresh
Clear cache and reload:
- **Chrome/Firefox**: Ctrl+Shift+R (Cmd+Shift+R on Mac)
- **Safari**: Cmd+Option+E, then Cmd+R

### Restart Dev Server
If things stop working:
```bash
# Stop server (Ctrl+C in terminal)
# Then restart:
npm run dev
```
