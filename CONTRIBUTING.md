# Contributing to Goldplated Photos

Thank you for your interest in contributing to Goldplated Photos! This guide will help you get started.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Working with Claude AI](#working-with-claude-ai)
- [Making Changes](#making-changes)
- [Pull Request Process](#pull-request-process)
- [Style Guidelines](#style-guidelines)

## Code of Conduct

Be respectful, inclusive, and constructive. We're building something great together.

## Getting Started

### Prerequisites

- Node.js 18+ (LTS recommended)
- npm 9+
- Git
- ffmpeg (for video metadata extraction)

### Development Setup

```bash
# Clone the repository
git clone https://github.com/klukacin/goldplated-photos.git
cd goldplated-photos

# Install dependencies
npm install

# Start development servers
npm run dev:bg      # Gallery on port 4321
npm run admin:bg    # Admin panel on port 4444
```

### Project Structure

```
goldplated-photos/
├── src/
│   ├── pages/          # Astro pages and API routes
│   ├── components/     # Astro components
│   ├── layouts/        # Layout templates
│   ├── lib/            # Utility functions
│   ├── content/        # Content collections (albums, home)
│   └── config.ts       # Site configuration
├── admin/              # Local admin panel
├── scripts/            # Utility scripts
├── docs/               # MkDocs documentation
└── public/             # Static assets
```

## Working with Claude AI

This project was built with Claude AI assistance, and we encourage contributors to use Claude Code for development.

### Using CLAUDE.md

The `CLAUDE.md` file contains comprehensive project context for AI-assisted development:

- Architecture overview
- Component documentation
- API endpoints
- Development patterns
- Troubleshooting guides

When using Claude Code:

1. Open the project in your terminal
2. Run `claude` to start Claude Code
3. Claude will automatically read CLAUDE.md for context
4. Ask Claude to help with your contribution

### Best Practices with Claude

- Reference specific files when asking for help
- Ask Claude to verify changes against existing patterns
- Use Claude for code review before submitting PRs
- Let Claude help write tests and documentation

## Making Changes

### Branch Naming

- `feature/description` - New features
- `fix/description` - Bug fixes
- `docs/description` - Documentation updates
- `refactor/description` - Code refactoring

### Commit Messages

Follow conventional commits:

```
type(scope): description

feat(lightbox): add zen mode toggle
fix(thumbnails): correct EXIF rotation
docs(readme): update installation guide
```

### Testing Your Changes

```bash
# Run development server
npm run dev

# Build and preview production
npm run build
npm run preview

# Test with admin panel
npm run admin
```

## Pull Request Process

1. **Fork the repository** and create your branch from `main`
2. **Make your changes** following our style guidelines
3. **Test thoroughly** - run the dev server and verify functionality
4. **Update documentation** if you changed public APIs or features
5. **Submit a PR** with a clear description of changes

### PR Description Template

```markdown
## Summary
Brief description of changes

## Changes
- Change 1
- Change 2

## Testing
How you tested the changes

## Screenshots
If applicable
```

### Review Process

- PRs require at least one approval
- Maintainers may request changes
- All conversations must be resolved before merging
- CI checks must pass

## Style Guidelines

### Code Style

- Use TypeScript for type safety
- Follow existing patterns in the codebase
- Use meaningful variable and function names
- Keep functions focused and small
- Add comments for complex logic only

### Astro Components

```astro
---
// Props interface at the top
interface Props {
  title: string;
  optional?: boolean;
}

// Destructure with defaults
const { title, optional = false } = Astro.props;

// Logic before template
const data = await fetchData();
---

<!-- Template with semantic HTML -->
<article class="component">
  <h2>{title}</h2>
</article>

<style>
  /* Scoped styles */
  .component {
    margin: 1rem;
  }
</style>
```

### CSS Guidelines

- Use CSS custom properties from Layout.astro
- Mobile-first responsive design
- WCAG AA accessibility compliance
- Respect `prefers-reduced-motion`

### Documentation

- Update CLAUDE.md for significant changes
- Keep README.md concise
- Add JSDoc comments for public functions
- Include examples in documentation

## Questions?

- Open an issue for bugs or feature requests
- Start a discussion for questions
- Check existing issues before creating new ones

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

---

Thank you for contributing to Goldplated Photos!
