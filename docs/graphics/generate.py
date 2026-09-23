"""Generate Fresco's conceptual SVG diagrams.

Run: python docs/graphics/generate.py
Only the Python standard library is required. These are explanatory diagrams,
not compiler renders, performance measurements, or golden test expectations.
"""

from html import escape
from math import cos, sin, pi
from pathlib import Path

ROOT = Path(__file__).resolve().parent
INK = "#edf3ff"
MUTED = "#adbad0"
PINK = "#ff4b91"
BLUE = "#63caff"
MINT = "#73e2bf"
GOLD = "#ffc875"
plates = []


def text(x, y, value, size=20, color=INK, weight=400, **attrs):
    extra = " ".join(f'{key.replace("_", "-")}="{escape(str(val))}"' for key, val in attrs.items())
    return f'<text x="{x}" y="{y}" font-size="{size}" fill="{color}" font-weight="{weight}" {extra}>{escape(value)}</text>'


def rect(x, y, w, h, fill="#131f33", stroke="#2b3c55", r=16, **attrs):
    extra = " ".join(f'{key.replace("_", "-")}="{val}"' for key, val in attrs.items())
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}" stroke="{stroke}" {extra}/>'


def line(x1, y1, x2, y2, color=BLUE, width=2, arrow=False, dash=False):
    return f'<path d="M{x1},{y1} L{x2},{y2}" fill="none" stroke="{color}" stroke-width="{width}"' + (' marker-end="url(#arrow)"' if arrow else '') + (' stroke-dasharray="6 6"' if dash else '') + '/>'


def dot(x, y, r=5, color=GOLD):
    return f'<circle cx="{x}" cy="{y}" r="{r}" fill="{color}"/>'


def path(d, color=BLUE, width=2, fill="none", **attrs):
    extra = " ".join(f'{key.replace("_", "-")}="{val}"' for key, val in attrs.items())
    return f'<path d="{d}" fill="{fill}" stroke="{color}" stroke-width="{width}" {extra}/>'


def card(x, y, w, h, title, lines=(), color=BLUE):
    s = rect(x, y, w, h) + rect(x, y, 4, h, color, color, 2)
    s += text(x+20, y+34, title, 22, color, 650)
    for i, value in enumerate(lines):
        s += text(x+20, y+67+i*27, value, 18, MUTED)
    return s


def grid(x, y, w, h, step=32):
    s = rect(x, y, w, h, "#0c1627", "#2b3c55", 8)
    for dx in range(step, w, step):
        s += line(x+dx, y, x+dx, y+h, "#203047", 1)
    for dy in range(step, h, step):
        s += line(x, y+dy, x+w, y+dy, "#203047", 1)
    return s


def plate(slug, title, subtitle, body, desc, height=620):
    defs = '''<defs>
    <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 1 L9 5 L0 9" fill="none" stroke="#63caff" stroke-width="1.5"/></marker>
    <linearGradient id="paint"><stop stop-color="#ff4b91"/><stop offset=".5" stop-color="#a68dff"/><stop offset="1" stop-color="#63caff"/></linearGradient>
    <radialGradient id="orb" cx="30%" cy="25%"><stop stop-color="#ffe3ad"/><stop offset=".55" stop-color="#f7a370"/><stop offset="1" stop-color="#703352"/></radialGradient>
    <radialGradient id="haze"><stop stop-color="#ffc875" stop-opacity=".9"/><stop offset=".45" stop-color="#ff9975" stop-opacity=".4"/><stop offset="1" stop-color="#ff9975" stop-opacity="0"/></radialGradient>
    </defs>'''
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1120" height="{height}" viewBox="0 0 1120 {height}" role="img" aria-labelledby="title description">
    <title id="title">{escape(title)}</title><desc id="description">{escape(desc)}</desc>
    {defs}<g font-family="Segoe UI, Arial, sans-serif">
    {rect(0, 0, 1120, height, '#0a1020', '#28364c', 22)}
    {text(36, 36, 'F R E S C O   /   VISUAL LANGUAGE ATLAS', 12, MINT, 700)}
    {text(36, 79, title, 32, INK, 700)}
    {text(36, 111, subtitle, 18, MUTED)}
    {body}
    {line(36, height-43, 1084, height-43, '#28364c', 1)}
    {text(36, height-19, 'GEOMETRY / mint     PAINT / pink     COORDINATES + FLOW / blue     RESOURCES / amber', 12, MUTED)}
    {text(1084, height-19, 'CONCEPTUAL DIAGRAM', 11, MUTED, text_anchor='end')}
    </g></svg>'''
    (ROOT / f"{slug}.svg").write_text(svg + "\n", encoding="utf-8", newline="\n")
    plates.append((slug, title, subtitle, svg))


def system():
    s = card(36, 155, 300, 192, '01 / Author intent', ['Shapes, layers, spaces', 'Materials and styles', 'Engine contracts + operations'], MINT)
    s += card(410, 155, 300, 192, '02 / Compile together', ['Check contracts + specialize', 'Rewrite supported patterns', 'Lower shaders + plan work'], PINK)
    s += card(784, 155, 300, 192, '03 / Run in your engine', ['Validate + bind resources', 'Create pipelines + allocate', 'Submit GPU work + present'], BLUE)
    s += line(342, 249, 402, 249, arrow=True) + line(716, 249, 776, 249, arrow=True)
    s += card(410, 373, 300, 108, 'Paired output', ['WGSL + execution metadata'], GOLD)
    s += line(560, 347, 560, 368, arrow=True)
    s += path('M710 425 H934 V354', BLUE, 2, marker_end='url(#arrow)')
    s += text(36, 392, 'Engine-authored contract', 20, MINT, 650)
    s += text(36, 422, 'Defines inputs, capabilities,', 18, MUTED)
    s += text(36, 450, 'and rendering boundaries.', 18, MUTED)
    s += text(784, 510, 'Adapter / provider boundary', 20, BLUE, 650)
    s += text(784, 539, 'Host data + backend integration', 17, MUTED)
    s += card(36, 582, 330, 112, 'Working reference', ['Bundled Rust / wgpu engine', 'Native + browser hosts'], MINT)
    s += card(390, 582, 330, 112, 'Potential target: Godot', ['Adapter + backend work needed', 'No ready-made plugin implied'], GOLD)
    s += card(744, 582, 340, 112, 'Potential target: Unreal', ['Or your own engine / renderer', 'Adapter + backend work needed'], GOLD)
    s += text(36, 738, 'Shader output and host execution must agree on bindings, resources, stages, and lifetime.', 19, INK)
    plate('engine-integration', 'From visual intent to your renderer', 'Fresco compiles authored behavior. Your engine supplies the world and executes the result.', s,
          'Authored content and engine declarations enter Fresco. It produces paired WGSL and execution metadata. A host validates resources and runs the work. Rust/wgpu is the working reference; Godot and Unreal are potential integration targets requiring adapters.', 810)


def depths():
    s = ''
    for x, title, lines, color in [(36, 'Canvas', ['Render a procedural picture', 'to an engine-owned target.'], BLUE), (396, 'Materials', ['Connect surface evaluation', 'to your lighting path.'], PINK), (756, 'Rendering contributions', ['Expose capabilities + boundaries', 'for compute and extra draws.'], MINT)]:
        s += card(x, 155, 328, 365, title, (), color)
        for i, value in enumerate(lines):
            s += text(x+20, 416+i*28, value, 18, MUTED)
    s += '<circle cx="200" cy="290" r="74" fill="none" stroke="#283d58" stroke-width="18"/>'
    s += '<circle cx="200" cy="290" r="74" fill="none" stroke="#63caff" stroke-width="18" stroke-dasharray="335 465" transform="rotate(-90 200 290)"/>'
    s += text(200, 299, '72%', 28, BLUE, 650, text_anchor='middle')
    s += '<circle cx="560" cy="290" r="80" fill="url(#orb)"/>'
    s += '<circle cx="920" cy="290" r="87" fill="#ff4b91"/>'
    s += '<circle cx="920" cy="290" r="78" fill="url(#orb)"/>'
    s += text(36, 555, 'Choose an integration depth; deeper access requires a richer engine contract.', 20)
    plate('integration-depths', 'Three ways to bring Fresco into an engine', 'Each route has a visible payoff and a concrete host responsibility.', s,
          'Canvas integration produces a picture in a target. Material integration connects surface logic to engine lighting. Rendering contributions additionally connect compute and draws to engine capabilities and ordering.')


def badge():
    s = card(36, 152, 240, 353, 'One shape', [], MINT)
    for grow in (36, 24, 12, 0):
        s += rect(89-grow/2, 271-grow/2, 134+grow, 94+grow, 'none', MINT if grow == 0 else '#315c56', 20+grow/3)
    s += text(60, 441, 'Rounded-box field', 20, MINT)
    s += text(60, 474, 'Geometry stays reusable.', 16, MUTED)
    for y in (211, 325, 439):
        s += path(f'M276 326 H300 V{y} H326', BLUE, 2, marker_end='url(#arrow)')
    for y, title, label, color in [(165, 'Shadow', 'Offset + soft coverage', GOLD), (279, 'Fill', 'Paint the interior', PINK), (393, 'Stroke → fill', 'Outline shape → paint', MINT)]:
        s += card(332, y, 260, 92, title, [label], color)
    s += text(663, 177, 'Source order → final picture', 23, INK, 650)
    s += '<g id="badge-shadow" transform="translate(0 45)">' + rect(745, 279, 200, 140, '#000000', '#243249', 28) + '</g>'
    s += '<g id="badge-fill" transform="translate(0 0)">' + rect(733, 267, 200, 140, PINK, PINK, 28) + '</g>'
    s += '<g id="badge-stroke" transform="translate(0 -45)">' + rect(733, 267, 200, 140, 'none', INK, 28, stroke_width=4) + '</g>'
    s += text(687, 510, 'Exploded layers; later entries on top.', 18, MUTED)
    s += text(36, 552, 'A shape is geometry. Filling it produces a layer that can be composed.', 22)
    plate('shape-layers', 'One shape. Three treatments. One picture.', 'Follow the badge from a distance field to painted layers.', s,
          'One rounded-box field feeds shadow, fill, and stroke followed by fill. The layers are shown exploded: shadow below pink fill below white outline. Later composition entries appear above earlier ones.')


def polar():
    s = grid(60, 185, 400, 288, 40)
    s += text(60, 162, 'Authored space', 22, MINT, 650)
    s += rect(61, 303, 398, 20, '#34445a', '#34445a', 0)
    s += line(160, 186, 160, 472, MINT, 2) + line(260, 186, 260, 472, PINK, 2)
    s += '<rect id="polar-bar" x="61" y="303" width="287" height="20" fill="#63caff"/>'
    s += line(60, 493, 460, 493, arrow=True) + text(190, 529, 'x = turn progress', 20, BLUE)
    s += text(67, 218, 'y = radius', 18, BLUE)
    s += line(495, 329, 605, 329, arrow=True) + text(511, 306, 'polar', 20, BLUE)
    for radius in (40, 80, 120, 160):
        s += f'<circle cx="837" cy="330" r="{radius}" fill="none" stroke="#293c56"/>'
    for i in range(12):
        a = i*pi/6
        s += line(837, 330, 837+160*cos(a), 330+160*sin(a), '#293c56', 1)
    s += line(837, 330, 997, 330, MINT, 2) + line(837, 330, 837, 490, PINK, 2)
    s += '<circle cx="837" cy="330" r="112" fill="none" stroke="#34445a" stroke-width="20"/>'
    s += '<circle id="polar-arc" cx="837" cy="330" r="112" fill="none" stroke="#63caff" stroke-width="20" stroke-dasharray="506.68 703.72" transform="rotate(-90 837 330)"/>'
    s += f'<circle id="polar-endpoint" cx="{837+112*sin(.72*2*pi):.2f}" cy="{330-112*cos(.72*2*pi):.2f}" r="5" fill="#ffc875"/>'
    s += text(700, 162, 'Mapped picture', 22, PINK, 650)
    s += text(837, 336, '72%', 28, INK, 650, id='polar-value', text_anchor='middle')
    s += text(709, 528, 'Same track. Same progress.', 20, MUTED)
    s += dot(349, 313) + text(349, 287, '0.72', 16, GOLD, text_anchor='middle')
    s = s.replace('<circle cx="349"', '<circle id="polar-point" cx="349"').replace('x="349" y="287"', 'id="polar-point-label" x="349" y="287"')
    plate('polar-space', 'Bend the space, bend the picture', 'A straight track becomes a dial: x wraps clockwise from the top; y sets radius.', s,
          'A straight progress bar occupies 72 percent of authored x coordinates. A polar space maps that progress clockwise around a ring, starting at the top. Track and bar share the mapping. Grid spacing is schematic.')


def inverse():
    s = grid(60, 178, 360, 320, 40) + grid(700, 178, 360, 320, 40)
    s += text(60, 157, 'Original field', 22, MINT, 650) + text(700, 157, 'Output picture', 22, PINK, 650)
    s += rect(150, 278, 180, 120, '#16392f', MINT, 16)
    s += '<g id="inverse-shape" transform="rotate(-30 880 338)">' + rect(790, 278, 180, 120, '#733251', PINK, 16) + '</g>'
    s += dot(940, 308, 7) + text(934, 286, 'output sample', 17, GOLD, text_anchor='end')
    s += '<circle id="inverse-point" cx="307" cy="342" r="7" fill="#ffc875"/>'
    s += line(452, 239, 668, 239, arrow=True) + text(560, 215, 'Author: rotate +30°', 20, PINK, text_anchor='middle', id='inverse-label')
    s += line(668, 402, 452, 402, arrow=True) + text(560, 439, 'Sample: undo rotation', 19, BLUE, text_anchor='middle')
    s += text(60, 546, 'For each output pixel, map its coordinate back before evaluating the original field.', 22)
    plate('inverse-sampling', 'The pixel travels backward', 'Content moves forward through a transform; sample lookup uses its inverse.', s,
          'The authored rounded box is rotated counterclockwise. To shade a fixed output sample, undo the rotation and evaluate the original field at that mapped coordinate. The yellow point shows the corresponding lookup.')


def gradients():
    s = ''
    for row, label, caption in [(0, 'anchor: scene', 'A shared ruler in the active space'), (1, 'anchor: shape', 'A full gradient for each receiver')]:
        y = 174+row*195
        s += text(36, y+20, label, 23, BLUE if row == 0 else PINK, 650)
        s += text(36, y+51, caption, 16, MUTED)
        if row == 0:
            s += rect(372, y, 682, 12, 'url(#paint)', 'none', 2)
            s += text(372, y-9, '0', 15, MUTED) + text(1054, y-9, '1', 15, MUTED, text_anchor='end')
        for n, (x, w) in enumerate([(395, 140), (610, 210), (903, 110)]):
            fill = 'url(#scene-paint)' if row == 0 else 'url(#paint)'
            s += rect(x, y+43, w, 80, fill, 'none', 20, id=f'gradient-{row}-{n}')
            if row == 1:
                s += f'<g id="shape-ruler-{n}">' + rect(x, y, w, 12, 'url(#paint)', 'none', 2)
                s += text(x, y-9, '0', 15, MUTED) + text(x+w, y-9, '1', 15, MUTED, text_anchor='end') + '</g>'
    s += '<defs><linearGradient id="scene-paint" gradientUnits="userSpaceOnUse" x1="372" x2="1054"><stop stop-color="#ff4b91"/><stop offset=".5" stop-color="#a68dff"/><stop offset="1" stop-color="#63caff"/></linearGradient></defs>'
    s += text(36, 551, 'Scene follows the active space. Shape anchoring requires a supported owning shape.', 21)
    plate('gradient-anchors', 'Where does the gradient live?', 'Matching geometry, different coordinate ownership.', s,
          'Scene-anchored shapes sample portions of one shared gradient ruler. Shape-anchored shapes each span the full pink-to-blue gradient. Scene means the active space, not necessarily fixed screen coordinates.')


def clip_polygon(poly, nx, ny, c):
    out = []
    for p, q in zip(poly, poly[1:]+poly[:1]):
        a, b = nx*p[0]+ny*p[1]-c, nx*q[0]+ny*q[1]-c
        if a <= 0:
            out.append(p)
        if (a <= 0) != (b <= 0):
            t = a/(a-b)
            out.append((p[0]+t*(q[0]-p[0]), p[1]+t*(q[1]-p[1])))
    return out


def cells():
    s = text(36, 161, 'Cells: one owner per sample', 22, MINT, 650)
    sites = [(110, 248), (246, 218), (372, 262), (147, 391), (300, 393), (426, 422)]
    for i, (x, y) in enumerate(sites):
        poly = [(36, 181), (486, 181), (486, 488), (36, 488)]
        for j, (qx, qy) in enumerate(sites):
            if i != j:
                poly = clip_polygon(poly, qx-x, qy-y, (qx*qx+qy*qy-x*x-y*y)/2)
        points = ' '.join(f'{px:.2f},{py:.2f}' for px, py in poly)
        s += f'<defs><clipPath id="cell-{i}"><polygon points="{points}"/></clipPath></defs>'
        s += f'<polygon points="{points}" fill="{["#153d42", "#2d2447", "#173346"][i%3]}" stroke="#73e2bf" stroke-width="1.5"/>'
        s += f'<g clip-path="url(#cell-{i})"><circle cx="{x}" cy="{y}" r="94" fill="url(#haze)"/>' + line(x-70, y+22, x+105, y+22, PINK, 14) + '</g>'
        s += dot(x, y, 5, MINT) + text(x+9, y-9, f'ID {i}', 15, INK)
    s += text(36, 524, 'Motifs clip at the ownership boundary.', 20, MUTED)
    s += text(560, 161, 'Scatter: instances can overlap', 22, PINK, 650)
    for x, y in [(670, 240), (747, 264), (824, 231)]:
        s += f'<circle cx="{x}" cy="{y}" r="58" fill="#ff4b91" fill-opacity=".35" stroke="#ff4b91"/>'
    s += text(560, 354, 'One pixel, multiple ownership queries', 22, BLUE, 650)
    s += rect(572, 379, 136, 136, '#163d43', MINT, 0)
    s += path('M625 379 H708 V515 H675 Z', '#5d487d', 0, '#5d487d')
    for row in range(4):
        for col in range(4):
            s += dot(589+34*col, 396+34*row, 4, INK)
    s += text(735, 410, 'Map → owner → body', 20, BLUE)
    s += text(735, 442, 'Repeat for each subsample.', 18, MUTED)
    s += text(735, 474, 'Combine alpha-weighted colors.', 18, MUTED)
    s += text(735, 506, 'Finite grids can miss tiny details.', 17, GOLD)
    plate('cellular-ownership', 'Who owns this pixel?', 'Ownership boundaries and sampling footprints are part of the picture.', s,
          'Six Voronoi regions each clip a glowing stripe to one owner. Scatter circles overlap instead. A magnified screen pixel has sixteen sample locations spanning two owners; each location reevaluates mapping, owner, and body. Finite grids are approximate.')


def effects():
    s = card(36, 155, 494, 351, 'A sampled effect', ['Chromatic split reads one layer', 'at three coordinate locations.'], BLUE)
    for x, label, color in [(126, 'left.r', PINK), (276, 'self.g', MINT), (426, 'right.b', BLUE)]:
        s += dot(x, 310, 15, color) + text(x, 349, label, 18, color, text_anchor='middle')
        s += line(x, 367, 276, 414, color, arrow=True)
    s += rect(220, 422, 112, 44, '#dcc0ef', 'none', 8) + text(276, 451, 'RGBA', 19, '#152039', 650, text_anchor='middle')
    s += card(570, 155, 514, 351, 'A recognized analytic rewrite', ['shape → fill → blur', 'Eligible pattern → analytic soften'], MINT)
    s += '<circle cx="710" cy="365" r="87" fill="url(#haze)"/>'
    for radius in (32, 48, 64, 80):
        s += f'<circle cx="928" cy="365" r="{radius}" fill="none" stroke="#73e2bf" stroke-opacity="{1-radius/110:.2f}"/>'
    s += line(808, 365, 838, 365, arrow=True)
    s += text(598, 476, 'Use --explain to inspect the selected rewrite.', 18, MUTED)
    s += text(36, 550, 'More samples can mean more work. An effect does not automatically imply another GPU pass.', 21)
    plate('effect-footprints', 'What does an effect sample?', 'Sampled evaluation and analytic rewrites have different execution costs.', s,
          'Chromatic split combines red from a left sample, green at the current sample, and blue from a right sample. An eligible shape-fill-blur pattern can instead use an analytic soften rewrite. The diagram makes no universal pass-count or timing claim.')


def styles():
    s = ''
    for x, title, lines, color in [(36, 'PBR response', ['Surface inputs + lighting', '→ shaded base surface'], GOLD), (396, 'Toon + outline', ['Banded light response', '+ expanded hull draw'], PINK), (756, 'Fur + generated shells', ['Density + generated vertices', '+ translucent shell draws'], MINT)]:
        s += card(x, 155, 328, 364, title, [], color)
        for i, label in enumerate(lines):
            s += text(x+20, 445+i*28, label, 18, MUTED)
    s += '<circle cx="200" cy="302" r="83" fill="url(#orb)"/>'
    s += '<circle cx="560" cy="302" r="91" fill="#ff4b91"/>'
    s += '<defs><clipPath id="toon"><circle cx="560" cy="302" r="80"/></clipPath></defs>'
    s += '<g clip-path="url(#toon)">' + rect(475, 217, 170, 170, '#753950', 'none', 0) + path('M475 217 H645 L581 387 H475Z', '#f0a276', 0, '#f0a276') + path('M475 217 H575 L510 340 H475Z', '#ffdfab', 0, '#ffdfab') + '</g>'
    s += '<circle cx="920" cy="302" r="61" fill="url(#orb)"/>'
    for radius in (70, 80, 90, 100):
        s += f'<circle cx="920" cy="302" r="{radius}" fill="none" stroke="#73e2bf" stroke-width="4" stroke-dasharray="4 5" stroke-opacity="{1-radius/150:.2f}"/>'
    s += text(36, 552, 'A style supplies shading hooks and can invoke operations through the engine contract.', 21)
    plate('material-styles', 'A style can change more than the lighting', 'Schematic cross-sections expose the work behind a material’s appearance.', s,
          'A smooth PBR sphere symbolizes base shading. Toon uses banded shading and an expanded outline hull. Fur uses a root surface plus density-controlled generated shells. These are conceptual drawings, not rendered output comparisons.')


def scheduling():
    s = card(36, 153, 246, 101, 'Fur settings', ['Seed + density resolution'], GOLD)
    s += card(354, 153, 248, 101, 'Density compute', ['Owned texture'], GOLD)
    s += card(718, 153, 366, 101, 'Fur base shading', ['Reads density + lighting inputs'], PINK)
    s += card(36, 314, 246, 105, 'Prepared fur mesh', ['Deformed material range'], MINT)
    s += card(354, 314, 248, 105, 'Shell vertex compute', ['Generated stream + settings'], MINT)
    s += card(718, 314, 366, 105, 'Complete opaque', ['All opaque depth + lighting'], BLUE)
    s += card(718, 471, 366, 85, 'Toon outline draws', ['Selected material ranges'], PINK)
    s += card(354, 607, 730, 101, 'Global transparency queue', ['Fur shells + ordinary transparent draws in one order'], GOLD)
    s += card(718, 759, 366, 78, 'Inspect / present', [], BLUE)
    for a, b, c, d in [(282, 203, 348, 203), (602, 203, 712, 203), (282, 366, 348, 366), (901, 254, 901, 308), (901, 419, 901, 465), (901, 556, 901, 601), (901, 708, 901, 753)]:
        s += line(a, b, c, d, arrow=True)
    s += path('M478 254 V284 H656 V579 H590 V601', GOLD, 2, marker_end='url(#arrow)')
    s += path('M478 419 V601', MINT, 2, marker_end='url(#arrow)')
    s += text(36, 482, 'Preparation follows its inputs.', 18, INK, 650)
    s += text(36, 513, 'It need not wait for opaque.', 18, MUTED)
    s += text(36, 771, 'Edges show required dependencies,', 18, MUTED)
    s += text(36, 800, 'not durations or GPU overlap.', 18, MUTED)
    s += text(36, 874, 'Forward-path sketch; only key edges shown. Lighting and view resources are also required.', 19, MUTED)
    plate('render-dependencies', 'Dependencies determine when work can run', 'Preparation branches meet at their consumers; extra draws join explicit engine boundaries.', s,
          'Fur settings feed density compute, which feeds forward base shading and transparent shells. Prepared fur geometry feeds shell vertex compute. Complete opaque rendering precedes Toon, then the global transparency queue, then inspection and presentation. Density and vertex preparation have no edge between them. Other inputs are omitted.', 945)


def frame():
    s = text(36, 159, 'ENGINE INPUTS', 14, GOLD, 700)
    s += card(36, 180, 300, 110, 'Prepared geometry', ['Selected object / material range'], MINT)
    s += card(408, 180, 300, 110, 'View + lighting', ['Camera, lights, scene resources'], GOLD)
    s += card(780, 180, 304, 110, 'Color + depth', ['Typed attachments + access'], GOLD)
    s += text(36, 346, 'FRAME ORDER', 14, BLUE, 700)
    for x, w, title, labels, color in [(36, 250, 'Opaque rendering', ['Toon shades its surface', 'alongside other objects.'], BLUE), (322, 258, 'after_opaque', ['Style invokes its outline.', 'Preserve color; test depth.'], PINK), (616, 232, 'Transparency', ['All transparent draws', 'join the global queue.'], GOLD), (884, 200, 'Present', ['Inspect + display', 'the final color.'], BLUE)]:
        s += card(x, 369, w, 139, title, labels, color)
    for x1, x2 in [(286, 316), (580, 610), (848, 878)]:
        s += line(x1, 437, x2, 437, arrow=True)
    s += path('M186 290 V318 H452 V363', MINT, 2, marker_end='url(#arrow)')
    s += path('M558 290 V336 H161 V363', GOLD, 2, marker_end='url(#arrow)')
    s += path('M932 290 V318 H490 V363', GOLD, 2, marker_end='url(#arrow)')
    s += text(36, 550, 'The boundary follows complete opaque rendering for the view, not just this object’s draw.', 21)
    plate('toon-frame', 'Where Toon joins your frame', 'The engine provides the contract. The style contributes work at an explicit integration point.', s,
          'The engine supplies prepared geometry, view and lighting, and typed color/depth attachments. Toon base shading participates in opaque rendering. After complete opaque rendering, its outline tests depth without writing it and preserves color. Transparency and presentation follow. Key dependencies only.')


def fields():
    s = text(36, 160, 'A number at every coordinate', 23, MINT, 650)
    # Circle signed distance, drawn as a sampled explanatory heatmap.
    for iy in range(30):
        for ix in range(30):
            d = (((ix+.5)/30-.5)**2 + ((iy+.5)/30-.5)**2)**.5 - .22
            amount = min(abs(d)/.35, 1)
            base = (115, 226, 191) if d < 0 else (99, 202, 255)
            color = '#' + ''.join(f'{int(c*(.28+.72*amount)):02x}' for c in base)
            s += rect(50+ix*10, 184+iy*10, 10.2, 10.2, color, 'none', 0)
    for radius in (36, 66, 96, 126):
        s += f'<circle cx="200" cy="334" r="{radius}" fill="none" stroke="{INK if radius == 66 else BLUE}" stroke-width="{2 if radius == 66 else 1}"/>'
    s += line(50, 334, 350, 334, GOLD, 2, dash=True)
    s += dot(290, 334, 7).replace('<circle ', '<circle id="field-probe" ')
    s += text(50, 518, 'White contour: d = 0', 19, INK)
    s += text(50, 548, 'Inside: d < 0    Outside: d > 0', 18, MUTED)
    s += text(430, 160, 'Slice through the center', 23, GOLD, 650)
    s += grid(430, 184, 630, 300, 60)
    # x spans 0..1; y maps distance with 400 screen units per unit.
    zero = 184+(.5-.0)*400
    s += line(430, zero, 1060, zero, INK, 1)
    s += path(f'M430 {zero-.28*400} L745 {zero+.22*400} L1060 {zero-.28*400}', MINT, 3)
    s += text(1045, zero-10, 'd = 0', 16, INK, text_anchor='end')
    s += dot(934, zero-.08*400, 7).replace('<circle ', '<circle id="field-slice-probe" ')
    s += text(430, 518, 'd(p) = length(p - center) - radius', 22, MINT)
    s += text(430, 548, 'Not every scalar field is a signed distance field.', 20, MUTED)
    s += card(36, 579, 320, 91, '01 / Evaluate', ['x = 0.80 → d = +0.08'], MINT)
    s = s.replace('>x = 0.80 → d = +0.08</text>', ' id="field-readout">x = 0.80 → d = +0.08</text>')
    s += line(362, 622, 390, 622, arrow=True)
    s += card(396, 579, 328, 91, '02 / Coverage', ['Inside / edge / outside'], BLUE)
    s += line(730, 622, 754, 622, arrow=True)
    s += card(760, 579, 324, 91, '03 / Paint', ['Coverage weights a color'], PINK)
    s += text(36, 709, 'The field is a function. This heatmap visualizes values; no texture allocation is implied.', 20)
    plate('fields', 'A field is a function over space', 'Probe a circle’s signed distance: geometry is a boundary in a continuous field.', s,
          'A heatmap visualizes a circle signed distance function. A horizontal slice is V-shaped: negative inside, zero at the boundary, positive outside. The highlighted probe represents the same coordinate in both views. Coverage and paint turn field values into a visible layer. General scalar fields need not be signed distance fields.', 780)


def compounded_spaces():
    s = ''
    for x, label, nesting, name, cx in [(36, 'Translate outside scale', 'translate { scale { shape } }', 'a', .71), (588, 'Scale outside translate', 'scale { translate { shape } }', 'b', .662)]:
        s += text(x, 158, label, 23, MINT if name == 'a' else PINK, 650)
        s += text(x, 190, nesting, 19, MUTED)
        s += grid(x, 210, 496, 280, 40)
        s += line(x, 350, x+496, 350, '#53708a', 1)
        s += line(x+248, 210, x+248, 490, '#53708a', 1)
        s += f'<circle cx="{x+496*.65}" cy="350" r="{496*.12}" fill="none" stroke="#adbad0" stroke-dasharray="5 5"/>'
        s += f'<circle id="compound-{name}" cx="{x+496*cx}" cy="350" r="{496*.12*.6}" fill="{MINT if name == "a" else PINK}" fill-opacity=".65"/>'
        s += text(x+18, 246, 'Dashed: original circle', 17, MUTED)
        s += text(x+18, 469, f'Output center x = {cx:.3f}', 20, INK, id=f'compound-label-{name}')
    s += text(36, 536, 'Pixel lookup: undo outer, then inner', 22, BLUE, 650)
    s += text(36, 574, 'q = center + (p - shift - center) / scale', 20, MINT)
    s += text(588, 574, 'q = center + (p - center) / scale - shift', 20, PINK)
    s += text(36, 622, 'Shift = (0.12, 0); scale = 0.60 around center. Both scopes affect all content inside.', 20, MUTED, id='compound-settings')
    s += text(36, 660, 'Swapping scopes changes whether the translation itself is scaled.', 23)
    plate('compounded-spaces', 'Spaces compound—and order matters', 'Identical circle, identical transforms, different nesting.', s,
          'Compare an outer translation with inner scale against outer scale with inner translation. Both use a circle at x 0.65, radius 0.12, shift 0.12, and scale 0.6 around the center. Centers become 0.710 and 0.662. Sample mapping undoes scopes from outer to inner.', 740)


def shader_translation():
    s = text(36, 154, 'AUTHORED FRESCO', 15, MINT, 700)
    s += text(524, 154, 'PER-SAMPLE EVALUATION', 15, BLUE, 700)
    rows = [
        ('in space translate((0.12, 0))', 'p1 = p - vec2(0.12, 0)', 'Undo the outer translation.', BLUE),
        ('in space scale(0.6, around: center)', 'q = center + (p1 - center) / 0.6', 'Undo the inner scale around its pivot.', BLUE),
        ('circle(at: (0.65, 0.5), radius: 0.12)', 'd = length(q - vec2(0.65, 0.5)) - 0.12', 'Evaluate the distance at that coordinate.', MINT),
        ('|> fill(#ff4b91)', 'a = clamp(0.5 - d / aa_width, 0, 1)', 'Convert distance into filtered coverage.', PINK),
        ('compose { background; circle_layer }', 'rgb = mix(background, ink, a)', 'Blend over the opaque background.', GOLD),
    ]
    for i, (source, code, caption, color) in enumerate(rows):
        y = 174+i*105
        s += rect(36, y, 426, 86)
        s += text(50, y+36, source, 18, color, 600)
        s += line(472, y+40, 512, y+40, arrow=True)
        s += rect(524, y, 560, 86)
        s += text(540, y+33, code, 20, color, 600)
        s += text(540, y+62, caption, 17, MUTED)
    s += text(36, 733, 'One fragment invocation follows this chain for its sample and returns an RGBA value.', 21)
    s += text(36, 769, 'Simplified pseudocode: generated WGSL includes context, scale guards, AA policy, and stage wrappers.', 18, MUTED)
    plate('source-to-shader', 'From a picture description to shader math', 'Read top to bottom: nested scopes become coordinates; geometry becomes numbers; paint becomes color.', s,
          'Five source-to-evaluation correspondences: translate subtracts an offset, scale divides coordinates around a pivot, circle evaluates signed distance, fill converts distance to coverage, and composition blends foreground and background. This is explanatory pseudocode, not literal emitted WGSL. The companion Fresco sample can be compiled to inspect actual output.', 840)


if __name__ == '__main__':
    for generate in (system, depths, fields, badge, polar, inverse, compounded_spaces, shader_translation, gradients, cells, effects, styles, scheduling, frame):
        generate()
    print(f'Generated {len(plates)} SVG diagrams.')
