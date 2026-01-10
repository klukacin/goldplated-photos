# Installation

Complete guide to setting up Goldplated Photos on your system.

---

## Requirements

- **Node.js** 18+ (LTS recommended)
- **npm** 9+ or **pnpm**
- **ffprobe** (optional, for video metadata)

---

## Platform Compatibility

Goldplated Photos works on **macOS**, **Linux**, and **Windows**.

| Feature | macOS | Linux | Windows | Windows + WSL |
|---------|:-----:|:-----:|:-------:|:-------------:|
| Dev server (`npm run dev`) | ✅ | ✅ | ✅ | ✅ |
| Admin panel (`npm run admin`) | ✅ | ✅ | ✅ | ✅ |
| Build (`npm run build`) | ✅ | ✅ | ✅ | ✅ |
| Background scripts (`dev:bg`, `admin:bg`) | ✅ | ✅ | ❌ | ✅ |
| Deploy script (`npm run deploy`) | ✅ | ✅ | ❌ | ✅ |

---

## Platform Setup

=== "Windows"

    ### Option 1: WSL2 (Recommended)
    
    WSL2 provides a native Linux environment inside Windows with **full compatibility**.
    
    **Install WSL2:**
    ```powershell
    # Run in PowerShell as Administrator
    wsl --install
    ```
    
    **After restart, set up Ubuntu:**
    ```bash
    # Update packages
    sudo apt update && sudo apt upgrade -y
    
    # Install Node.js via nvm (recommended)
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
    source ~/.bashrc
    nvm install 20
    nvm use 20
    
    # Install ffmpeg (optional, for video metadata)
    sudo apt install ffmpeg -y
    
    # Clone and run project
    git clone https://github.com/klukacin/goldplated-photos.git
    cd goldplated-photos
    npm install
    npm run dev
    ```
    
    !!! tip "Access from Windows"
        Files are at `\\wsl$\Ubuntu\home\<username>\goldplated-photos`
    
    ### Option 2: Native Windows (Limited)
    
    For development only - **no bash scripts or deployment**.
    
    **Prerequisites:**
    
    1. **Node.js 20+**: Download from [nodejs.org](https://nodejs.org/)
    2. **Git**: Download from [git-scm.com](https://git-scm.com/)
    3. **FFmpeg** (optional): Download from [ffmpeg.org](https://ffmpeg.org/download.html) and add to PATH
    
    **Setup:**
    ```powershell
    # Clone repository
    git clone https://github.com/klukacin/goldplated-photos.git
    cd goldplated-photos
    
    # Install dependencies
    npm install
    
    # Start dev server (foreground only)
    npm run dev
    
    # In another terminal, start admin panel
    npm run admin
    ```
    
    !!! warning "Limitations on Native Windows"
        - ❌ `npm run dev:bg` / `npm run admin:bg` (background scripts)
        - ❌ `npm run stop:dev` / `npm run stop:admin`
        - ❌ `npm run deploy` (requires bash + rsync)
        
        **Workaround:** Open two terminal windows and run servers in foreground.
    
    ### Option 3: Git Bash (Partial)
    
    Git Bash provides a bash shell but with limitations.
    
    - ✅ Basic bash scripts work
    - ✅ `npm run dev`, `npm run admin`, `npm run build`
    - ❌ `rsync` not included - deploy will fail

=== "Linux"

    Linux has **native compatibility**. Install Node.js and optional dependencies:
    
    **Ubuntu/Debian:**
    ```bash
    # Install Node.js via nvm (recommended)
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
    source ~/.bashrc
    nvm install 20
    
    # Install ffmpeg (for video metadata)
    sudo apt install ffmpeg -y
    
    # Install rsync (for deployment, usually pre-installed)
    sudo apt install rsync -y
    ```
    
    **Fedora/RHEL:**
    ```bash
    # Install Node.js
    sudo dnf install nodejs -y
    
    # Install ffmpeg and rsync
    sudo dnf install ffmpeg rsync -y
    ```
    
    **Arch Linux:**
    ```bash
    # Install Node.js, ffmpeg, rsync
    sudo pacman -S nodejs npm ffmpeg rsync
    ```

=== "macOS"

    macOS has **native compatibility**. Install Node.js and optional dependencies:
    
    **Using Homebrew (recommended):**
    ```bash
    # Install Homebrew if not installed
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    
    # Install Node.js
    brew install node
    
    # Install ffmpeg (optional, for video metadata)
    brew install ffmpeg
    ```
    
    **Using nvm:**
    ```bash
    # Install nvm
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
    source ~/.zshrc  # or ~/.bashrc
    
    # Install Node.js
    nvm install 20
    nvm use 20
    ```

---

## Quick Install

```bash
# Clone the repository
git clone https://github.com/klukacin/goldplated-photos
cd goldplated-photos

# Install dependencies
npm install

# Start development server
npm run dev
```

The gallery will be available at `http://localhost:4321`

---

## Project Structure

```
goldplated-photos/
├── src/
│   ├── content/
│   │   ├── albums/          # Photo albums go here
│   │   │   └── {year}/
│   │   │       └── {category}/
│   │   │           └── {album-name}/
│   │   │               ├── index.md    # Album metadata
│   │   │               └── *.jpg       # Photos
│   │   └── home/            # Home page content
│   ├── components/          # Astro components
│   ├── layouts/             # Page layouts
│   ├── lib/                 # Utility functions
│   └── pages/               # Routes
├── public/                  # Static assets
│   ├── images/              # Landing page images
│   ├── sounds/              # Audio files
│   └── home/                # Home page assets
├── .meta/                   # Generated thumbnails (gitignored)
└── docs/                    # Documentation
```

---

## All npm Commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start Astro dev server (foreground, port 4321) |
| `npm run dev:bg` | Start dev server in background |
| `npm run stop:dev` | Stop background dev server |
| `npm run build` | Build production site to `./dist/` |
| `npm run preview` | Preview production build locally |
| `npm run deploy` | Full deployment to production server |
| `npm run update` | Normalize album structure (auto-create index.md) |
| `npm run admin` | Start admin panel (foreground, port 4444) |
| `npm run admin:bg` | Start admin panel in background |
| `npm run stop:admin` | Stop background admin panel |

---

## Development Setup

The recommended setup runs both the dev server and admin panel:

```bash
# Start both servers in background
npm run dev:bg
npm run admin:bg

# Open in browser
# Admin Panel: http://localhost:4444
# Gallery Preview: http://localhost:4321

# Stop when done
npm run stop:dev
npm run stop:admin
```

!!! tip "Admin Panel"
    The [Admin Panel](../guides/admin-panel.md) provides a web-based interface for managing albums, photos, and home page content. It's optional but makes content management much easier.

---

## Production Deployment

### Build for Production

```bash
npm run build
```

This creates a `dist/` folder with your optimized site.

### Server Requirements

Since Goldplated Photos uses SSR for password protection, you need a Node.js server:

```bash
# Production start
node dist/server/entry.mjs
```

### Environment Variables

```bash
# Optional: Set production port
PORT=3000
HOST=0.0.0.0
```

---

## Optional: Video Support

For video metadata extraction, install ffprobe:

=== "macOS"
    ```bash
    brew install ffmpeg
    ```

=== "Ubuntu/Debian"
    ```bash
    sudo apt install ffmpeg
    ```

=== "Windows"
    Download from [ffmpeg.org](https://ffmpeg.org/download.html) and add to PATH.

---

## Troubleshooting Installation

### Port Already in Use

The dev server will automatically try ports 4321, 4322, or 4323.

### Sharp Build Errors

Sharp (image processing) may need platform-specific binaries:

```bash
npm rebuild sharp
```

### Thumbnail Permissions

Ensure the `.meta/thumbnails/` directory is writable:

```bash
mkdir -p .meta/thumbnails
chmod 755 .meta/thumbnails
```

---

## Next Steps

- [Quick Start Guide](quick-start.md) - Create your first album
- [Configuration](configuration.md) - Customize your gallery
