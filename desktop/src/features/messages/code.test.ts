import assert from "node:assert/strict";
import { test } from "node:test";

import { findCode } from "./code.ts";

test("finds common shapes of code", () => {
  assert.equal(findCode("Your verification code is 482913."), "482913");
  assert.equal(findCode("Il tuo codice è 7734"), "7734");
  assert.equal(findCode("G-482913 is your Google verification code."), "G-482913");
  assert.equal(findCode("Your code: 482 913"), "482913");
});

test("prefers the number after the word that announces it", () => {
  assert.equal(findCode("On 2026 orders, use code 5521 at checkout"), "5521");
});

test("leaves ordinary messages alone", () => {
  assert.equal(findCode("See you at 1830 tomorrow"), null);
  assert.equal(findCode("Call me on +393331234567"), null);
  assert.equal(findCode("Your order of 1200 items shipped"), null);
});
