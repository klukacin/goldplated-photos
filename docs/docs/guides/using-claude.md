# Using Claude AI for Development

Goldplated Photos was built with [Claude AI](https://claude.ai) assistance and includes comprehensive documentation for AI-assisted development.

## What is Claude Code?

[Claude Code](https://claude.ai/code) is Anthropic's AI-powered coding assistant that runs in your terminal. It can:

- Read and understand your codebase
- Write and modify code
- Run commands and tests
- Explain complex code
- Debug issues

## Getting Started

### 1. Install Claude Code

```bash
# Install via npm
npm install -g @anthropic-ai/claude-code

# Or via Homebrew (macOS)
brew install claude-code
```

### 2. Navigate to Project

```bash
cd /path/to/goldplated-photos
```

### 3. Start Claude Code

```bash
claude
```

Claude will automatically read `CLAUDE.md` and understand the project structure.

## The CLAUDE.md File

The project includes a comprehensive `CLAUDE.md` file that provides:

- **Development Commands** - All npm scripts and their purposes
- **Architecture Overview** - How the application is structured
- **Component Documentation** - What each component does
- **API Endpoints** - All routes with parameters and responses
- **Important Patterns** - Coding conventions and gotchas
- **Troubleshooting** - Common issues and solutions

This file serves as context for Claude, enabling it to provide accurate, project-specific assistance.

## Example Workflows

### Adding a New Feature

```
You: I want to add a slideshow mode to the photo grid

Claude: I'll help you add a slideshow mode. Let me first check the
existing view modes in PhotoGrid.astro...

[Claude reads the relevant files and suggests implementation]
```

### Debugging an Issue

```
You: The thumbnails aren't loading for some albums

Claude: Let me investigate the thumbnail generation system. I'll check
the API endpoint and the cache directory...

[Claude traces the issue through the codebase]
```

### Understanding Code

```
You: How does the password protection work?

Claude: The password protection uses Server-Side Rendering (SSR). Let
me walk you through the flow...

[Claude explains the authentication flow with code references]
```

## Best Practices

### Be Specific

Instead of:
```
Fix the bug
```

Try:
```
The EXIF overlay isn't showing when I press the 'i' key in the lightbox
```

### Reference Files

```
In src/components/PhotoGrid.astro, can you explain the keyboard
navigation logic?
```

### Ask for Verification

```
Before you make changes, can you verify this won't break the
existing sort functionality?
```

### Request Tests

```
Can you add this feature and also update any relevant tests?
```

## What Claude Does Well

- **Code Navigation** - Finding relevant files and functions
- **Pattern Matching** - Following existing code conventions
- **Documentation** - Writing clear comments and docs
- **Debugging** - Tracing issues through the codebase
- **Refactoring** - Improving code structure safely

## Limitations

- **External APIs** - May need to verify current API documentation
- **Complex State** - May need guidance on application-specific state
- **Design Decisions** - Works best with clear requirements
- **Testing** - May need help setting up test environments

## Contributing with Claude

When contributing to Goldplated Photos:

1. **Start Fresh** - Begin a new Claude session for each feature
2. **Provide Context** - Explain what you're trying to achieve
3. **Review Changes** - Always review Claude's suggestions before committing
4. **Test Thoroughly** - Run the dev server and verify functionality
5. **Update CLAUDE.md** - If you add new features, update the documentation

## Keeping CLAUDE.md Current

If you add significant features:

1. Document new components in the appropriate section
2. Add new API endpoints to the reference
3. Update troubleshooting if you discover common issues
4. Note any new patterns or conventions

This ensures future developers (and Claude) can effectively work with your additions.

## Resources

- [Claude Code Documentation](https://claude.ai/code)
- [Anthropic API Documentation](https://docs.anthropic.com)
- [CLAUDE.md in this project](https://github.com/klukacin/goldplated-photos/blob/main/CLAUDE.md)

---

*AI-assisted development is the future. We're building it openly.*
