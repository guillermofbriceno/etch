import DOMPurify from 'dompurify';
import { resolveMediaUrl } from './media';

const BASE_TAGS = [
    'b', 'strong', 'i', 'em', 'u', 'del', 's', 'strike',
    'code', 'pre', 'blockquote', 'br', 'p', 'span',
    'ul', 'ol', 'li', 'a', 'img', 'h1', 'h2', 'h3',
    'h4', 'h5', 'h6', 'hr', 'table', 'thead', 'tbody',
    'tr', 'th', 'td', 'sup', 'sub',
];

const BASE_ATTR = ['href', 'src', 'alt', 'title', 'class'];

/** Sanitize options for locally-rendered markdown (plain-text messages). */
export const MARKDOWN_SANITIZE = {
    ALLOWED_TAGS: BASE_TAGS,
    ALLOWED_ATTR: BASE_ATTR,
    RETURN_TRUSTED_TYPE: false as const,
};

/** Sanitize options for server-rendered HTML (Matrix formatted_body). */
export const HTML_BODY_SANITIZE = {
    ALLOWED_TAGS: [...BASE_TAGS, 'mx-reply'],
    ALLOWED_ATTR: [...BASE_ATTR, 'data-mx-maths'],
    RETURN_TRUSTED_TYPE: false as const,
};

let initialized = false;

/** Register app-wide DOMPurify hooks. Call once at startup. */
export function initSanitizer(): void {
    if (initialized) return;
    initialized = true;
    DOMPurify.addHook('afterSanitizeAttributes', (node) => {
        if (node instanceof HTMLElement) {
            for (const attr of ['src', 'href']) {
                const val = node.getAttribute(attr);
                if (val?.startsWith('mxc://')) {
                    node.setAttribute(attr, resolveMediaUrl(val)!);
                }
            }
        }
    });
}
