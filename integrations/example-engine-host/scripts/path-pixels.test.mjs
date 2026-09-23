import assert from "node:assert/strict";
import test from "node:test";
import { assertPathPixels } from "./path-pixels.mjs";

const image = () => ({ width: 514, height: 514, data: new Uint8Array(514 * 514 * 4).fill(128) });
test("equivalent path images accept only the documented bounded RGB differences", () => {
  const expected = image(), actual = image();
  assert.deepEqual(assertPathPixels(expected, actual), { changedPixels: 0, maximumChannelDifference: 0 });
  for (let i = 0; i < 26; i++) actual.data[i * 4]++;
  assert.deepEqual(assertPathPixels(expected, actual), { changedPixels: 26, maximumChannelDifference: 1 });
  actual.data[26 * 4]++;
  assert.throws(() => assertPathPixels(expected, actual), /0.01%/);
});
for (const [name, mutate, message] of [
  ["excessive RGB difference", a => a.data[0] += 2, /one 8-bit level/],
  ["alpha difference", a => a.data[3]++, /alpha/],
  ["width mismatch", a => a.width++, /width/],
  ["height mismatch", a => a.height++, /height/],
  ["truncated RGBA", a => a.data = a.data.slice(4), /RGBA length/],
]) {
  test(`rejects ${name}`, () => {
    const expected = image(), actual = image();
    mutate(actual);
    assert.throws(() => assertPathPixels(expected, actual), message);
  });
}
