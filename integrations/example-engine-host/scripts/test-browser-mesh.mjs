// Local-only WebGPU checks for the shared Rust mesh host; never run in CI.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../../../crates/fresco-wasm/web/package.json', import.meta.url));
const { chromium } = require('@playwright/test');
const material = readFileSync(new URL('../../example-engine/examples/material_parameters.fr', import.meta.url), 'utf8');
const textured = readFileSync(new URL('../../example-engine/examples/material_texture.fr', import.meta.url), 'utf8');
const browser = await chromium.launch({ channel: process.env.FRESCO_GPU_BROWSER ?? 'chrome', headless: true });
const deadline = setTimeout(() => browser.close().catch(console.error), 180_000);
try {
  const page = await browser.newPage({ viewport: { width:1100, height:800 }, deviceScaleFactor:1 });
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(process.env.FRESCO_ENGINE_URL ?? 'http://127.0.0.1:5182');
  const status = page.locator('#status');
  const canvas = page.locator('canvas');
  await status.filter({ hasText:'Rendering demo' }).waitFor({ timeout:120_000 });
  await page.locator('#pause').click();
  const frames = async () => {
    const before = Number(await canvas.getAttribute('data-frames'));
    await page.waitForFunction(n => Number(document.querySelector('canvas').dataset.frames) > n + 2, before);
  };
  const compile = async source => {
    await page.locator('#source').fill(source);
    await page.locator('#compile').click();
  };
  await compile(material);
  await status.filter({ hasText:'Rendering adjustable' }).waitFor();
  await frames();
  const red = await canvas.screenshot();
  const pixels = await page.evaluate(async encoded => {
    const bitmap = await createImageBitmap(await (await fetch('data:image/png;base64,' + encoded)).blob());
    const copy = new OffscreenCanvas(bitmap.width,bitmap.height);
    const ctx = copy.getContext('2d'); ctx.drawImage(bitmap,0,0);
    const data = ctx.getImageData(0,0,copy.width,copy.height).data;
    let red=0, clear=0;
    for(let i=0;i<data.length;i+=4) { if(data[i]>240 && data[i+1]<10 && data[i+2]<10 && data[i+3]===255)red++; if(data[i]<10 && data[i+1]<10 && data[i+2]<10)clear++; }
    return {red,clear,total:data.length/4};
  }, red.toString('base64'));
  assert.ok(pixels.red > pixels.total*0.1 && pixels.clear > pixels.total*0.1, 'mesh sphere and background both reach the browser canvas');
  await page.locator('#parameters').fill('{"tint":[0,1,0,1]}');
  await page.locator('#apply-parameters').click();
  await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
  const green = await canvas.screenshot(); assert.ok(!red.equals(green),'material edits change visible pixels');
  const shapes = [];
  for (const shape of ['plane', 'box']) {
    await page.locator('#mesh').selectOption(shape);
    await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
    const pixels = await canvas.screenshot();
    assert.ok(!pixels.equals(green), `${shape} changes visible geometry`);
    assert.deepEqual(JSON.parse(await page.locator('#parameters').inputValue()).tint, [0,1,0,1], 'mesh selection preserves material edits');
    shapes.push(pixels);
    await page.locator('#rebuild').click();
    await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
    assert.ok((await canvas.screenshot()).equals(pixels), `${shape} survives GPU recreation`);
  }
  assert.ok(!shapes[0].equals(shapes[1]), 'plane and box have different silhouettes');
  const bounds = await canvas.boundingBox();
  const center = {x:bounds.x+bounds.width/2,y:bounds.y+bounds.height/2};
  await page.mouse.move(center.x,center.y);
  await page.mouse.down();
  await page.mouse.move(center.x+60,center.y+30,{steps:5});
  await page.mouse.up(); await frames();
  const orbited = await canvas.screenshot();
  assert.ok(!orbited.equals(shapes[1]), 'drag changes the box view');
  await page.mouse.wheel(0,200); await frames();
  const zoomed = await canvas.screenshot();
  assert.ok(!zoomed.equals(orbited), 'wheel changes camera distance');
  await page.locator('#rebuild').click();
  await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(zoomed), 'camera survives GPU recreation');
  await page.locator('#reset-camera').click(); await frames();
  assert.ok((await canvas.screenshot()).equals(shapes[1]), 'reset restores the initial box view');
  await page.locator('#mesh').selectOption('sphere');
  await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(green), 'returning to sphere preserves its material');
  await page.locator('#parameters').fill('{"gain":0,"tint":[1,2,3]}');
  await page.locator('#apply-parameters').click();
  await status.filter({hasText:'Failed; keeping'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(green),'failed material batches preserve pixels');
  await page.locator('#rebuild').click();
  await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(green),'GPU recreation restores material edits');
  await compile('canvas swapped(ctx: CanvasContext) -> color { rgba(1,0,0,1) }');
  await status.filter({hasText:'Rendering swapped'}).waitFor(); await frames();
  assert.ok(!(await canvas.screenshot()).equals(red),'canvas replacement covers the sphere background');
  await compile(material);
  await status.filter({hasText:'Rendering adjustable'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(red),'switching back installs mesh defaults');
  await compile(textured);
  await status.filter({hasText:'Failed; keeping'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(red),'missing material image keeps the previous entry');
  await page.locator('#texture-file').setInputFiles({name:'checker.png',mimeType:'image/png',buffer:readFileSync(new URL('../../example-engine/assets/checker.png',import.meta.url))});
  await status.filter({hasText:'Rendering textured'}).waitFor(); await frames();
  const original = await canvas.screenshot(); assert.ok(!original.equals(red));
  await page.locator('#parameters').fill('{"strength":0.5}');
  await page.locator('#apply-parameters').click();
  await status.filter({hasText:'Rendering textured'}).waitFor(); await frames();
  const edited = await canvas.screenshot(); assert.ok(!edited.equals(original));
  await page.locator('#rebuild').click();
  await status.filter({hasText:'Rendering textured'}).waitFor(); await frames();
  assert.ok((await canvas.screenshot()).equals(edited),'mesh textures and parameter edits survive GPU recreation');
  const lifecycle = await page.evaluate(async source => {
    const {BrowserEngine} = await import('./renderer/fresco_example_engine_host.js');
    const worker = new Worker('./compiler-worker.js',{type:'module'});
    const result = await new Promise((resolve,reject) => {
      worker.onerror = event => reject(Error(event.message));
      worker.onmessage = ({data}) => {
        if(data.ready)worker.postMessage({id:1,source});
        else if(data.error)reject(Error(data.error));
        else resolve(data.result);
      };
    });
    worker.terminate();
    if(!result.ok)throw Error(JSON.stringify(result.diagnostics));
    const manifest=JSON.stringify(result.manifest,(_,v)=>v instanceof Map?Object.fromEntries(v):v);
    const target=document.createElement('canvas'); target.width=target.height=64;
    const engine=await BrowserEngine.create(target);
    try {
      const installed=await engine.install(result.wgsl,manifest,'adjustable');
      const first=engine.render(0,0);
      const pending=engine.install(result.wgsl,manifest,'adjustable');
      const during=engine.render(0,0); engine.cancel_pending(); const canceled=await pending;
      const earlier=engine.resize(80,64); const later=engine.resize(96,32);
      const resizeOrder=await Promise.all([earlier,later]);
      const resized=engine.render(0,0); const size=[target.width,target.height];
      await engine.resize(0,0); const zero=engine.render(0,0);
      await engine.resize(64,64); const restored=engine.render(0,0);
      const baseline=target.toDataURL();
      const unavailable = JSON.parse(manifest);
      unavailable.surfaces[0].surface_requirements.uv_channels.push({
        selector: "uv3", stream_index: 2, semantic: "TEXCOORD2",
        components: 2, required: true, status: "required",
      });
      let missingUv = "";
      try { await engine.install(result.wgsl, JSON.stringify(unavailable), "adjustable"); }
      catch (error) { missingUv = String(error); }
      if (!missingUv.includes("missing required UV streams") || !missingUv.includes("uv3")
        || !missingUv.includes("surface_requirements.uv_channels")) throw Error(`Unexpected UV diagnostic: ${missingUv}`);
      engine.render(0,0);
      if (target.toDataURL() !== baseline) throw Error("Rejected UV requirement changed installed pixels");

      let rejected=false; try{await engine.install('invalid shader',manifest,'adjustable');}catch{rejected=true;}
      engine.render(0,0); const preserved=target.toDataURL()===baseline;
      const old=engine.update_parameters('{"gain":0.25}'); const latest=engine.update_parameters('{"gain":0.75}');
      const parameterOrder=await Promise.all([old,latest]);
      const values=JSON.parse(engine.parameter_values_json());
      const replacing=engine.install(result.wgsl,manifest,'adjustable');
      const resizing=engine.resize(48,32); await Promise.all([replacing,resizing]);
      const afterRace=engine.render(0,0); const finalSize=[target.width,target.height];
      return {installed,first,during,canceled,resizeOrder,resized,size,zero,restored,rejected,preserved,parameterOrder,gain:values.gain,afterRace,finalSize};
    } finally {engine.cancel_pending();engine.free();}
  }, material);
  assert.deepEqual(lifecycle,{installed:true,first:true,during:true,canceled:false,resizeOrder:[false,true],resized:true,size:[96,32],zero:false,restored:true,rejected:true,preserved:true,parameterOrder:[false,true],gain:0.75,afterRace:true,finalSize:[48,32]});
  assert.deepEqual(errors,[],'no uncaught browser errors');
  console.log('Browser mesh GPU checks passed: sphere, parameters, texture upload, replacement/rebuild, cancellation, concurrent resize, zero size, failed shader, and resize/install races.');
} finally {clearTimeout(deadline);await browser.close();}
