# Custom Stylesheets

You can override the app's appearance by pointing to a custom CSS file in `settings.json`:

```json
{
  "custom_css": "/home/user/etch-theme.css"
}
```

The CSS file is injected after all default styles, so your rules take precedence. The easiest way to restyle is by overriding CSS variables:

```css
:root {
  --text-primary: #e0e0e0;
  --accent: #ff6b9d;
  --primary: #ff6b9d;
  --bg-primary: #1a1a2e;
  --message-spacing: 8px;
  --font-family: 'JetBrains Mono', monospace;
}
```

If the CSS file cannot be read, the app loads normally with default styling.

## Available Variables

### Backgrounds

| Variable | Default | Description |
|---|---|---|
| `--bg-base` | gradient | App background |
| `--bg-primary` | `#121212` | Main content background |
| `--bg-secondary` | `#0a0a0a` | Sidebar background |
| `--bg-tertiary` | `#1e1e1e` | Tertiary surface |
| `--bg-inset` | `#2f3136` | Code blocks, inset panels |
| `--bg-floating` | `#18191c` | Dropdown menus, context menus |
| `--bg-input` | `rgba(255,255,255,0.06)` | Input field background |
| `--bg-hover` | `rgba(255,255,255,0.06)` | Hover state background |
| `--bg-active` | `rgba(255,255,255,0.08)` | Active state background |
| `--bg-panel` | `rgba(255,255,255,0.04)` | Panel background |

### Colors

| Variable | Default | Description |
|---|---|---|
| `--accent` | `#7289da` | Accent color (borders, highlights) |
| `--accent-hover` | `#8ea1e1` | Accent hover |
| `--primary` | `#5865f2` | Primary action color (buttons, indicators) |
| `--primary-hover` | `#4752c4` | Primary hover |

### Text

| Variable | Default | Description |
|---|---|---|
| `--text-bright` | `#fff` | Brightest text (headings, active items) |
| `--text-primary` | `#dcddde` | Primary text |
| `--text-secondary` | `#b9bbbe` | Secondary text |
| `--text-tertiary` | `#8e9297` | Tertiary text (icons, dim labels) |
| `--text-muted` | `#72767d` | Muted text (timestamps, placeholders) |
| `--text-link` | `#00aff4` | Link color |

### Status

| Variable | Default | Description |
|---|---|---|
| `--status-success` | `#3ba55d` | Connected, talking |
| `--status-warning` | `#faa61a` | Connecting |
| `--status-danger` | `#ed4245` | Errors, disconnected, muted |

### Borders

| Variable | Default | Description |
|---|---|---|
| `--border-subtle` | `#202225` | Subtle borders and dividers |
| `--border-medium` | `#4f545c` | Medium borders (blockquotes, rules) |
| `--border-input` | `rgba(255,255,255,0.08)` | Input field border |
| `--border-panel` | `transparent` | Panel border |
| `--scrollbar-thumb` | `#202225` | Scrollbar thumb |
| `--scrollbar-track` | `#2e3035` | Scrollbar track |

### Typography

| Variable | Default | Description |
|---|---|---|
| `--font-family` | `'Inter', sans-serif` | Global font |
| `--font-family-mono` | `'Consolas', monospace` | Code font |
| `--font-size-base` | `14px` | Base font size for body text |
| `--font-size-channel` | `16px` | Channel name and category header font size |
| `--font-size-chat` | `15px` | Message body text font size |

### Layout

| Variable | Default | Description |
|---|---|---|
| `--message-spacing` | `16px` | Vertical gap between message groups |
| `--channel-item-padding` | `6px 8px` | Channel list item padding |
| `--sidebar-width` | `240px` | Sidebar width |
| `--avatar-size` | `40px` | Message avatar size |
