# Deployment

Deploy your gallery to production.

---

## Recommended Workflow

Goldplated Photos is designed for a **local development → deploy to server** workflow:

```
┌─────────────────────────────────────────────────────────┐
│  LOCAL DEVELOPMENT                                       │
│                                                          │
│  ┌─────────────┐         ┌─────────────┐                │
│  │ Admin Panel │ ──────► │ Dev Server  │                │
│  │ :4444       │  edits  │ :4321       │                │
│  └─────────────┘         └─────────────┘                │
│                                                          │
│  • Edit albums, upload photos via Admin Panel            │
│  • Preview changes in Dev Server                         │
│  • All changes saved to src/content/albums/              │
└─────────────────────────────────────────────────────────┘
                          │
                          │ npm run deploy
                          ▼
┌─────────────────────────────────────────────────────────┐
│  PRODUCTION SERVER                                       │
│                                                          │
│  • Builds site locally (npm run build)                  │
│  • Syncs content via rsync (--delete)                   │
│  • Restarts Node.js app via PM2                         │
└─────────────────────────────────────────────────────────┘
```

!!! info "Key Points"
    - **Admin Panel is LOCAL ONLY** - Never deployed to production
    - **Local content is source of truth** - rsync uses `--delete` flag
    - **Single command deployment** - `npm run deploy` handles everything

---

## Quick Deploy

If you have SSH access configured and PM2 on your server:

```bash
npm run deploy
```

This single command:

1. Builds the production site locally
2. Fixes paths for production server
3. Syncs all files to remote via rsync
4. Sets up symlinks on server
5. Installs production dependencies
6. Restarts PM2

---

## Deploy Script Details

The `npm run deploy` command runs `scripts/deploy.sh` which performs:

### Step 1: Build

```bash
npm run build
```

Creates `dist/` folder with optimized site.

### Step 2: Fix Paths

Runs `fix-server-paths.mjs` to update paths for production.

### Step 3: Sync Files

Uses rsync to transfer:

- `dist/client/` - Static assets
- `dist/server/` - Server code
- `src/content/albums/` - All albums (**with --delete**)
- `public/` - Hero images, card images, etc.
- `package.json` - Dependencies
- `ecosystem.config.cjs` - PM2 configuration

### Step 4: Configure Remote

Creates symlinks on server for Apache to serve images directly.

### Step 5: Install Dependencies

```bash
npm install --production
```

### Step 6: Restart PM2

```bash
pm2 restart ecosystem.config.cjs
```

---

## Deploy Configuration

Edit `scripts/deploy.sh` to configure your server:

```bash
REMOTE_USER="your-username"
REMOTE_HOST="your-server.com"
REMOTE_ROOT="/path/to/public_html"
```

### Prerequisites

- SSH key access to server (no password prompt)
- PM2 installed on server
- Node.js 18+ on server
- `ecosystem.config.cjs` file in project root

---

## Manual Deployment

If you prefer manual deployment or don't have rsync:

### Build for Production

```bash
npm run build
```

This creates a `dist/` folder with:

- `dist/client/` - Static assets
- `dist/server/` - Node.js server code

---

## Server Requirements

Since Goldplated Photos uses SSR for password protection, you need a Node.js server:

- Node.js 18+
- Persistent file system (for thumbnail cache)
- HTTPS recommended (for secure cookies)

---

## Running in Production

### Basic Start

```bash
node dist/server/entry.mjs
```

### With Environment Variables

```bash
PORT=3000 HOST=0.0.0.0 node dist/server/entry.mjs
```

### With PM2 (Process Manager)

```bash
# Install PM2
npm install -g pm2

# Start with PM2
pm2 start dist/server/entry.mjs --name goldplated-photos

# Auto-restart on reboot
pm2 startup
pm2 save
```

---

## Deployment Platforms

### VPS / Dedicated Server

1. Clone repository to server
2. Install dependencies: `npm install`
3. Build: `npm run build`
4. Start with PM2 or systemd
5. Configure reverse proxy (nginx/Caddy)

### Docker

```dockerfile
FROM node:20-alpine

WORKDIR /app
COPY package*.json ./
RUN npm ci --only=production

COPY . .
RUN npm run build

EXPOSE 3000
ENV HOST=0.0.0.0
ENV PORT=3000

CMD ["node", "dist/server/entry.mjs"]
```

### Vercel / Netlify

These platforms have limited SSR support. For full functionality:

1. Use the Vercel/Netlify Astro adapter instead of Node
2. Note: Some features may not work (in-memory rate limiting)

---

## Reverse Proxy

### Nginx

```nginx
server {
    listen 80;
    server_name photos.example.com;
    return 301 https://$server_name$request_uri;
}

server {
    listen 443 ssl http2;
    server_name photos.example.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://localhost:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_cache_bypass $http_upgrade;
    }

    # Cache static assets
    location /_astro/ {
        proxy_pass http://localhost:3000;
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}
```

### Caddy

```caddyfile
photos.example.com {
    reverse_proxy localhost:3000
}
```

---

## Content Management Workflow

### Adding New Albums

1. Create album folder in `src/content/albums/`
2. Add `index.md` and photos
3. Rebuild: `npm run build`
4. Restart server

### Updating Existing Albums

For photo changes only (no code changes):

1. Add/remove photos in album folder
2. Delete cached thumbnails if needed: `rm -rf .meta/thumbnails`
3. Rebuild: `npm run build`
4. Restart server

### Quick Preview

```bash
npm run preview
```

Preview runs the production build locally.

---

## SSL Certificates

### Let's Encrypt (Certbot)

```bash
# Install certbot
sudo apt install certbot python3-certbot-nginx

# Get certificate
sudo certbot --nginx -d photos.example.com

# Auto-renewal is configured automatically
```

### Cloudflare

1. Enable Cloudflare proxy for your domain
2. Set SSL mode to "Full (strict)"
3. Certificates managed automatically

---

## Monitoring

### PM2 Monitoring

```bash
# View logs
pm2 logs goldplated-photos

# Monitor resources
pm2 monit

# View status
pm2 status
```

### Health Check Endpoint

Add a simple health check in your nginx config:

```nginx
location /health {
    proxy_pass http://localhost:3000/;
    access_log off;
}
```

---

## Backup Strategy

### What to Backup

1. `src/content/albums/` - All your photos and metadata
2. `.meta/thumbnails/` - Optional (can be regenerated)
3. Database/state - Not applicable (all file-based)

### Backup Script

```bash
#!/bin/bash
BACKUP_DIR="/backups/goldplated-$(date +%Y%m%d)"
mkdir -p "$BACKUP_DIR"

# Backup albums
tar -czf "$BACKUP_DIR/albums.tar.gz" src/content/albums/

# Backup configuration
cp astro.config.mjs "$BACKUP_DIR/"
cp package.json "$BACKUP_DIR/"
```

---

## Performance Tips

### Thumbnail Pre-warming

Visit all albums once after deployment to generate thumbnails:

```bash
# Simple approach - visit each album page
curl http://localhost:3000/photos/2025/album1
curl http://localhost:3000/photos/2025/album2
```

### CDN for Static Assets

Configure your CDN to cache `/_astro/` paths with long TTLs.

### Image Optimization

Before uploading, optimize source images:

```bash
# Using ImageMagick
mogrify -strip -quality 85 -resize "6000x6000>" *.jpg
```

---

## Troubleshooting

### Server Won't Start

1. Check Node.js version: `node --version`
2. Verify build completed: `ls dist/server/`
3. Check port availability: `lsof -i :3000`

### Thumbnails Not Generating

1. Check `.meta/` directory permissions
2. Verify Sharp is installed correctly
3. Check server logs for errors

### Password Protection Not Working

1. Verify SSR is enabled in `astro.config.mjs`
2. Check `album-access` cookie in browser
3. Ensure HTTPS for secure cookies in production

---

## Related

- [Admin Panel](admin-panel.md) - Local content management
- [Configuration](../getting-started/configuration.md) - All options
- [Architecture](../reference/architecture.md) - Technical details
