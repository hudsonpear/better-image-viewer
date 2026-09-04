// Run with: node src/svgEdit.test.js
import assert from "node:assert/strict";
import { svgColorsIn, replaceColorIn, readSvgSize, resizeSvgCode } from "./svgEdit.js";

// ---- colors ----

const svg = `<svg viewBox="0 0 10 10" xmlns="http://www.w3.org/2000/svg">
  <defs><linearGradient id="fff000"><stop stop-color="#fff"/></linearGradient></defs>
  <rect fill="#fff" stroke='red' style="fill:#fff;stroke-width:2"/>
  <circle fill="none" stroke="url(#fff000)"/>
  <path fill="rgb(1, 2, 3)"/>
</svg>`;

const found = svgColorsIn(svg);

assert.equal(found.get("#fff"), 3, "counts every fill/stroke/stop-color hit");
assert.equal(found.get("red"), 1, "named colors count");
assert.equal(found.get("rgb(1, 2, 3)"), 1, "rgb() survives its commas and spaces");
assert.ok(!found.has("none"), "'none' is not a color");
assert.ok(![...found.keys()].some(k => k.startsWith("url(")), "gradient references are skipped");

const recolored = replaceColorIn(svg, "#fff", "#123456");

assert.equal(svgColorsIn(recolored).get("#123456"), 3, "every occurrence is swapped");
assert.ok(!recolored.includes('"#fff"'), "no original token left behind");
assert.ok(recolored.includes('id="fff000"'), "ids that merely contain the token are untouched");
assert.ok(recolored.includes('stroke="url(#fff000)"'), "url() references are untouched");

assert.equal(
  replaceColorIn('<path fill="rgb(1, 2, 3)"/>', "rgb(1, 2, 3)", "#abcdef"),
  '<path fill="#abcdef"/>',
  "rgb() values are escaped before matching"
);

// ---- size ----

assert.deepEqual(readSvgSize(svg), { width: 10, height: 10 }, "falls back to the viewBox");

assert.deepEqual(
  readSvgSize('<svg width="24px" height="16px"><rect/></svg>'),
  { width: 24, height: 16 },
  "units are stripped"
);

assert.deepEqual(
  readSvgSize('<svg width="100%" height="100%" viewBox="0 0 8 4"/>'),
  { width: 8, height: 4 },
  "percentages fall through to the viewBox"
);

assert.equal(readSvgSize("<p>not an svg</p>"), null, "non-SVG input is rejected");

const sized = resizeSvgCode('<svg width="24" height="16"><rect fill="red"/></svg>', 96, 64);

assert.ok(sized.includes('width="96"') && sized.includes('height="64"'), "new size is written");
assert.ok(sized.includes('viewBox="0 0 24 16"'), "a viewBox is added from the old size");
assert.ok(sized.includes('<rect fill="red"/>'), "the drawing is untouched");
assert.deepEqual(readSvgSize(sized), { width: 96, height: 64 }, "reads back at the new size");

const keptBox = resizeSvgCode('<svg viewBox="0 0 5 5" width="5" height="5"><g/></svg>', 50, 50);
assert.equal(
  (keptBox.match(/viewBox/g) || []).length,
  1,
  "an existing viewBox is not duplicated"
);
assert.ok(keptBox.includes('viewBox="0 0 5 5"'), "an existing viewBox keeps its values");

// a ">" inside an attribute value must not be mistaken for the end of the tag
const trickyTag = `<svg xmlns="http://www.w3.org/2000/svg" data-note="a > b" viewBox="0 0 2 2"><rect/></svg>`;
const trickySized = resizeSvgCode(trickyTag, 20, 20);
assert.ok(trickySized.includes('data-note="a > b"'), "attributes with '>' survive");
assert.ok(trickySized.includes("<rect/></svg>"), "tag end is found correctly");

const selfClosing = resizeSvgCode('<svg viewBox="0 0 2 2" width="2" height="2"/>', 8, 8);
assert.ok(selfClosing.trimEnd().endsWith("/>"), "a self-closing root stays self-closing");

assert.equal(resizeSvgCode(svg, 0, 10), null, "a zero dimension is refused");

console.log("svgEdit: all checks passed");
