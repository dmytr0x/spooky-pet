import assert from "node:assert/strict";
import test from "node:test";
import { chunkVisibleMs } from "./delay.js";

test("a full bubble of text uses the configured delay", () => {
  assert.equal(chunkVisibleMs("x".repeat(80), 8000), 8000);
});

test("half as much text stays visible half as long", () => {
  assert.equal(chunkVisibleMs("x".repeat(40), 8000), 4000);
});

test("a short leftover chunk stays visible less than a long one", () => {
  const longMs = chunkVisibleMs("I counted your tabs. They counted back. Twice.", 8000);
  const shortMs = chunkVisibleMs("Boo.", 8000);
  assert.equal(longMs, 4600);
  assert.equal(shortMs, 1000);
  assert.ok(shortMs < longMs);
});

test("text longer than a full bubble does not exceed the configured delay", () => {
  assert.equal(chunkVisibleMs("x".repeat(200), 8000), 8000);
});

test("empty text uses the one-second floor", () => {
  assert.equal(chunkVisibleMs("   ", 8000), 1000);
});
