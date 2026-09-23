// Local-only regression of the actual playground using the shared Rust renderer.
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const require=createRequire(new URL("../../../crates/fresco-wasm/web/package.json",import.meta.url));
const {chromium}=require("@playwright/test");
const browser=await chromium.launch({channel:process.env.FRESCO_GPU_BROWSER??"chrome",headless:true});
const deadline=setTimeout(()=>browser.close().catch(console.error),180_000);
const base=process.env.FRESCO_PLAYGROUND_URL??"http://127.0.0.1:5183/";
try {
  const page=await browser.newPage({viewport:{width:1200,height:900}});
  const errors=[];page.on("pageerror",error=>errors.push(String(error)));
  const legacyRequests=[];
  page.on("request",request=>{
    if (/\/(?:assets|src\/preview|tests\/reference)\/(?:preview|surface)-renderer[-.]/.test(new URL(request.url()).pathname)) {
      legacyRequests.push(request.url());
    }
  });
  const canvas=page.locator("#preview-canvas");
  // Capture GPU output without the overlaid picker's hover/focus decoration.
  const capture=()=>canvas.screenshot({style:"#mesh-picker, #preview-spin { visibility: hidden !important; }"});
  const frames=async()=>{const before=Number(await canvas.getAttribute("data-engine-frames"));await page.waitForFunction(n=>Number(document.querySelector("#preview-canvas").dataset.engineFrames)>n+3,before);};
  const open=async (source, surface)=>{
    const url=new URL(base);url.searchParams.delete("renderer");url.searchParams.set("code","raw."+Buffer.from(source).toString("base64url"));
    if (surface) url.searchParams.set("surface", surface);
    await page.goto(url.href);
    await page.waitForFunction(()=>Number(document.querySelector("#preview-canvas").dataset.engineFrames)>3,undefined,{timeout:60_000});
    assert.equal(await page.locator("#preview-backend").count(), 0);
  };
  const multiSurface = `
    surface plain(sp: surf) -> material(unlit) { compose { base(albedo: #ff0000) } }
    surface lit(sp: surf) -> material(standard) { compose { base(albedo: #00ff00) } }
  `;
  await open(multiSurface);
  assert.equal(await canvas.getAttribute("data-engine-entry"), "lit", "automatic preview prefers the material with a lighting pipeline");
  const automaticSurface = await capture();
  await open(multiSurface, "lit");
  assert.equal(await canvas.getAttribute("data-engine-entry"), "lit");
  assert.deepEqual(await capture(), automaticSurface, "automatic and explicit selection execute the same surface");
  await open(multiSurface, "plain");
  assert.equal(await canvas.getAttribute("data-engine-entry"), "plain", "explicit selection overrides the lighting preference");
  assert.notDeepEqual(await capture(), automaticSurface, "selecting another surface changes actual GPU output");
  await open("canvas probe(ctx: CanvasContext) -> color { param gain: f32 = 1 in 0 .. 1; rgba(gain, 1 - gain, 0, 1) }");
  assert.deepEqual(legacyRequests,[],"Rust preview does not fetch the legacy GPU renderers");
  const red=await capture();
  const gain=page.getByRole("textbox",{name:"gain",exact:true});
  await gain.fill("0");await gain.press("Enter");await frames();
  const green=await capture();assert.ok(!green.equals(red),"parameter edit changes canvas pixels");
  await page.locator("#preview-play").click();
  const paused=await capture();
  await page.locator("#preview-step-forward").click();
  await page.waitForFunction(()=>Number(document.querySelector("#preview-canvas").dataset.engineTime)>0);
  assert.ok((await capture()).equals(paused),"stepping a static canvas preserves pixels");
  // Exercise persisted lifecycle events explicitly: GPU-capable pages are not
  // guaranteed to be admitted to the browser's back/forward cache.
  for (let cycle = 0; cycle < 2; cycle++) {
    const savedTime = await canvas.getAttribute("data-engine-time");
    await page.evaluate(() => dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true })));
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const hiddenFrames = await canvas.getAttribute("data-engine-frames");
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.equal(await canvas.getAttribute("data-engine-frames"), hiddenFrames, "cached page suspends drawing");
    await page.evaluate(() => dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true })));
    await page.waitForFunction(before => Number(document.querySelector("#preview-canvas").dataset.engineFrames) > before, Number(hiddenFrames));
    assert.equal(await canvas.getAttribute("data-engine-time"), savedTime, "restored paused page retains simulation time");
    assert.equal(Number(await gain.inputValue()), 0, "restored page retains parameter edits");
    assert.ok((await capture()).equals(paused), "restored page retains rendered output");
  }

  await open("canvas probe(ctx: CanvasContext) -> color { param red: f32 = 0 in 0 .. 1; param green: f32 = 0 in 0 .. 1; rgba(red, green, 0, 1) }");
  const black=await capture();
  const editSliders=async values=>page.evaluate(values=>{
    for(const [name,value] of Object.entries(values)) {
      const input=document.querySelector(`input[aria-label="${name} slider"]`);
      input.value=String(value);input.dispatchEvent(new Event("input",{bubbles:true}));
    }
  },values);
  await editSliders({red:1,green:1});await frames();
  assert.ok(!(await capture()).equals(black),"both parameters affect the preview");
  await editSliders({red:0,green:0});await frames();
  assert.ok((await capture()).equals(black),"same-turn partial parameter edits both reach the GPU");

  await open("canvas probe(ctx: CanvasContext) -> color { param points: array<vec3> = [vec3(0,1,0)]; rgba(points[0].x, points[0].y, points[0].z, 1) }");
  const arrayGreen=await capture();
  const reflected=JSON.parse(await page.locator("#manifest").textContent());
  assert.deepEqual(reflected.canvases[0].params[0].default.values,[[0,1,0]],"manifest panel preserves structured WASM defaults");
  assert.equal(Number(await page.getByRole("textbox",{name:"points[0].1",exact:true}).inputValue()),1);
  const pointX=page.getByRole("textbox",{name:"points[0].0",exact:true});
  await pointX.fill("1");await pointX.press("Enter");await frames();
  assert.ok(!(await capture()).equals(arrayGreen),"vector storage edit reaches GPU");
  await pointX.fill("0");await pointX.press("Enter");await frames();
  assert.ok((await capture()).equals(arrayGreen),"vector storage UI round-trips engine values");
  await page.getByRole("button",{name:"Add element to points",exact:true}).click();await frames();
  assert.equal(await page.getByRole("textbox",{name:"points[1].0",exact:true}).count(),1);
  await page.getByRole("button",{name:"Remove points[1]",exact:true}).click();await frames();
  assert.ok((await capture()).equals(arrayGreen),"storage growth and shrink preserve original elements");

  await open('canvas probe(ctx: CanvasContext) -> color { param gain: f32 = 1 in 0 .. 1; [binding(default = "assets/textures/brick_tex.png")] uniform tex: texture; tex.at(ctx.uv) |> opacity(gain) }');
  const textureGain=page.getByRole("textbox",{name:"gain",exact:true});
  await textureGain.fill("0.5");await textureGain.press("Enter");await frames();
  const originalTexture=await capture();
  const textureSelect=page.locator(".param-row select");
  const initialTextureUrl=await textureSelect.inputValue();
  const alternateUrl=await textureSelect.locator("option").evaluateAll((options,initial)=>options.map(o=>o.value).find(value=>value!==initial && value.startsWith("/assets/")),initialTextureUrl);
  assert.ok(alternateUrl,"a second packaged texture is available");
  const blockedTexture=new URL(alternateUrl,base).href;
  await page.route(blockedTexture,route=>route.fulfill({status:404,body:"missing test texture"}));
  await textureSelect.selectOption(alternateUrl);
  await page.waitForFunction(()=>document.querySelector("#compile-status").textContent==="Preview Error");
  assert.equal(await textureSelect.inputValue(),initialTextureUrl,"failed texture selection rolls back");
  assert.equal(Number(await textureGain.inputValue()),0.5,"failed replacement preserves parameter controls");
  assert.ok((await capture()).equals(originalTexture),"failed texture load preserves pixels");
  await page.unroute(blockedTexture);
  const swapTexture=async url=>{
    const before=await canvas.getAttribute("data-engine-input-revision") || "";
    await textureSelect.selectOption(url);
    await page.waitForFunction(before=>(document.querySelector("#preview-canvas").dataset.engineInputRevision || "")!==before,before);
    await frames();
  };
  let releaseTexture;
  const textureGate=new Promise(resolve=>{releaseTexture=resolve;});
  let requestedTexture;
  const textureRequested=new Promise(resolve=>{requestedTexture=resolve;});
  await page.route(blockedTexture,async route=>{requestedTexture();await textureGate;await route.continue();});
  const replacementRevision=await canvas.getAttribute("data-engine-input-revision") || "";
  await textureSelect.selectOption(alternateUrl);
  await textureRequested;
  await textureGain.fill("0.25");await textureGain.press("Enter");
  releaseTexture();
  await page.waitForFunction(before=>(document.querySelector("#preview-canvas").dataset.engineInputRevision || "")!==before,replacementRevision);
  await frames();
  assert.equal(Number(await textureGain.inputValue()),0.25,"edit during texture preparation is retained in the control");
  await page.unroute(blockedTexture);
  await textureGain.fill("0.5");await textureGain.press("Enter");await frames();
  assert.ok(!(await capture()).equals(originalTexture),"replacement texture changes pixels");
  assert.equal(Number(await textureGain.inputValue()),0.5,"texture replacement preserves parameter edits");
  await swapTexture(initialTextureUrl);
  assert.ok((await capture()).equals(originalTexture),"restoring texture restores the edited preview");

  await open("surface probe(sp: surf) -> material(unlit) { param gain: f32 = 1 in 0 .. 1; compose { base(albedo: rgba(gain, 1 - gain, 0, 1)) } }");
  await page.evaluate(() => {
    const slider = document.querySelector('input[aria-label="gain slider"]');
    slider.value = "0"; slider.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector('[data-mesh="plane"]').click();
  });
  await page.waitForFunction(() => document.querySelector("#preview-canvas").dataset.engineMesh === "plane");
  await frames();
  assert.equal(Number(await gain.inputValue()), 0, "mesh replacement retains the preceding parameter edit");
  const editedPlane = await capture();
  await gain.fill("1"); await gain.press("Enter"); await frames();
  assert.ok(!(await capture()).equals(editedPlane), "retained edit reaches rendered pixels");
  await gain.fill("0"); await gain.press("Enter"); await frames();
  assert.ok((await capture()).equals(editedPlane), "queued edit matches the separately committed value");

  await open("surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #f00) } }");
  const sphere=await capture();
  await page.locator('[data-mesh="plane"]').click();await page.waitForFunction(()=>document.querySelector("#preview-canvas").dataset.engineMesh==="plane");await frames();
  const plane=await capture();
  assert.ok(!plane.equals(sphere),"mesh selector changes Rust geometry");
  assert.equal(await page.locator('[data-mesh="plane"]').getAttribute("aria-pressed"),"true");
  await page.evaluate(async()=>{
    const {BrowserEngine}=await import("/example-engine-renderer/fresco_example_engine_host.js");
    const install=BrowserEngine.prototype.install_with_assets;
    BrowserEngine.prototype.install_with_assets=function(...args){
      if(args[5]==="box") {
        BrowserEngine.prototype.install_with_assets=install;
        return Promise.reject(new Error("injected geometry preparation failure"));
      }
      return install.apply(this,args);
    };
  });
  await page.locator('[data-mesh="box"]').click();
  await page.waitForFunction(()=>document.querySelector("#compile-status").textContent==="Preview Error");
  assert.equal(await page.locator('[data-mesh="plane"]').getAttribute("aria-pressed"),"true","failed mesh keeps committed selection");
  assert.equal(await canvas.getAttribute("data-engine-mesh"),"plane");
  assert.ok((await capture()).equals(plane),"failed geometry preparation preserves pixels");
  await page.locator('[data-mesh="sphere"]').click();await page.waitForFunction(()=>document.querySelector("#preview-canvas").dataset.engineMesh==="sphere");await frames();
  const returned=await capture();
  assert.ok(returned.equals(sphere),"returning to sphere restores pixels");
  assert.equal(await page.locator('[data-mesh="sphere"]').getAttribute("aria-pressed"),"true");

  // Direct manipulation takes over from auto-spin and releases capture on up.
  await page.locator('[data-mesh="box"]').click();
  await page.waitForFunction(()=>document.querySelector("#preview-canvas").dataset.engineMesh==="box");
  await frames();
  const beforeOrbit=await capture();
  await page.locator("#preview-spin").click();
  assert.ok(await page.locator("#preview-spin").isHidden(),"spin is active");
  await canvas.evaluate(element=>element.addEventListener("pointerdown",event=>{
    window.cameraPointer=event.pointerId;
  },{once:true}));
  const bounds=await canvas.boundingBox();
  const cx=bounds.x+bounds.width/2, cy=bounds.y+bounds.height/2;
  await page.mouse.move(cx,cy);await page.mouse.down();
  assert.ok(await canvas.evaluate(element=>element.hasPointerCapture(window.cameraPointer)));
  await page.mouse.move(cx+45,cy+25,{steps:5});await page.mouse.up();await frames();
  assert.ok(await page.locator("#preview-spin").isVisible(),"dragging stops auto-spin");
  assert.ok(!await canvas.evaluate(element=>element.hasPointerCapture(window.cameraPointer)),"pointerup releases capture");
  assert.equal(await canvas.evaluate(element=>getComputedStyle(element).touchAction),"none","touch dragging is reserved for the camera");
  const afterOrbit=await capture();
  assert.ok(!afterOrbit.equals(beforeOrbit),"drag changes the mesh view");
  await page.mouse.move(cx-35,cy-20);await frames();
  assert.ok((await capture()).equals(afterOrbit),"hover after release does not move the camera");
  await page.mouse.move(cx,cy);await page.mouse.down();
  await canvas.evaluate(element=>element.dispatchEvent(new PointerEvent("pointercancel",{
    pointerId:window.cameraPointer,bubbles:true,
  })));
  assert.ok(!await canvas.evaluate(element=>element.hasPointerCapture(window.cameraPointer)),"pointercancel releases capture");
  await page.mouse.move(cx+30,cy-30);await page.mouse.up();await frames();
  assert.ok((await capture()).equals(afterOrbit),"cancelled drag cannot keep moving the camera");
  await page.mouse.wheel(0,180);await frames();
  assert.ok(!(await capture()).equals(afterOrbit),"wheel changes camera distance");

  await page.locator("#preview-rate").selectOption("-1");
  await page.locator("#example-select").selectOption("50) particles/drifting_sparks",{force:true});
  await page.waitForFunction(()=>document.querySelector("#compile-status").textContent.includes("Compiled"));await frames();
  await page.waitForFunction(()=>document.querySelector("#preview-canvas").dataset.engineEntry==="drifting_sparks");await frames();
  assert.equal(await page.locator("#preview-rate").inputValue(),"1","particle entry resets inherited reverse playback");
  assert.ok(await page.locator('#preview-rate option[value="-1"]').isDisabled());
  assert.ok(await page.locator("#preview-step-back").isDisabled());
  assert.ok(await page.locator("#mesh-picker").isHidden());
  assert.ok(await page.locator("#preview-spin").isHidden());
  await page.locator("#preview-play").click();
  const time=await page.locator("#preview-time-input").inputValue();
  const particles=await capture();
  const particleFrame=Number(await canvas.getAttribute("data-engine-frames"));
  await page.locator("#preview-step-forward").click();
  await page.waitForFunction(old=>document.querySelector("#preview-time-input").value!==old,time);
  await page.waitForFunction(n=>Number(document.querySelector("#preview-canvas").dataset.engineFrames)>n,particleFrame);
  assert.ok(!(await capture()).equals(particles),"paused particle step advances simulation");
  const input=page.locator("#preview-time-input");await input.fill("0");await input.press("Enter");
  await page.waitForFunction(()=>document.querySelector("#preview-canvas").dataset.engineTime==="0");
  await page.locator('[data-tab="inspector"]').click();
  await page.getByRole("button", { name: "engine/engine.fr", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".file-tab.active")?.getAttribute("title") === "engine/engine.fr");
  // Monaco paints the new model after the file-tab state has changed.
  await page.waitForFunction(() => document.querySelector("#editor")?.textContent.includes("core/01_core.fr"));
  assert.match(await page.locator("#editor").innerText(), /core\/01_core\.fr/);
  await open(`canvas probe(ctx: CanvasContext) -> color {
    let time = context(time)
    let value = sin(time) // @viz(time)
    rgba(value * 0.5 + 0.5, 0, 0, 1)
  }`);
  assert.ok(await page.locator('#preview-rate option[value="-1"]').isEnabled());
  assert.ok(await page.locator("#preview-step-back").isEnabled());
  await page.waitForFunction(()=>[...document.querySelectorAll("img.viz-canvas")]
    .some(image=>image.complete && image.naturalWidth>0),undefined,{timeout:30_000});

  // An editor-only device failure must not prevent the Rust engine from drawing.
  const isolated=await browser.newPage();
  await isolated.addInitScript(()=>{
    const requestDevice=GPUAdapter.prototype.requestDevice;
    GPUAdapter.prototype.requestDevice=function(options){
      if(options?.label==="Fresco editor visualizers") {
        window.visualizerFailureInjected=true;
        return Promise.reject(new Error("test visualizer device failure"));
      }
      return requestDevice.call(this,options);
    };
  });
  const productionUrl = new URL(page.url());
  productionUrl.searchParams.set("renderer", "typescript");
  await isolated.goto(productionUrl.href);
  await isolated.waitForFunction(()=>window.visualizerFailureInjected
    && Number(document.querySelector("#preview-canvas").dataset.engineFrames)>3);
  assert.ok(await isolated.locator("#preview-unavailable").isHidden());
  assert.equal(await isolated.locator("#preview-backend").count(), 0);
  await isolated.close();
  assert.deepEqual(errors,[]);
  console.log("Playground Rust engine checks passed: canvas parameters and arrays, texture replacement/recovery, mesh selection, particle playback, stepping, reset, inline visualizers, and visualizer failure isolation.");
} finally {clearTimeout(deadline);await browser.close();}
