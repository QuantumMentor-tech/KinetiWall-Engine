/**
 * Safe Declarative UI Node Primitives & Escaped HTML Renderer (`DEC-UI-02`).
 * Guarantees zero `dangerouslySetInnerHTML` / XSS execution and provides deterministic
 * tree inspection for unit, accessibility, and integration tests.
 */

export interface UiKeyboardEvent {
  key: string;
  shiftKey?: boolean;
  preventDefault?: () => void;
  stopPropagation?: () => void;
}

export interface UiNode {
  tag: string;
  key?: string;
  attrs: Record<string, string | number | boolean | undefined>;
  children: Array<UiNode | string>;
  onClick?: () => void;
  onKeyDown?: (event: UiKeyboardEvent) => void;
  onInput?: (value: string) => void;
  onChange?: (value: string) => void;
  onImageError?: () => void;
}

export function h(
  tag: string,
  props: {
    key?: string;
    attrs?: Record<string, string | number | boolean | undefined>;
    onClick?: () => void;
    onKeyDown?: (event: UiKeyboardEvent) => void;
    onInput?: (value: string) => void;
    onChange?: (value: string) => void;
    onImageError?: () => void;
  } | null,
  ...children: Array<UiNode | string | null | undefined | false>
): UiNode {
  const normalizedChildren: Array<UiNode | string> = [];
  for (const child of children) {
    if (child !== null && child !== undefined && child !== false) {
      normalizedChildren.push(child);
    }
  }
  return {
    tag,
    key: props?.key,
    attrs: { ...(props?.attrs ?? {}) },
    children: normalizedChildren,
    onClick: props?.onClick,
    onKeyDown: props?.onKeyDown,
    onInput: props?.onInput,
    onChange: props?.onChange,
    onImageError: props?.onImageError,
  };
}

/**
 * Strictly escapes arbitrary strings before rendering to HTML text or attribute values (`DEC-UI-02`).
 */
export function escapeHtml(raw: string): string {
  return raw
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

const VOID_ELEMENTS = new Set(['img', 'input', 'br', 'hr', 'meta', 'link']);

export function renderToHtml(node: UiNode | string): string {
  if (typeof node === 'string') {
    return escapeHtml(node);
  }
  const attrPairs: string[] = [];
  if (node.key !== undefined) {
    attrPairs.push(`data-key="${escapeHtml(node.key)}"`);
  }
  for (const [k, v] of Object.entries(node.attrs)) {
    if (v === undefined || v === false) {
      continue;
    }
    if (v === true) {
      attrPairs.push(escapeHtml(k));
    } else {
      attrPairs.push(`${escapeHtml(k)}="${escapeHtml(String(v))}"`);
    }
  }
  const attrString = attrPairs.length > 0 ? ` ${attrPairs.join(' ')}` : '';
  if (VOID_ELEMENTS.has(node.tag)) {
    return `<${node.tag}${attrString} />`;
  }
  const inner = node.children.map((c) => renderToHtml(c)).join('');
  return `<${node.tag}${attrString}>${inner}</${node.tag}>`;
}

export function getTextContent(node: UiNode | string): string {
  if (typeof node === 'string') {
    return node;
  }
  return node.children.map((c) => getTextContent(c)).join('');
}

export function findAllNodes(root: UiNode, predicate: (node: UiNode) => boolean): UiNode[] {
  const results: UiNode[] = [];
  function walk(current: UiNode | string): void {
    if (typeof current === 'string') {
      return;
    }
    if (predicate(current)) {
      results.push(current);
    }
    for (const child of current.children) {
      walk(child);
    }
  }
  walk(root);
  return results;
}

export function findNodeByTestId(root: UiNode, testId: string): UiNode | null {
  const matches = findAllNodes(root, (n) => n.attrs['data-testid'] === testId);
  return matches[0] ?? null;
}

export function findAllByTestIdPrefix(root: UiNode, prefix: string): UiNode[] {
  return findAllNodes(
    root,
    (n) => typeof n.attrs['data-testid'] === 'string' && n.attrs['data-testid'].startsWith(prefix),
  );
}

/**
 * Simulates keyboard activation (`Enter` or `Space`) on an interactive node per WCAG 2.1 AA.
 */
export function activateByKeyboard(node: UiNode, key: 'Enter' | ' '): boolean {
  if (node.attrs.disabled === true || node.attrs['aria-disabled'] === 'true') {
    return false;
  }
  if (node.onKeyDown) {
    node.onKeyDown({ key });
    return true;
  }
  if ((node.tag === 'button' || node.attrs.role === 'button') && node.onClick) {
    node.onClick();
    return true;
  }
  return false;
}

/**
 * Simulates a pointer click on an interactive node, respecting disabled state.
 */
export function clickNode(node: UiNode): boolean {
  if (node.attrs.disabled === true || node.attrs['aria-disabled'] === 'true') {
    return false;
  }
  if (node.onClick) {
    node.onClick();
    return true;
  }
  return false;
}

/**
 * Simulates typing or selecting a value on an input/select node, respecting disabled state.
 */
export function changeNodeValue(node: UiNode, value: string): boolean {
  if (node.attrs.disabled === true || node.attrs['aria-disabled'] === 'true') {
    return false;
  }
  if (node.onInput) {
    node.onInput(value);
    return true;
  }
  if (node.onChange) {
    node.onChange(value);
    return true;
  }
  return false;
}


