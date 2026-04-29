import { Marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import { hljs } from './highlight';
import DOMPurify from 'dompurify';
import { MARKDOWN_SANITIZE } from './sanitize';
import { replaceInTextNodes } from './dom';

// Renderer with syntax highlighting for display (only labeled blocks)
const renderParser = new Marked(
    markedHighlight({
        langPrefix: 'hljs language-',
        highlight(code, lang) {
            if (lang && hljs.getLanguage(lang)) {
                return hljs.highlight(code, { language: lang }).value;
            }
            return code;
        },
    }),
    { breaks: true, gfm: true },
);

// Compose parser: no syntax highlighting (recipients highlight on their end)
const composeParser = new Marked({ breaks: true, gfm: true });

/** Parse markdown to sanitized HTML for rendering incoming plain-text messages. */
export function markdownToHtml(md: string): string {
    const raw = renderParser.parse(md, { async: false }) as string;
    return DOMPurify.sanitize(raw, MARKDOWN_SANITIZE) as string;
}

/** Parse markdown to sanitized HTML for outgoing messages (Matrix formatted_body). */
export function composeHtml(md: string): string {
    const raw = composeParser.parse(md, { async: false }) as string;
    return DOMPurify.sanitize(raw, MARKDOWN_SANITIZE) as string;
}

/**
 * Rewrite @DisplayName text in HTML to matrix.to anchor tags.
 * Uses DOM traversal to avoid corrupting content inside attributes or code blocks.
 */
export function insertMentionLinks(html: string, mentions: Map<string, string>): string {
    if (mentions.size === 0) return html;

    const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');

    // Longest names first to avoid partial matches
    const patterns = [...mentions.keys()]
        .sort((a, b) => b.length - a.length)
        .map(name => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    const re = new RegExp(`@(${patterns.join('|')})(?=\\s|$)`, 'g');

    replaceInTextNodes(doc.body, re, (match) => {
        const displayName = match[1];
        const matrixId = mentions.get(displayName)!;
        const a = doc.createElement('a');
        a.href = `https://matrix.to/#/${matrixId}`;
        a.textContent = displayName;
        return a;
    });

    return doc.body.innerHTML;
}
