# Password Protection

Secure server-side access control for private albums.

---

## Overview

Goldplated Photos uses **Server-Side Rendering (SSR)** for password protection. Protected content is never sent to the browser until access is verified.

!!! success "Security Model"
    - Image URLs are NOT exposed in page source until authenticated
    - Passwords validated server-side with timing-safe comparison
    - Rate limiting prevents brute force attacks
    - HttpOnly cookies prevent XSS token theft

---

## Adding Password Protection

Add a `password` field to your album's `index.md`:

```yaml
---
title: "Private Wedding"
password: "guestsonly2025"
---
```

---

## How It Works

### 1. User Visits Protected Album

The album page uses SSR (`prerender = false`). Server checks for valid access token in the `album-access` cookie.

### 2. No Access Token

Only the password form is rendered - no image URLs appear in the page source.

### 3. User Enters Password

Form POSTs to `/api/unlock` endpoint.

### 4. Server Validates

- Timing-safe password comparison
- Rate limit check (10 attempts / 15 minutes)
- On success: Sets HttpOnly cookie with album token

### 5. Redirect to Album

User is redirected back, now with valid cookie. Full album content renders.

---

## Cookie Security

The `album-access` cookie has these security flags:

| Flag | Value | Purpose |
|------|-------|---------|
| `httpOnly` | `true` | Prevents JavaScript access (XSS protection) |
| `secure` | `true` (prod) | HTTPS only in production |
| `sameSite` | `strict` | Prevents CSRF attacks |
| `maxAge` | `86400` | 24-hour expiration |

---

## Rate Limiting

Brute force protection is built-in:

- **10 attempts** before lockout
- **15 minute** cooldown period
- Per-IP tracking
- Cleared on successful login

Rate limit state is in-memory (resets on server restart).

---

## Access Inheritance

Unlocking a parent album grants access to child albums:

```
/photos/2025/weddings/         # Password: "secret"
/photos/2025/weddings/ceremony # No password - inherits access
/photos/2025/weddings/party    # No password - inherits access
```

Child albums without their own password are automatically unlocked when the parent is accessed.

---

## Shareable Links

Albums can be shared via URL tokens:

```yaml
---
title: "Shared Album"
password: "secret"
allowAnonymous: true
---
```

Share URL: `https://example.com/photos/2025/album?token=abc123`

The token is validated against the album's stored token, granting access without the password.

---

## API Endpoints

### POST /api/unlock

Main authentication endpoint.

**Request (form data):**

- `albumPath` - Path to the album
- `password` - User-entered password
- `returnUrl` - Redirect destination

**Response:** Redirect with cookie set

### POST /api/check-password

Legacy JSON API for password validation.

**Request:**
```json
{
  "albumPath": "2025/weddings/party",
  "password": "guestsonly"
}
```

**Response:**
```json
{
  "success": true
}
```

---

## Troubleshooting

### "Too many attempts"

Wait 15 minutes for rate limit reset, or restart the server.

### Password Not Working

1. Check the `album-access` cookie in DevTools
2. Clear the cookie and try again
3. Verify password in `index.md` (case-sensitive)

### Album Content Visible Without Password

Verify `prerender = false` is set in `[...path].astro`. The page must use SSR, not static generation.

---

## Security Considerations

!!! warning "Password Storage"
    Passwords are stored as plaintext in `index.md` files. This is intentional for simplicity but means:

    - Don't use passwords you use elsewhere
    - Consider this "casual" protection, not enterprise-grade
    - The security comes from SSR preventing content exposure

---

## Related

- [Architecture](../reference/architecture.md) - Technical details
- [API Endpoints](../reference/api-endpoints.md) - Full API reference
