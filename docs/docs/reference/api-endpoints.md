# API Endpoints

Complete reference for all API endpoints.

---

## Image Serving

### GET /albums/[...path]

Serves original images from the albums directory.

**Parameters:**

- `path` - Path to image file (e.g., `2025/vacation/photo.jpg`)

**Response:**

- `200` - Image file with caching headers
- `400` - Invalid path (path traversal attempt)
- `404` - File not found

**Headers:**

```
Content-Type: image/jpeg
Cache-Control: public, max-age=31536000, immutable
```

---

## Thumbnail Generation

### GET /api/thumbnail

Generates and serves optimized thumbnails.

**Query Parameters:**

| Param | Required | Description |
|-------|----------|-------------|
| `path` | Yes | Relative path to original image |
| `size` | Yes | Size preset: `small`, `medium`, `large` |

**Example:**

```
GET /api/thumbnail?path=2025/vacation/beach.jpg&size=medium
```

**Response:**

- `200` - JPEG thumbnail
- `400` - Missing or invalid parameters
- `404` - Original image not found
- `500` - Processing error

**Size Presets:**

| Size | Max Width |
|------|-----------|
| small | 400px |
| medium | 1200px |
| large | 1920px |

---

## Metadata Extraction

### GET /api/exif

Extracts EXIF metadata from photos.

**Query Parameters:**

| Param | Required | Description |
|-------|----------|-------------|
| `path` | Yes | Relative path to image |

**Example:**

```
GET /api/exif?path=2025/vacation/beach.jpg
```

**Response:**

```json
{
  "Make": "Canon",
  "Model": "EOS R5",
  "LensModel": "RF 24-70mm F2.8",
  "FocalLength": 35,
  "FNumber": 2.8,
  "ExposureTime": 0.004,
  "ISO": 100,
  "DateTimeOriginal": "2025:06:15 14:30:00",
  "ImageWidth": 8192,
  "ImageHeight": 5464
}
```

### GET /api/video-info

Extracts video metadata via ffprobe.

**Query Parameters:**

| Param | Required | Description |
|-------|----------|-------------|
| `path` | Yes | Relative path to video |

**Response:**

```json
{
  "duration": 120.5,
  "width": 1920,
  "height": 1080,
  "codec": "h264",
  "fps": 30
}
```

---

## Authentication

### POST /api/unlock

Primary authentication endpoint for password-protected albums.

**Content-Type:** `application/x-www-form-urlencoded`

**Form Fields:**

| Field | Required | Description |
|-------|----------|-------------|
| `albumPath` | Yes | Path to the album |
| `password` | Yes | User-entered password |
| `returnUrl` | No | Redirect URL after success |

**Response:**

- `302` - Redirect to album (with cookie set)
- `302` - Redirect with `?error=wrong-password`
- `302` - Redirect with `?error=rate-limited`

**Cookie Set on Success:**

```
album-access: ["token1", "token2", ...]
```

### POST /api/check-password

Legacy JSON API for password validation.

**Content-Type:** `application/json`

**Request:**

```json
{
  "albumPath": "2025/weddings/party",
  "password": "guestsonly"
}
```

**Response (Success):**

```json
{
  "success": true
}
```

**Response (Failure):**

```json
{
  "success": false,
  "remainingAttempts": 7
}
```

**Response (Rate Limited):**

```json
{
  "success": false,
  "error": "Too many attempts. Please try again in 15 minutes.",
  "rateLimited": true
}
```

### POST /api/check-access

Checks if user has access to an album.

**Content-Type:** `application/json`

**Request:**

```json
{
  "albumPath": "2025/weddings/party",
  "password": "optional",
  "unlockedTokens": ["token1", "token2"],
  "providedToken": "optional-url-token"
}
```

**Response (Access Granted):**

```json
{
  "success": true,
  "unlockedTokens": ["token1", "token2", "token3"]
}
```

**Response (Password Required):**

```json
{
  "success": false,
  "requiresPassword": true,
  "blockingAlbum": {
    "path": "2025/weddings",
    "title": "2025 Weddings"
  }
}
```

---

## Album Download

### GET /api/download-album

Creates a ZIP archive of album photos.

**Query Parameters:**

| Param | Required | Description |
|-------|----------|-------------|
| `path` | Yes | Path to the album |

**Headers Required:**

| Header | Description |
|--------|-------------|
| `X-Album-Token` | Album access token (for protected albums) |

**Response:**

- `200` - ZIP file download
- `401` - Unauthorized (missing or invalid token)
- `404` - Album not found

**Response Headers:**

```
Content-Type: application/zip
Content-Disposition: attachment; filename="album-name.zip"
```

---

## Error Responses

All endpoints return consistent error responses:

### 400 Bad Request

```json
{
  "error": "Missing required parameter: path"
}
```

### 401 Unauthorized

```json
{
  "error": "Access denied"
}
```

### 404 Not Found

```json
{
  "error": "Album not found"
}
```

### 429 Too Many Requests

```json
{
  "error": "Too many attempts. Please try again in 15 minutes.",
  "rateLimited": true
}
```

### 500 Internal Server Error

```json
{
  "error": "Server error"
}
```

---

## Security Notes

### Path Validation

All endpoints validate paths to prevent directory traversal:

- Paths containing `..` are rejected
- Paths starting with `/` are rejected
- Only files within `src/content/albums/` are accessible

### Rate Limiting

Password endpoints are rate-limited:

- 10 attempts per IP
- 15-minute lockout after exceeding
- Rate limit cleared on successful login

### Cookie Security

Authentication cookies have security flags:

- `httpOnly` - Not accessible via JavaScript
- `secure` - HTTPS only (in production)
- `sameSite: strict` - CSRF protection
- 24-hour expiration

---

## Related

- [Architecture](architecture.md) - Technical overview
- [Password Protection](../features/password-protection.md) - Auth details
