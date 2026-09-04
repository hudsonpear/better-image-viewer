// String helpers for the SVG control panel: paint scanning/swapping and root
// size edits. Pure, no DOM, so svgEdit.test.js can exercise them under node.

// Only values sitting in a paint property are colors; a bare hex scan would
// also catch ids, class names and gradient offsets.
// The value is either a function call — rgb(), hsl(), url() — whose parens and
// commas have to survive, or a plain token like "#fff" or "red".
const SVG_PAINT_RE =
  /\b(?:fill|stroke|stop-color|flood-color|lighting-color|solid-color)\s*(?:=\s*["']|:\s*)\s*([a-z-]+\([^)]*\)|[^"';)>\s]+)/gi;

const NON_COLORS = new Set([
  "none", "currentcolor", "inherit", "initial", "unset", "transparent",
  "context-fill", "context-stroke"
]);

// Distinct paint values in the markup, mapped to how often each appears.
export function svgColorsIn(code) {
  const counts = new Map();

  for (const match of code.matchAll(SVG_PAINT_RE)) {
    const raw = match[1].trim();
    if (!raw || raw.startsWith("url(") || NON_COLORS.has(raw.toLowerCase())) continue;
    counts.set(raw, (counts.get(raw) || 0) + 1);
  }

  return counts;
}

function escapeRe(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Swaps one paint value for another everywhere it appears as a whole token, so
// "#fff" doesn't also match inside "#fff000" or an id.
export function replaceColorIn(code, from, to) {
  const re = new RegExp(`(?<=[:="'\\s])${escapeRe(from)}(?=[;"'\\s>)]|$)`, "gi");
  return code.replace(re, to);
}

// ---- root <svg> size ----

// Span of the opening root tag. Scanned rather than regexed so a ">" inside an
// attribute value (a style, a title) doesn't end the tag early.
function rootTagRange(code) {
  const start = code.search(/<svg\b/i);
  if (start < 0) return null;

  let quote = null;
  for (let i = start; i < code.length; i++) {
    const ch = code[i];
    if (quote) {
      if (ch === quote) quote = null;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === ">") {
      return [start, i + 1];
    }
  }

  return null;
}

function parseAttrs(tag) {
  const attrs = [];
  const body = tag.replace(/^<svg/i, "").replace(/\/?>$/, "");

  for (const m of body.matchAll(/([:\w.-]+)\s*=\s*("[^"]*"|'[^']*'|[^\s"'>]+)/g)) {
    const quoted = /^["']/.test(m[2]);
    attrs.push({
      name: m[1],
      value: quoted ? m[2].slice(1, -1) : m[2],
      quote: quoted ? m[2][0] : '"'
    });
  }

  return attrs;
}

function findAttr(attrs, name) {
  return attrs.find(a => a.name.toLowerCase() === name);
}

// Lengths may carry units ("24px", "3em") or be relative ("100%"); only an
// absolute number is usable as a pixel size.
function lengthValue(raw) {
  if (!raw || /%\s*$/.test(raw)) return null;
  const n = parseFloat(raw);
  return Number.isFinite(n) && n > 0 ? n : null;
}

function viewBoxSize(attrs) {
  const box = (findAttr(attrs, "viewbox")?.value || "").trim().split(/[\s,]+/).map(Number);
  return box.length === 4 && box[2] > 0 && box[3] > 0 ? [box[2], box[3]] : null;
}

// Rendered size of the document, preferring explicit width/height and falling
// back to the viewBox. Null when the file states neither.
export function readSvgSize(code) {
  const range = rootTagRange(code);
  if (!range) return null;

  const attrs = parseAttrs(code.slice(range[0], range[1]));
  const box = viewBoxSize(attrs);

  const width = lengthValue(findAttr(attrs, "width")?.value) || (box && box[0]);
  const height = lengthValue(findAttr(attrs, "height")?.value) || (box && box[1]);

  return width && height ? { width, height } : null;
}

// Rewrites the root's width/height. A viewBox is added from the old size when
// the file lacks one, otherwise the new width/height would crop the drawing
// instead of scaling it.
export function resizeSvgCode(code, width, height) {
  if (!(width > 0) || !(height > 0)) return null;

  const range = rootTagRange(code);
  if (!range) return null;

  const tag = code.slice(range[0], range[1]);
  const attrs = parseAttrs(tag);
  const selfClosing = /\/>$/.test(tag);

  if (!viewBoxSize(attrs)) {
    const current = readSvgSize(code);
    if (!current) return null;
    attrs.push({ name: "viewBox", value: `0 0 ${current.width} ${current.height}`, quote: '"' });
  }

  for (const [name, value] of [["width", width], ["height", height]]) {
    const existing = findAttr(attrs, name);
    if (existing) {
      existing.value = String(value);
    } else {
      attrs.push({ name, value: String(value), quote: '"' });
    }
  }

  const rebuilt =
    "<svg" +
    attrs.map(a => ` ${a.name}=${a.quote}${a.value}${a.quote}`).join("") +
    (selfClosing ? " />" : ">");

  return code.slice(0, range[0]) + rebuilt + code.slice(range[1]);
}
