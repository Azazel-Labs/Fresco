import { test, expect } from "@playwright/test";
const prefix="in space cells(cell: tile) {\n let outer = tile.contour();\n let inner = tile.contour();\n ";
test("editor automatically offers chase arguments and contour values", async ({page}) => {
  await page.goto("/tests/editor/"); await page.waitForFunction(()=>window.editorTest);
  await page.evaluate(source=>window.editorTest.setSource(source),prefix+"chase");
  await page.keyboard.type("(");
  const rows=page.locator(".suggest-widget.visible .monaco-list-row");
  await expect(rows.filter({hasText:"along:"}).first()).toBeVisible();
  await rows.filter({hasText:"along:"}).first().click();
  await expect(rows.filter({hasText:"outer"}).first()).toBeVisible();
  await expect(rows.filter({hasText:"inner"}).first()).toBeVisible();
});
test("editor reopens suggestions after argument whitespace and newline",async({page})=>{
  await page.goto("/tests/editor/");await page.waitForFunction(()=>window.editorTest);
  for(const [source,text,label] of [["chase("," ","along:"],["chase(along: outer,","\n","speed:"],["chase(along:"," ","outer"]]) {
    await page.keyboard.press("Escape");
    await page.evaluate(source=>window.editorTest.setSource(source),prefix+source);
    await page.keyboard.type(text);
    await expect(page.locator(".suggest-widget.visible .monaco-list-row").filter({hasText:label}).first()).toBeVisible();
  }
});
test("deleting whole values or members automatically reopens suggestions", async ({page}) => {
  await page.goto("/tests/editor/"); await page.waitForFunction(() => window.editorTest);
  for (const [source, count, key, label] of [
    ["chase(along: outer", 5, "Backspace", "outer"],
    ["outer.distance", 8, "Delete", "distance"],
    ["chase(x", 1, "Backspace", "along:"],
    ["let value = outer", 5, "Backspace", "abs"],
  ]) {
    await page.keyboard.press("Escape");
    await page.evaluate(source => window.editorTest.setSource(source), prefix + source);
    for (let i = 0; i < count; i++) await page.keyboard.press("Shift+ArrowLeft");
    await page.keyboard.press(key);
    await expect(page.locator(".suggest-widget.visible .monaco-list-row").filter({hasText: label}).first()).toBeVisible();
  }
});
