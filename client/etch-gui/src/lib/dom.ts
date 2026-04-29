/**
 * Walk text nodes inside a container, skipping <code> and <pre>, and replace
 * regex matches with nodes produced by `replacer`.
 *
 * The regex must use the global flag. It is reset for each text node.
 */
export function replaceInTextNodes(
    root: Node,
    regex: RegExp,
    replacer: (match: RegExpExecArray) => Node,
): void {
    const doc = root.ownerDocument ?? document;
    const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const textNodes: Text[] = [];
    let node: Node | null;
    while ((node = walker.nextNode())) textNodes.push(node as Text);

    for (const textNode of textNodes) {
        if (textNode.parentElement?.closest('code, pre')) continue;
        const text = textNode.textContent ?? '';
        const frag = doc.createDocumentFragment();
        let lastIndex = 0;
        let match: RegExpExecArray | null;
        let hasMatches = false;

        regex.lastIndex = 0;
        while ((match = regex.exec(text)) !== null) {
            hasMatches = true;
            if (match.index > lastIndex) {
                frag.appendChild(doc.createTextNode(text.slice(lastIndex, match.index)));
            }
            frag.appendChild(replacer(match));
            lastIndex = match.index + match[0].length;
        }

        if (hasMatches) {
            if (lastIndex < text.length) {
                frag.appendChild(doc.createTextNode(text.slice(lastIndex)));
            }
            textNode.replaceWith(frag);
        }
    }
}
