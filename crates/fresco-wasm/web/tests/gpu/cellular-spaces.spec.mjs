import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";

function random(x, y, seed) {
  let h = (x ^ Math.imul(y, 0x9e3779b9) ^ seed) >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x7feb352d) >>> 0;
  h = Math.imul(h ^ (h >>> 15), 0x846ca68b) >>> 0;
  h = (h ^ (h >>> 16)) >>> 0;
  return (h >>> 8) / 16777216;
}

function reference(layout, seed) {
  const sites = new Map();
  function site(x, y) {
    const key = `${x},${y}`;
    if (!sites.has(key)) {
      const stagger = layout === "brick" || layout === "hex" ? ((y % 2) + 2) % 2 * 0.5 : 0;
      const irregular = layout === "jittered" || layout === "voronoi";
      sites.set(key, {
        x: x + 0.5 + stagger + (irregular ? random(x, y, seed) - 0.5 : 0),
        y: (y + 0.5) * (layout === "hex" ? Math.sqrt(3) / 2 : 1)
          + (irregular ? random(x, y, seed ^ 0xa511e9b3) - 0.5 : 0),
        id: [x, y], rand: random(x, y, seed),
      });
    }
    return sites.get(key);
  }
  return (x, y) => {
    let iy = Math.floor(y / (layout === "hex" ? Math.sqrt(3) / 2 : 1));
    let ix = Math.floor(x - (layout === "brick" ? ((iy % 2) + 2) % 2 * 0.5 : 0));
    if (layout !== "voronoi" && layout !== "hex") return site(ix, iy);
    let best;
    let distance = Infinity;
    // Deliberately wider than the compiler's 5x5 candidate window.
    for (let row = iy - 4; row <= iy + 4; row++) {
      for (let col = ix - 4; col <= ix + 4; col++) {
        const candidate = site(col, row);
        const d = (x - candidate.x) ** 2 + (y - candidate.y) ** 2;
        if (d < distance) { distance = d; best = candidate; }
      }
    }
    return best;
  };
}

// Independent half-plane reference for normalized geometry. Candidate range is
// wider than the compiler's face query, and pixel offsets use explicit Jacobians.
function geometryReference(layout, owner, qx, qy, seed, width, height, rotated) {
  const px = qx - owner.x, py = qy - owner.y;
  let planes;
  if (layout === "hex") {
    planes = Array.from({length: 6}, (_, i) => [Math.cos(i * Math.PI / 3), Math.sin(i * Math.PI / 3), 0.5]);
  } else if (layout === "voronoi") {
    planes = [];
    for (let y = owner.id[1] - 4; y <= owner.id[1] + 4; y++) for (let x = owner.id[0] - 4; x <= owner.id[0] + 4; x++) {
      const dx = x + random(x, y, seed) - owner.x;
      const dy = y + random(x, y, seed ^ 0xa511e9b3) - owner.y;
      const d = Math.hypot(dx, dy);
      if (d > 1e-8) planes.push([dx / d, dy / d, d / 2]);
    }
  } else {
    const cx = layout === "jittered" ? owner.id[0] + 0.5 - owner.x : 0;
    const cy = layout === "jittered" ? owner.id[1] + 0.5 - owner.y : 0;
    planes = [[1,0,0.5+cx],[-1,0,0.5-cx],[0,1,0.5+cy],[0,-1,0.5-cy]];
  }
  let edge = Infinity, inset = Infinity, radius = Infinity;
  const angle = Math.PI / 6;
  for (const [nx, ny, h] of planes) {
    const distance = h - nx * px - ny * py;
    const pixel = rotated ? Math.hypot(ny / (width * 0.13), nx / (height * 0.17)) : Math.hypot(nx / (width * 0.17), ny / (height * 0.13));
    edge = Math.min(edge, distance);
    inset = Math.min(inset, distance - 2 * pixel);
    const dot = nx * Math.cos(angle) + ny * Math.sin(angle);
    if (dot > 1e-8) radius = Math.min(radius, h / dot);
  }
  return [0.1 + 0.3 * edge, 0.5 + 0.2 * radius * Math.cos(angle), 0.1 + 0.3 * Math.abs(inset)];
}

for (const layout of ["square", "brick", "hex", "jittered", "voronoi"]) for (const rotated of [false, true]) {
  test(`cell geometry ${layout} rotated=${rotated} matches distances, ray intersection and 2px inset`, async ({page}) => {
    const seed = 29;
    const jitter = ["jittered", "voronoi"].includes(layout) ? ", jitter: 1.0" : "";
    const program = `canvas geometry(ctx: CanvasContext) -> color {
      in space translate(by: (0.63, 0.47)) ${rotated ? ".rotate(90deg, around: (0,0))" : ""}
        .cells(layout: ${layout}, every: (0.17, 0.13), seed: ${seed}, sampling: center${jitter}, cell: tile) {
          let point = tile.boundary_point(angle: 30deg)
          fill(rgba(0.1 + 0.3 * tile.edge_distance, 0.5 + 0.2 * (point.x / 0.17 - 0.5), 0.1 + 0.3 * tile.inset_distance(by: 2px), 1))
        }
    }`;
    await page.goto("/tests/gpu/");
    await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(source => window.gpuTest.init("flip-card", source), program);
    const ownerAt = reference(layout, seed);
    for (const [width, height] of [[80,72], [144,96]]) {
      await page.evaluate(({width,height}) => window.gpuTest.frame("geometry",0,0,width,height), {width,height});
      const pixels = await page.evaluate(() => window.gpuTest.pixels("geometry"));
      let total = 0, maximum = 0;
      for (let y=0; y<height; y++) for (let x=0; x<width; x++) {
        const px=(x+0.5)/width-0.63, py=1-(y+0.5)/height-0.47;
        const qx=(rotated ? -py : px)/0.17, qy=(rotated ? px : py)/0.13;
        const expected=geometryReference(layout,ownerAt(qx,qy),qx,qy,seed,width,height,rotated);
        for(let c=0;c<3;c++) {
          const error=Math.abs(pixels[(y*width+x)*4+c]-Math.max(0,Math.min(1,expected[c]))*255);
          total+=error; maximum=Math.max(maximum,error);
        }
      }
      expect(total/(width*height*3)).toBeLessThan(0.7);
      expect(maximum).toBeLessThan(3);
    }
  });
}

test("cell geometry hex gallery preserves its original masks", async ({page}, testInfo) => {
  const program = readFileSync(new URL("../../../../../tests/fixtures/hex_circuit_geometry.fr", import.meta.url), "utf8");
  const reference = program
    .replace("tile.local", "(tile.uv - (0.5, 0.5))")
    .replace("tile.inset_distance(by: 0.037)", "abs(max(abs(local.x), 0.5 * abs(local.x) + 0.8660254 * abs(local.y)) - 0.463)")
    .replace("tile.inset_distance(by: 0.105)", "abs(max(abs(local.x), 0.5 * abs(local.x) + 0.8660254 * abs(local.y)) - 0.395)")
    .replace("tile.angle / 1turn", "atan2(local.y, local.x) / 6.2831853");
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  for (const [name, source] of [["actual",program],["reference",reference]]) {
    await page.evaluate(source => window.gpuTest.init("flip-card", source), source);
    await page.evaluate(name => window.gpuTest.frame(name,0.7,0,640,480), name);
  }
  const comparison=await page.evaluate(() => window.gpuTest.compare("actual","reference"));
  expect(comparison.maxDifference).toBeLessThanOrEqual(1);
  const png=await page.evaluate(() => window.gpuTest.png("actual"));
  await testInfo.attach("hex-circuit", {body:Buffer.from(png.split(",")[1],"base64"),contentType:"image/png"});
});

function source(layout, seed, samples = 16, shape = false, rotated = false) {
  const sampling = { 1: "center", 4: "grid2x2", 9: "grid3x3", 16: "grid4x4" }[samples];
  const jitter = ["voronoi", "jittered"].includes(layout) ? ", jitter: 1.0" : "";
  const blue = layout === "jittered" ? "tile.rand * 0.5 + (tile.uv.x + tile.uv.y) * 0.125 + 0.125" : "tile.rand";
  const color = `rgba((tile.id.x + 8.0) / 16.0, (tile.id.y + 8.0) / 16.0, ${blue}, 1.0)`;
  return `canvas cells_probe(ctx: CanvasContext) -> color {
    compose {
      in space translate(by: (0.63, 0.47)) ${rotated ? ". rotate(90deg, around: (0.0, 0.0))" : ""} . cells(layout: ${layout}, every: (0.17, 0.13), seed: ${seed}, sampling: ${sampling}${jitter}, cell: tile) {
        ${shape ? `circle(at: tile.center, radius: 0.052) |> fill(${color})` : `fill(${color})`}
      }
    }
  }`;
}

for (const samples of [1, 4, 9, 16]) for (const [layout, rotated, seed] of [
  ...["square", "brick", "hex", "jittered", "voronoi"].map(layout => [layout, false, 17]),
  ["voronoi", true, 91],
]) {
  test(`${layout}${rotated ? " rotated" : ""} ${samples} samples match independent ownership and boundary integration`, async ({ page }, testInfo) => {
    test.setTimeout(120_000);
    await page.goto("/tests/gpu/");
    await page.waitForFunction(() => window.gpuTest);
    const adapter = await page.evaluate(source => window.gpuTest.init("flip-card", source), source(layout, seed, samples, false, rotated));
    await testInfo.attach("adapter", { body: JSON.stringify(adapter), contentType: "application/json" });
    const width = 80, height = 72;
    const axis = Math.sqrt(samples);
    await page.evaluate(({ width, height }) => window.gpuTest.frame("cells", 0, 0, width, height), { width, height });
    const pixels = await page.evaluate(() => window.gpuTest.pixels("cells"));
    const evaluate = reference(layout, seed);
    let error = 0, maxError = 0;
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const expected = [0, 0, 0];
        for (let sy = 0; sy < axis; sy++) {
          for (let sx = 0; sx < axis; sx++) {
            const px = (x + (sx + 0.5) / axis) / width - 0.63;
            const py = 1 - (y + (sy + 0.5) / axis) / height - 0.47;
            const qx = (rotated ? -py : px) / 0.17;
            const qy = (rotated ? px : py) / 0.13;
            const cell = evaluate(qx, qy);
            const blue = layout === "jittered"
              ? cell.rand * 0.5 + (qx - cell.x + qy - cell.y + 1) * 0.125 + 0.125
              : cell.rand;
            const color = [(cell.id[0] + 8) / 16, (cell.id[1] + 8) / 16, blue];
            for (let c = 0; c < 3; c++) expected[c] += color[c] * 255 / samples;
          }
        }
        for (let c = 0; c < 3; c++) {
          const difference = Math.abs(pixels[(y * width + x) * 4 + c] - expected[c]);
          error += difference;
          maxError = Math.max(maxError, difference);
        }
        expect(pixels[(y * width + x) * 4 + 3]).toBe(255);
      }
    }
    const meanError = error / (width * height * 3);
    await testInfo.attach("numeric-reference", { body: JSON.stringify({ meanError, maxError }), contentType: "application/json" });
    const png = await page.evaluate(() => window.gpuTest.png("cells"));
    await testInfo.attach(layout, { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
    expect(meanError).toBeLessThan(0.7);
    expect(maxError).toBeLessThan(3);
    await page.evaluate(({ width, height }) => window.gpuTest.frame("again", 2, 0, width, height), { width, height });
    expect(await page.evaluate(() => window.gpuTest.compare("cells", "again"))).toEqual({ changedFraction: 0, maxDifference: 0 });
  });
}

test("cellular shape coverage converges to a higher-resolution reference", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(source => window.gpuTest.init("flip-card", source), source("voronoi", 23, 16, true));
  await page.evaluate(async () => {
    await window.gpuTest.frame("native", 0, 0, 160, 144);
    await window.gpuTest.frame("high", 0, 0, 640, 576);
    window.gpuTest.downsample("reference", "high", 4);
  });
  const [native, reference] = await page.evaluate(() => [window.gpuTest.pixels("native"), window.gpuTest.pixels("reference")]);
  let error = 0;
  for (let i = 0; i < native.length; i += 4) {
    for (let c = 0; c < 3; c++) error += Math.abs(native[i + c] - reference[i + c]);
  }
  const meanError = error / (native.length / 4 * 3);
  await testInfo.attach("coverage-error", { body: JSON.stringify({ meanError }), contentType: "application/json" });
  expect(meanError).toBeLessThan(2);
  for (const name of ["native", "reference"]) {
    const png = await page.evaluate(name => window.gpuTest.png(name), name);
    await testInfo.attach(name, { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
  }
});


test("cellular example renders its transformed local centers", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const program = readFileSync(new URL("../../../../../examples/20) techniques/cellular_voronoi.fr", import.meta.url), "utf8");
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(source => window.gpuTest.init("flip-card", source), program);
  await page.evaluate(() => window.gpuTest.frame("example", 0, 0, 480, 360));
  const pixels = await page.evaluate(() => window.gpuTest.pixels("example"));
  let bright = 0;
  for (let i = 0; i < pixels.length; i += 4) if (pixels[i + 1] > 150) bright++;
  expect(bright / (pixels.length / 4)).toBeGreaterThan(0.1);
  expect(bright / (pixels.length / 4)).toBeLessThan(0.7);
  const png = await page.evaluate(() => window.gpuTest.png("example"));
  await testInfo.attach("cellular-voronoi-example", { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
});

test("cellular polka gallery renders all panels and preserves animation", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const program = readFileSync(new URL("../../../../../examples/90) gallery/cellular_polka.fr", import.meta.url), "utf8");
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(source => window.gpuTest.init("flip-card", source), program);
  await page.evaluate(async () => {
    await window.gpuTest.frame("still", 0, 0, 480, 360);
    await window.gpuTest.frame("moving", 0.6, 0, 480, 360);
    await window.gpuTest.frame("again", 0, 0, 480, 360);
  });
  const pixels = await page.evaluate(() => window.gpuTest.pixels("still"));
  const bright = [0, 0, 0];
  for (let y = 0; y < 360; y++) {
    for (let x = 0; x < 480; x++) {
      if (pixels[(y * 480 + x) * 4 + 1] > 150) bright[Math.floor(x / 160)]++;
    }
  }
  for (const count of bright) expect(count / (160 * 360)).toBeGreaterThan(0.08);
  expect((await page.evaluate(() => window.gpuTest.compare("still", "moving"))).changedFraction).toBeGreaterThan(0.01);
  expect(await page.evaluate(() => window.gpuTest.compare("still", "again"))).toEqual({ changedFraction: 0, maxDifference: 0 });
  const png = await page.evaluate(() => window.gpuTest.png("still"));
  await testInfo.attach("cellular-polka", { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
});

// Independent polygon clipping reference for the new arc-length API.
function contourReference(layout, owner, seed, width, height, rotated, inset = 0.04) {
  let planes;
  if (layout === "voronoi") {
    planes=[];
    for(let y=owner.id[1]-4;y<=owner.id[1]+4;y++) for(let x=owner.id[0]-4;x<=owner.id[0]+4;x++) {
      const dx=x+random(x,y,seed)-owner.x, dy=y+random(x,y,seed^0xa511e9b3)-owner.y;
      const d=Math.hypot(dx,dy);
      if(d>1e-10) planes.push([dx/d,dy/d,d/2-inset]);
    }
  } else {
    const n=layout==="hex"?6:4;
    const cx=layout==="jittered"?owner.id[0]+0.5-owner.x:0;
    const cy=layout==="jittered"?owner.id[1]+0.5-owner.y:0;
    planes=Array.from({length:n},(_,i)=>{ const a=i*2*Math.PI/n, x=Math.cos(a),y=Math.sin(a); return [x,y,0.5+x*cx+y*cy-inset]; });
  }
  let polygon=[[-3,-3],[3,-3],[3,3],[-3,3]];
  for(const [nx,ny,h] of planes) {
    const result=[];
    for(let i=0;i<polygon.length;i++) {
      const a=polygon[i], b=polygon[(i+1)%polygon.length];
      const da=h-nx*a[0]-ny*a[1], db=h-nx*b[0]-ny*b[1];
      if(da>=-1e-10) result.push(a);
      if((da>0)!==(db>0)) { const t=da/(da-db); result.push([a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1])]); }
    }
    polygon=result;
  }
  const toPixels=([x,y])=>rotated?[y*0.13*width,-x*0.17*height]:[x*0.17*width,y*0.13*height];
  const edges=polygon.map((a,i)=>{
    const b=polygon[(i+1)%polygon.length], dx=b[0]-a[0],dy=b[1]-a[1];
    let angle=Math.atan2(-dx,dy); if(Math.abs(angle)<1e-8) angle=0; if(angle<0) angle+=2*Math.PI;
    return {a,b,angle};
  }).filter(e=>Math.hypot(e.b[0]-e.a[0],e.b[1]-e.a[1])>1e-8).sort((a,b)=>a.angle-b.angle);
  return (point,pixel=false)=> {
    point=pixel?toPixels(point):point;
    let best=Infinity, position=0,total=0;
    for(const e of edges) {
      const a=pixel?toPixels(e.a):e.a,b=pixel?toPixels(e.b):e.b;
      const dx=b[0]-a[0],dy=b[1]-a[1],len=Math.hypot(dx,dy);
      const t=Math.max(0,Math.min(1,((point[0]-a[0])*dx+(point[1]-a[1])*dy)/(len*len)));
      const distance=Math.hypot(point[0]-a[0]-t*dx,point[1]-a[1]-t*dy);
      if(distance<best-1e-9) {best=distance;position=total+t*len;}
      total+=len;
    }
    return {distance:best,progress:position/total%1,length:total};
  };
}

for(const layout of ["square","brick","hex","jittered","voronoi"]) for(const rotated of [false,true]) {
  test(`contour ${layout} rotated=${rotated} has real arc length and pixel-speed pulses`,async({page})=>{
    const seed=29, width=64,height=48;
    const jitter=["jittered","voronoi"].includes(layout)?", jitter: 1":"";
    const source=`canvas t(ctx: CanvasContext) -> color {
      in space translate(by: (0.63,0.47)) ${rotated?".rotate(90deg, around: (0,0))":""}
      .cells(layout: ${layout}, every: (0.17,0.13), seed: ${seed}, sampling: center${jitter}, cell: tile) {
        let track=tile.contour(inset: 0.04)
        let pulse=chase(along: track, speed: 20px/s, tail: 3px, direction: counterclockwise)
        fill(rgb(track.progress, track.length * 0.1, pulse * band(track.distance, width: 4px, profile: soft)))
      }
    }`;
    await page.goto("/tests/gpu/");await page.waitForFunction(()=>window.gpuTest);
    await page.evaluate(source=>window.gpuTest.init("flip-card",source),source);
    const ownerAt=reference(layout,seed),cache=new Map();
    for(const time of [0,0.7]) {
      await page.evaluate(({time,width,height})=>window.gpuTest.frame("contour",time,0,width,height),{time,width,height});
      const pixels=await page.evaluate(()=>window.gpuTest.pixels("contour"));
      let sum=0,bad=0;
      for(let y=0;y<height;y++)for(let x=0;x<width;x++) {
        const px=(x+0.5)/width-0.63,py=1-(y+0.5)/height-0.47;
        const qx=(rotated?-py:px)/0.17,qy=(rotated?px:py)/0.13;
        const owner=ownerAt(qx,qy), key=owner.id.join(",");
        if(!cache.has(key))cache.set(key,contourReference(layout,owner,seed,width,height,rotated));
        const query=cache.get(key),point=[qx-owner.x,qy-owner.y];
        const local=query(point),screen=query(point,true);
        const gap=((20*time/screen.length-screen.progress)%1+1)%1;
        const pulse=Math.exp(-Math.LN2*gap*screen.length/3);
        const band=Math.exp(-2*Math.LN2*screen.distance/4);
        const expected=[local.progress,local.length*0.1,pulse*band];
        for(let c=0;c<3;c++) { const error=Math.abs(pixels[(y*width+x)*4+c]-Math.max(0,Math.min(1,expected[c]))*255);sum+=error;if(error>3)bad++; }
      }
      expect(sum/(width*height*3)).toBeLessThan(0.8);
      // Float roundoff can change the nearest edge at exact medial-axis ties.
      expect(bad/(width*height*3)).toBeLessThan(0.005);
    }
  });
}

test("contour gallery remains compact and animated",async({page},testInfo)=>{
  const program=readFileSync(new URL("../../../../../examples/90) gallery/hex_circuit.fr",import.meta.url),"utf8");
  await page.goto("/tests/gpu/");await page.waitForFunction(()=>window.gpuTest);
  await page.evaluate(source=>window.gpuTest.init("flip-card",source),program);
  for(const time of [0.7,1.4])await page.evaluate(time=>window.gpuTest.frame(String(time),time,0,640,480),time);
  const comparison=await page.evaluate(()=>window.gpuTest.compare("0.7","1.4"));
  expect(comparison.maxDifference).toBeGreaterThan(20);
  const png=await page.evaluate(()=>window.gpuTest.png("0.7"));
  await testInfo.attach("hex-contour-circuit",{body:Buffer.from(png.split(",")[1],"base64"),contentType:"image/png"});
});


test("contour points use arc length and collapsed contours emit no light", async ({page}) => {
  await page.goto("/tests/gpu/"); await page.waitForFunction(()=>window.gpuTest);
  for (const inset of ["0.1", "2.0", "100px"]) {
    const source=`canvas t(ctx: CanvasContext) -> color {
      in space cells(layout: square, every: 1, seed: 0, sampling: center, cell: tile) {
        let track=tile.contour(inset: ${inset})
        let point=track.point(at: 0.25)
        fill(rgb(point.x, point.y, track.length / 4 + chase(along: track, lap: 3s, tail: 0.1) * ${inset === "0.1" ? "0" : "1"}))
      }
    }`;
    await page.evaluate(source=>window.gpuTest.init("flip-card",source),source);
    await page.evaluate(()=>window.gpuTest.frame("points",0.7,0,32,32));
    const pixels=await page.evaluate(()=>window.gpuTest.pixels("points"));
    const expected=inset==="0.1"?[0.9,0.9,0.8]:[0.5,0.5,0];
    for(let i=0;i<pixels.length;i+=4)for(let c=0;c<3;c++)expect(Math.abs(pixels[i+c]-expected[c]*255)).toBeLessThan(2);
  }
});


test("contour pixel insets and widths keep their screen size", async ({page}) => {
  const source=`canvas t(ctx: CanvasContext) -> color {
    in space cells(layout: square, every: 1, seed: 0, sampling: center, cell: tile) {
      let track=tile.contour(inset: 2px)
      fill(rgb(track.length / 4, band(track.distance, width: 2px, profile: soft), 0))
    }
  }`;
  await page.goto("/tests/gpu/"); await page.waitForFunction(()=>window.gpuTest);
  await page.evaluate(source=>window.gpuTest.init("flip-card",source),source);
  for(const [width,height] of [[64,48],[96,64]]) {
    await page.evaluate(({width,height})=>window.gpuTest.frame("inset",0,0,width,height),{width,height});
    const pixels=await page.evaluate(()=>window.gpuTest.pixels("inset"));
    const length=4-8/width-8/height;
    for(let y=0;y<height;y++)for(let x=0;x<width;x++) {
      const px=x+0.5,py=y+0.5;
      const dx=Math.max(2-px,0,px-(width-2)),dy=Math.max(2-py,0,py-(height-2));
      const outside=dx>0||dy>0;
      const distance=outside?Math.hypot(dx,dy):Math.min(px-2,width-2-px,py-2,height-2-py);
      const expected=[length/4,Math.exp(-Math.LN2*distance)];
      for(let c=0;c<2;c++)expect(Math.abs(pixels[(y*width+x)*4+c]-expected[c]*255)).toBeLessThan(2);
    }
  }
});


test("hex contour helpers match the original light fields at multiple times and sizes", async ({page}) => {
  // Keep the exact conversion as a compiler equivalence fixture. The authored
  // gallery intentionally rounds its tuning constants to three decimal places.
  const actual=readFileSync(new URL("../../../../../tests/fixtures/hex_circuit_contour_equivalence.fr", import.meta.url),"utf8");
  const original=readFileSync(new URL("../../../../../tests/fixtures/hex_circuit_geometry.fr", import.meta.url),"utf8");
  await page.goto("/tests/gpu/"); await page.waitForFunction(()=>window.gpuTest);
  for(const [width,height] of [[320,240],[640,480]]) for(const time of [0,0.7,2.3]) {
    for(const [name,source] of [["actual",actual],["original",original]]) {
      await page.evaluate(source=>window.gpuTest.init("flip-card",source),source);
      await page.evaluate(({name,time,width,height})=>window.gpuTest.frame(name,time,0,width,height),{name,time,width,height});
    }
    const result=await page.evaluate(()=>window.gpuTest.compare("actual","original"));
    expect(result.maxDifference, JSON.stringify({width,height,time,result})).toBeLessThanOrEqual(1);
  }
});
