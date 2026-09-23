import assert from "node:assert/strict";

// This tolerance applies only to the equivalent two/66-segment raster probe.
// Storage packing and matched-loop constant/storage comparisons remain exact.
export function assertPathPixels(expected, actual) {
  assert.equal(actual.width, expected.width, "path image width");
  assert.equal(actual.height, expected.height, "path image height");
  const count = expected.width * expected.height;
  assert.ok(Number.isSafeInteger(count) && count > 0, "positive image dimensions");
  assert.equal(expected.data.length, count * 4, "expected RGBA length");
  assert.equal(actual.data.length, count * 4, "actual RGBA length");
  let changedPixels = 0;
  let maximumChannelDifference = 0;
  for (let offset = 0; offset < expected.data.length; offset += 4) {
    assert.equal(actual.data[offset + 3], expected.data[offset + 3], "path alpha must match exactly");
    let changed = false;
    for (let channel = 0; channel < 3; channel++) {
      const delta = Math.abs(actual.data[offset + channel] - expected.data[offset + channel]);
      assert.ok(delta <= 1, "path RGB difference exceeds one 8-bit level");
      changed ||= delta !== 0;
      maximumChannelDifference = Math.max(maximumChannelDifference, delta);
    }
    changedPixels += Number(changed);
  }
  assert.ok(changedPixels <= Math.floor(count / 10_000), "path changed pixels exceed 0.01%");
  return { changedPixels, maximumChannelDifference };
}

export async function decodePixels(page, pngs) {
  return page.evaluate(async encodedImages => Promise.all(encodedImages.map(async encoded => {
    const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${encoded}`)).blob());
    const surface = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = surface.getContext("2d");
    context.drawImage(bitmap, 0, 0);
    bitmap.close();
    return { width: surface.width, height: surface.height,
      data: Array.from(context.getImageData(0, 0, surface.width, surface.height).data) };
  })), pngs.map(png => png.toString("base64")));
}
