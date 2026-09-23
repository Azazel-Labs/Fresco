import { test, expect } from "@playwright/test";

for (const kind of ["scalar", "vector", "matrix", "color", "array", "dynamic", "dynamic-vector", "dynamic-color"]) {
  test(`${kind} preserves draft text and commits on Enter or blur`, async ({ page }) => {
    await page.route("**/param-editor-test", route => route.fulfill({
      contentType: "text/html", body: '<div id="host"></div><button>Outside</button>',
    }));
    await page.goto("/param-editor-test");
    await page.evaluate(async kind => {
      const scalar = await import("/src/app/param-editors/scalar-param-editors.tsx");
      const { renderMatrixParamEditor } = await import("/src/app/param-editors/matrix-param-editor.tsx");
      const { renderColorParamEditor } = await import("/src/app/param-editors/color-param-editor.tsx");
      const { renderDynamicArrayParamEditor } = await import("/src/app/param-editors/dynamic-array-param-editor.tsx");
      window.changes = [];
      const options = {
        value: 0.5, values: [0.5, 0.5, 0.5, 0.5], min: 0, max: 1,
        componentCount: 2, cols: 2, rows: 2, withSlider: true,
        onChange: value => window.changes.push(value),
      };
      const host = document.getElementById("host");
      if (kind === "scalar") window.editor = scalar.renderNumberParamEditor(host, options);
      if (kind === "vector") window.editor = scalar.renderVectorParamEditor(host, options);
      if (kind === "array") window.editor = scalar.renderArrayItemParamEditor(host, options);
      if (kind === "matrix") window.editor = renderMatrixParamEditor(host, options);
      if (kind === "color") window.editor = renderColorParamEditor(host, { ...options, value: options.values });
      if (kind.startsWith("dynamic")) window.editor = renderDynamicArrayParamEditor(host, {
        ...options, paramName: "items", elementType: "float",
        componentWidth: kind === "dynamic-color" ? 4 : kind === "dynamic-vector" ? 2 : 1,
        isColor: kind === "dynamic-color",
      });
    }, kind);
    const input = page.locator("[data-number-editor]").first();
    await input.focus();
    for (const text of ["", "-", "1e-", "whatever", "12.34"]) {
      await input.fill(text);
      await expect(input).toHaveValue(text);
      expect(await page.evaluate(() => window.changes)).toEqual([]);
    }
    await page.evaluate(kind => window.editor.update(
      kind === "scalar" ? 0.75 : [0.75, 0.75, 0.75, 0.75],
    ), kind);
    await expect(input).toHaveValue("12.34");
    await input.press("Enter");
    expect(await input.inputValue()).toMatch(/^1(?:\.00)?$/);
    expect(await page.evaluate(() => window.changes.length)).toBe(1);
    await page.getByRole("button", { name: "Outside", exact: true }).click();
    expect(await page.evaluate(() => window.changes.length)).toBe(1);
    await input.fill("0.25");
    await page.getByRole("button", { name: "Outside", exact: true }).click();
    await expect(input).toHaveValue("0.25");
    expect(await page.evaluate(() => window.changes.length)).toBe(2);
    await input.fill("invalid");
    await input.press("Enter");
    await expect(input).toHaveValue("0.25");
    expect(await page.evaluate(() => window.changes.length)).toBe(2);
    await input.fill("0");
    await input.press("Enter");
    expect(await input.inputValue()).toMatch(/^0(?:\.00)?$/);
    if (kind === "color" || kind === "dynamic-color") {
      const alpha = page.locator("[data-number-editor]").nth(3);
      await alpha.fill("0");
      await alpha.press("Enter");
      await expect(alpha).toHaveValue("0.00");
      expect(await page.evaluate(() => window.changes.at(-1)[3])).toBe(0);
    }
    if (kind === "scalar") {
      await page.locator('input[type="range"]').fill("0.6");
      expect(await page.evaluate(() => window.changes.at(-1))).toBe(0.6);
      await expect(input).toHaveValue("0.6");
    }
  });
}
