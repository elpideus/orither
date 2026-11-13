import assert from "node:assert/strict";
import { test } from "node:test";

import { qrModules } from "./qr.ts";

test("the corner squares sit at the very edge, where the drawing expects them", () => {
  const { size, data } = qrModules("orither://pair/0123456789ABCDEFGHJKMNPQRS");
  const startsDark = (row: boolean[]) => row.slice(0, 7).every(Boolean);
  assert.ok(startsDark(data[0]));
  assert.ok(startsDark(data[size - 1]));
});
