// Local-only Chrome/WebGPU checks; excluded from CI.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
const require=createRequire(new URL('../../../crates/fresco-wasm/web/package.json',import.meta.url));
const {chromium}=require('@playwright/test');
const source=readFileSync(new URL('../../../examples/50) particles/drifting_sparks.fr',import.meta.url),'utf8')
  .replace('spawn_rate: 60.0','spawn_rate: 4.0\n    allocation: ParticleAllocationMode.Automatic\n    allocation_hint: 1\n    max_particles: 32')
  .replace('particle_fountain(vec3(0.0, -0.65, 0.0), 0.35, 1.0, id)','particle_fountain(vec3(0.0, -0.65, 0.0), 0.35, 1.0, id)\n        particle_fade_size(0.3)')
  .replace('particle_fade_size(0.055)','particle_fade_size(0.3)')
  .replace('    let radius =','    param tint: color = #fff\n    let radius =')
  .replace('rgba(1.0, 0.15 + 0.75 * core, 0.03 + 0.35 * core, 1.0)','tint');
const browser=await chromium.launch({channel:process.env.FRESCO_GPU_BROWSER??'chrome',headless:true});
const deadline=setTimeout(()=>browser.close().catch(console.error),180_000);
try {
  const page=await browser.newPage({viewport:{width:1100,height:800},deviceScaleFactor:1});
  const errors=[]; page.on('pageerror',error=>errors.push(String(error)));
  await page.goto(process.env.FRESCO_ENGINE_URL??'http://127.0.0.1:5182');
  const status=page.locator('#status'),canvas=page.locator('canvas');
  await status.filter({hasText:'Rendering demo'}).waitFor({timeout:120_000});
  await page.locator('#pause').click();
  const frames=async()=>{
    const before=Number(await canvas.getAttribute('data-frames'));
    await page.waitForFunction(n=>Number(document.querySelector('canvas').dataset.frames)>n+5,before);
  };
  await page.locator('#source').fill(source); await page.locator('#compile').click();
  await status.filter({hasText:'Rendering drifting_sparks'}).waitFor().catch(async error=>{console.error(await page.locator('#diagnostics').innerText());throw error;}); await frames();
  const initial=await canvas.screenshot();
  const visible=await page.evaluate(async encoded=>{
    const bitmap=await createImageBitmap(await(await fetch('data:image/png;base64,'+encoded)).blob());
    const copy=new OffscreenCanvas(bitmap.width,bitmap.height),ctx=copy.getContext('2d');
    ctx.drawImage(bitmap,0,0);
    const data=ctx.getImageData(0,0,copy.width,copy.height).data;
    let white=0;for(let i=0;i<data.length;i+=4)if(data[i]>200&&data[i+1]>200&&data[i+2]>200)white++;
    return white;
  },initial.toString('base64'));
  assert.ok(visible>100,'authored particles render after automatic growth');
  await frames();assert.ok((await canvas.screenshot()).equals(initial),'pause preserves particles');
  await page.locator('#pause').click();await frames();await page.locator('#pause').click();await frames();
  assert.ok(!(await canvas.screenshot()).equals(initial),'simulation advances');
  await page.locator('#reset').click();await frames();
  assert.ok((await canvas.screenshot()).equals(initial),'reset restores initial spawn');
  await page.locator('#parameters').fill('{"tint":[0,1,0,1]}');await page.locator('#apply-parameters').click();
  await status.filter({hasText:'Rendering drifting_sparks'}).waitFor();await frames();
  const green=await canvas.screenshot();assert.ok(!green.equals(initial),'particle material edits change pixels');
  await page.locator('#rebuild').click();await status.filter({hasText:'Rendering drifting_sparks'}).waitFor();await frames();
  assert.ok((await canvas.screenshot()).equals(green),'GPU rebuild restores material and restarts particles');
  await page.locator('#source').fill('invalid source');await page.locator('#compile').click();
  await status.filter({hasText:'Failed; keeping'}).waitFor();await frames();
  assert.ok((await canvas.screenshot()).equals(green),'failed compile preserves particles');
  const lifecycle=await page.evaluate(async source=>{
    const {BrowserEngine}=await import('./renderer/fresco_example_engine_host.js');
    const worker=new Worker('./compiler-worker.js',{type:'module'});
    const result=await new Promise((resolve,reject)=>{
      worker.onerror=event=>reject(Error(event.message));
      worker.onmessage=({data})=>{
        if(data.ready)worker.postMessage({id:1,source});
        else if(data.error)reject(Error(data.error));
        else resolve(data.result);
      };
    });
    worker.terminate();
    if(!result.ok)throw Error(JSON.stringify(result.diagnostics));
    const manifest=JSON.stringify(result.manifest,(_,v)=>v instanceof Map?Object.fromEntries(v):v);
    const target=document.createElement('canvas');target.width=target.height=64;
    const engine=await BrowserEngine.create(target);
    try {
      const install=()=>engine.install(result.wgsl,manifest,'drifting_sparks');
      await install();
      const pendingReset=engine.render_async(0,0);engine.reset_playback();
      const resetCanceled=await pendingReset;
      const first=await engine.render_async(0,0);
      const baseline=target.toDataURL();
      engine.reset_playback();
      const pendingCancel=engine.render_async(1,1);engine.cancel_pending();
      const canceled=await pendingCancel;
      await engine.render_async(0,0);
      const preserved=target.toDataURL()===baseline;
      engine.reset_playback();
      const earlier=engine.render_async(0.5,0.5);
      const later=engine.render_async(0,0);
      const frameOrder=await Promise.all([earlier,later]);
      const latestPreserved=target.toDataURL()===baseline;
      engine.reset_playback();
      const pendingResize=engine.render_async(1,1);
      const resizing=engine.resize(96,32);
      const resizeOrder=await Promise.all([pendingResize,resizing]);
      const resized=await engine.render_async(0,0);
      const size=[target.width,target.height];
      await engine.resize(0,0);const zero=await engine.render_async(0,0);
      await engine.resize(64,64);const restored=await engine.render_async(0,0);
      engine.reset_playback();
      const pendingReplace=engine.render_async(1,1);const replacing=install();
      const replaceOrder=await Promise.all([pendingReplace,replacing]);
      const replacement=await engine.render_async(0,0);
      const replacementPreserved=target.toDataURL()===baseline;
      return {resetCanceled,first,canceled,preserved,frameOrder,latestPreserved,resizeOrder,resized,size,zero,restored,replaceOrder,replacement,replacementPreserved};
    } finally {engine.cancel_pending();engine.free();}
  },source);
  assert.deepEqual(lifecycle,{resetCanceled:false,first:true,canceled:false,preserved:true,frameOrder:[false,true],latestPreserved:true,resizeOrder:[false,true],resized:true,size:[96,32],zero:false,restored:true,replaceOrder:[false,true],replacement:true,replacementPreserved:true});
  assert.deepEqual(errors,[]);
  console.log('Browser particle GPU checks passed: automatic growth, visible authored stages, simulation, pause/reset, material edits, rebuild, failed compilation, reset/cancel during growth, concurrent frames, resize, zero size, and replacement races.');
} finally {clearTimeout(deadline);await browser.close();}
