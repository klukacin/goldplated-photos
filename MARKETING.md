# Goldplated Photos - Marketing Content

> This file contains marketing copy for the goldplated.photos website.
> Content is organized by section for easy use in the marketing website.

---

## Headlines & Taglines

### Primary Headline
**Your Photos. Your Server. Your Rules.**

### Secondary Headlines
- A Beautiful Photo Gallery That Respects Your Privacy
- Self-Hosted Photo Gallery for Photographers
- Professional Gallery Features, Zero Cloud Dependencies

### Taglines
- Beautiful. Secure. Self-Hosted.
- Where your photos stay yours.
- The gallery that doesn't phone home.
- Own your memories.

---

## Value Propositions

### True Privacy
Your photos never leave your server. No cloud services, no third-party tracking, no data mining. When you host Goldplated Photos, your images stay exactly where you put them.

### No Database Required
Built on a file-based architecture using simple folders and markdown files. Back up with rsync, version control with git, migrate by copying files. Your content is never locked in.

### Modern Technology
Built with Astro 5, PhotoSwipe 5, and Sharp for image processing. Fast, modern, and actively maintained by developers who care about the web.

### Production Ready
Rate limiting, secure cookies, path traversal protection, and timing-safe password comparison. Security features that enterprise software charges extra for.

### One-Command Deploy
Configure your server once, then deploy with `npm run deploy`. Rsync-based synchronization with PM2 process management. Professional deployment without DevOps complexity.

### Human + AI Crafted
Built by Kristijan Lukacin with Claude AI assistance. Openly documented for AI-assisted development. A template for the future of software creation.

---

## Feature Highlights

### Gallery Experience

#### Multiple View Modes
Switch between Grid, Masonry, Single-column, and Slideshow views. Each mode is optimized for different browsing experiences—from quick scanning to immersive viewing.

#### PhotoSwipe Lightbox
Industry-standard lightbox with touch gestures, pinch-to-zoom, and swipe navigation. Feels native on mobile, powerful on desktop.

#### EXIF Metadata Display
Show your audience the camera settings behind each shot. Camera model, lens, aperture, shutter speed, ISO—all beautifully presented.

#### Video Support
Mixed media albums with inline video playback. Automatic metadata extraction shows duration, resolution, and codec information.

#### Keyboard Navigation
Full keyboard support with spatial awareness. Navigate photos naturally with arrow keys, open with Enter, close with Escape.

### Organization

#### Hierarchical Albums
Organize by year, event, client, or any structure that makes sense. Nested folders become nested albums automatically.

#### Collections
Group related albums under a parent collection. Perfect for multi-day events or ongoing projects.

#### Custom Cover Photos
Let the system pick the first photo, or choose your own. Every album deserves a great first impression.

#### Album Ordering
Control exactly how albums appear with simple frontmatter. Pin important albums to the top.

### Security

#### Password Protection
Server-side rendering means protected content is never sent to unauthorized browsers. Image URLs don't appear in source until after authentication.

#### Rate Limiting
10 failed attempts per 15 minutes per IP. Brute force attacks don't work here.

#### Secure Cookies
HttpOnly, Secure, and SameSite strict flags. Modern cookie security without configuration.

#### Path Traversal Protection
Every API endpoint validates paths. Directory traversal attacks are blocked at the source.

### Performance

#### Smart Thumbnails
Three sizes: 400px for grids, 1200px for lightbox, 1920px for full-screen. Right-sized images for every context.

#### On-Demand Generation
Thumbnails are created when first requested, then cached forever. No lengthy build processes for new albums.

#### Persistent Caching
Generated thumbnails are stored on disk with immutable cache headers. Subsequent visitors get instant loading.

#### EXIF Auto-Rotation
Sharp automatically rotates images based on EXIF orientation. No more sideways photos.

---

## Use Cases

### Professional Photographers
Deliver client galleries without monthly platform fees. Password protect sensitive work. Maintain your brand with customization.

### Personal Photo Sharing
Share family photos with relatives without uploading to social media. Control who sees what with album passwords.

### Event Photography
Wedding galleries, corporate events, conferences. Organize by date, share with clients, download entire albums as ZIP.

### Portfolio Sites
Showcase your best work with a gallery that loads fast and looks professional. No watermarks, no ads, no distractions.

### Photo Archives
Long-term storage with a beautiful frontend. File-based storage means your photos are safe for decades.

---

## Technical Highlights

### Stack
- **Astro 5** - Modern static site generator with SSR
- **PhotoSwipe 5** - Touch-friendly lightbox
- **Sharp** - High-performance image processing
- **Node.js** - Server runtime
- **Express** - Admin panel

### Requirements
- Node.js 18+ (LTS)
- npm 9+
- ffmpeg (optional, for video)
- Any Linux/macOS/Windows server

### Deployment Options
- VPS (DigitalOcean, Linode, Vultr)
- Dedicated server
- Home server
- Raspberry Pi (with patience)

---

## Comparison

### vs. Google Photos
| Feature | Goldplated | Google Photos |
|---------|-----------|---------------|
| Privacy | Your server | Google's servers |
| Cost | Server cost only | Free tier + storage fees |
| Ads | Never | In free tier |
| Data mining | Never | Yes |
| Offline access | Full control | Limited |
| Customization | Complete | None |

### vs. SmugMug/Zenfolio
| Feature | Goldplated | SmugMug |
|---------|-----------|---------|
| Monthly fee | None | $14-42/month |
| Branding | Your domain | Their branding |
| Storage | Your server | Their limits |
| Migration | Copy files | Export process |
| Features | Core gallery | Everything |

### vs. WordPress Gallery Plugins
| Feature | Goldplated | WordPress |
|---------|-----------|-----------|
| Security | Purpose-built | Plugin dependent |
| Performance | Static/SSR | Database queries |
| Maintenance | npm update | WP + plugins |
| Attack surface | Minimal | Large |

---

## FAQ

### Is this free?
Yes, Goldplated Photos is open source under the MIT license. You only pay for your server hosting.

### Do I need technical skills?
Basic command line knowledge is helpful. If you can run `npm install` and edit a config file, you can run Goldplated Photos.

### Can I use this for client work?
Absolutely. The MIT license allows commercial use. Many photographers use it for client delivery.

### How do I get support?
GitHub Issues for bugs, GitHub Discussions for questions. The community is friendly and responsive.

### Is my data really private?
Yes. Goldplated Photos never phones home. No analytics, no tracking, no external requests. Verify it yourself—the code is open source.

### Can I customize the design?
Yes. The codebase uses CSS custom properties and Astro components. Change colors, fonts, and layouts to match your brand.

### What about mobile?
The gallery is fully responsive with touch-optimized gestures. Long-press for context menus, swipe to navigate, pinch to zoom.

---

## Call to Action

### Primary CTA
**Get Started in 5 Minutes**
Clone the repo, install dependencies, run the dev server. Your photos, your gallery, your way.

### Secondary CTAs
- View the Live Demo
- Read the Documentation
- Star on GitHub
- Join the Community

---

## About Section

### The Creator
Kristijan Lukacin is a web developer and photographer from Croatia. He built Goldplated Photos to host his own photography work without compromising on privacy or paying monthly fees.

### The AI Partner
Claude, created by Anthropic, assisted with development. This human-AI collaboration produced a codebase that's clean, well-documented, and ready for contribution.

### The Philosophy
We believe photographers should own their platforms. No algorithms deciding what gets seen. No terms of service that can change overnight. No platform risk.

---

## Social Proof (Placeholders)

> "Finally, a photo gallery I can self-host without fighting WordPress plugins."
> — Developer & Photographer

> "My clients love the clean interface. I love not paying monthly fees."
> — Professional Photographer

> "Set it up once, forgot about it. Just works."
> — Hobbyist

---

## Links

- **Website**: https://goldplated.photos
- **Documentation**: https://docs.goldplated.photos
- **GitHub**: https://github.com/klukacin/goldplated-photos
- **Live Demo**: https://kristijan.lukacin.com
- **Creator**: https://kristijan.lukacin.com

---

*This marketing content is for the goldplated.photos website.*
*Last updated: January 2026*
