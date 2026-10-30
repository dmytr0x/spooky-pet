import assert from "node:assert/strict";
import test from "node:test";
import { splitToViewport } from "./chunks.js";

function fitsAtMost(limit) {
  return (text) => text.length <= limit;
}

test("short text stays one chunk", () => {
  assert.deepEqual(splitToViewport("Boo.", fitsAtMost(10)), ["Boo."]);
});

test("long text splits on word boundaries at the viewport limit", () => {
  assert.deepEqual(splitToViewport("one two three four", fitsAtMost(8)), [
    "one two",
    "three",
    "four",
  ]);
});

test("a single overlong token is hard-split to fit", () => {
  assert.deepEqual(splitToViewport("supercalifragilistic", fitsAtMost(6)), [
    "super",
    "calif",
    "ragil",
    "istic",
  ]);
});

test("empty text is one empty chunk", () => {
  assert.deepEqual(splitToViewport("   ", fitsAtMost(4)), [""]);
});

test("two-page text splits near the middle instead of leaving a runt", () => {
  assert.deepEqual(
    splitToViewport("alpha beta gamma delta epsilon zeta", fitsAtMost(30)),
    ["alpha beta gamma", "delta epsilon zeta"],
  );
});

test("prefers a sentence break near the midpoint", () => {
  assert.deepEqual(
    splitToViewport(
      "The costume has too many layers. Bugs in the seams.",
      fitsAtMost(44),
    ),
    ["The costume has too many layers.", "Bugs in the seams."],
  );
});
