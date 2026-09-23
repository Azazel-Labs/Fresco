import { isColorLikeParamType, normalizeTypeName } from "../preview/helpers";
import { VizAnnotationScanner, removeVizDirectiveFromLine } from "../viz-annotations";
import { CompiledVisualizerRenderer, addAxisOverlay } from "../visualizer-renderer";

type VizZoneManagerDeps = {
  requestWorkerQuery: (...args: any[]) => Promise<any>;
  writeVizRuntimeParamUniforms: (uniform: Float32Array) => void;
  resolveAnnotationSweep: (annotation: any) => any;
  resolveAnnotationSweepMax: (annotation: any) => number;
  buildShaderExplainText: (annotation: any, semanticType: string, renderPath: string, sweepInfo: any, wgsl: string, visualizerMeta?: any) => string;
  renderer: any;
  sourceEditor: any;
  switchToFile: (filename: string) => void;
};

let requestWorkerQuery: VizZoneManagerDeps["requestWorkerQuery"] = async () => null;
let writeVizRuntimeParamUniforms: VizZoneManagerDeps["writeVizRuntimeParamUniforms"] = () => {};
let resolveAnnotationSweep: VizZoneManagerDeps["resolveAnnotationSweep"] = () => ({ value: 0 });
let resolveAnnotationSweepMax: VizZoneManagerDeps["resolveAnnotationSweepMax"] = () => 0;
let buildShaderExplainText: VizZoneManagerDeps["buildShaderExplainText"] = () => "";
let renderer: VizZoneManagerDeps["renderer"] = null;
let sourceEditor: VizZoneManagerDeps["sourceEditor"] = null;
let switchToFile: VizZoneManagerDeps["switchToFile"] = () => {};

export function configureVizZoneManagerDeps(deps: Partial<VizZoneManagerDeps>) {
  if (typeof deps.requestWorkerQuery === "function") requestWorkerQuery = deps.requestWorkerQuery;
  if (typeof deps.writeVizRuntimeParamUniforms === "function") writeVizRuntimeParamUniforms = deps.writeVizRuntimeParamUniforms;
  if (typeof deps.resolveAnnotationSweep === "function") resolveAnnotationSweep = deps.resolveAnnotationSweep;
  if (typeof deps.resolveAnnotationSweepMax === "function") resolveAnnotationSweepMax = deps.resolveAnnotationSweepMax;
  if (typeof deps.buildShaderExplainText === "function") buildShaderExplainText = deps.buildShaderExplainText;
  if (deps.renderer !== undefined) renderer = deps.renderer;
  if (deps.sourceEditor !== undefined) sourceEditor = deps.sourceEditor;
  if (typeof deps.switchToFile === "function") switchToFile = deps.switchToFile;
}
// VizZoneManager: Manages Monaco view zones for @viz annotations
export class VizZoneManager {
  editor;
  device;
  format;
  scanner;
  compiledVisualizerRenderer = null;
  expandedZones = new Set();
  zones = new Map();
  updateTimer = null;
  refreshGeneration = 0;
  floatingWindows = new Map();
  nextFloatCascade = 0;
  zoneRelayoutTimers = new Map();
  scrollListener = null;
  layoutListener = null;
  shaderDialog = null;

  constructor(editor, device, format) {
    this.editor = editor;
    this.device = device;
    this.format = format;
    this.scanner = new VizAnnotationScanner();

    this.scrollListener = this.editor?.onDidScrollChange?.(() => {
      for (const [key, zone] of this.zones.entries()) {
        if (this.isAnnotationVisible(zone.annotation)) {
          this.scheduleZoneRelayout(key);
        }
      }
    }) || null;
    this.layoutListener = this.editor?.onDidLayoutChange?.(() => {
      for (const [key, zone] of this.zones.entries()) {
        if (this.isAnnotationVisible(zone.annotation)) {
          this.scheduleZoneRelayout(key);
        }
      }
    }) || null;
  }

  async initialize() {
    if (!this.device || !this.format) return;

    this.compiledVisualizerRenderer = new CompiledVisualizerRenderer(
      this.device,
      this.format,
      writeVizRuntimeParamUniforms
    );
    
    await this.refresh();
  }

  getSeriesDimensions() {
    const contentWidth = Number(this.editor?.getLayoutInfo?.()?.contentWidth) || 640;
    const width = Math.max(220, Math.min(760, Math.floor(contentWidth - 196)));
    return { width, height: 96 };
  }

  getThumbnailDimensions(expanded = false) {
    const contentWidth = Number(this.editor?.getLayoutInfo?.()?.contentWidth) || 640;
    const base = Math.max(220, Math.min(360, Math.floor(contentWidth * 0.40)));
    if (!expanded) {
      return { width: base, height: base };
    }
    const enlarged = Math.max(base + 120, Math.floor(base * 1.7));
    const clamped = Math.min(560, enlarged);
    return { width: clamped, height: clamped };
  }

  // Refresh all view zones based on current source
  async refresh() {
    const model = this.editor.getModel();
    if (!model) return;

    const generation = ++this.refreshGeneration;

    const keyOf = (ann) => String(ann?.stableKey || `${ann.spanStart}:${ann.spanEnd}:${ann.domain}`);
    const anchorOf = (ann) => {
      const line = Number(ann?.lineNumber || 0);
      const directiveLine = Number(ann?.directiveLineNumber || line);
      const placement = String(ann?.directivePlacement || 'trailing');
      return `${placement}:${line}:${directiveLine}`;
    };

    const source = model.getValue();
    const annotations = this.scanner.scan(model);

    // Re-key zones by stable visualizer anchor so edited directives keep the
    // previous rendered image until the replacement is ready.
    const anchorToExistingKey = new Map();
    for (const [existingKey, existingZone] of this.zones.entries()) {
      anchorToExistingKey.set(anchorOf(existingZone.annotation), existingKey);
    }
    for (const ann of annotations) {
      const nextKey = keyOf(ann);
      if (this.zones.has(nextKey)) {
        continue;
      }
      const oldKey = anchorToExistingKey.get(anchorOf(ann));
      if (!oldKey || oldKey === nextKey || !this.zones.has(oldKey)) {
        continue;
      }
      const existing = this.zones.get(oldKey);
      this.zones.delete(oldKey);
      existing.annotation = ann;
      this.zones.set(nextKey, existing);

      const pendingTimer = this.zoneRelayoutTimers.get(oldKey);
      if (pendingTimer) {
        clearTimeout(pendingTimer);
        this.zoneRelayoutTimers.delete(oldKey);
        this.zoneRelayoutTimers.set(nextKey, pendingTimer);
      }
    }
    
    // Build set of current annotation keys
    const newKeys = new Set(
      annotations.map((ann) => keyOf(ann))
    );
    this.expandedZones.forEach((expandedKey) => {
      if (!newKeys.has(expandedKey)) {
        this.expandedZones.delete(expandedKey);
      }
    });
    
    // Remove zones that no longer have annotations
    for (const [key, zone] of this.zones.entries()) {
      if (!newKeys.has(key)) {
        const pendingTimer = this.zoneRelayoutTimers.get(key);
        if (pendingTimer) {
          clearTimeout(pendingTimer);
          this.zoneRelayoutTimers.delete(key);
        }
        this.closeFloatingWindow(key);
        this.editor.changeViewZones((accessor) => {
          accessor.removeZone(zone.viewZoneId);
        });
        this.zones.delete(key);
      }
    }
    
    // Add or update zones for current annotations
    for (const ann of annotations) {
      if (generation !== this.refreshGeneration) {
        return;
      }
      const key = keyOf(ann);
      const isExpanded = this.expandedZones.has(key);
      let lastSemanticType = 'unknown';
      let lastVariantWgsl = '';
      let lastShaderExplain = '';
      let lastVisualizerMeta = null;
      
      try {
        // Query semantic type
        const spanInfo = await requestWorkerQuery("query_span", {
          source,
          spanStart: ann.spanStart,
          spanEnd: ann.spanEnd
        });
        if (generation !== this.refreshGeneration) {
          return;
        }
        if (!spanInfo?.ok) {
          const paramFallback = this.buildParamFallbackViz(ann);
          if (paramFallback) {
            const { dataUrl, semanticType, width, height } = paramFallback;
            if (this.zones.has(key)) {
              const zone = this.zones.get(key);
              const needsLayoutUpgrade = !zone.domNode?.querySelector('.viz-right-actions');
              if (zone.canvas && !needsLayoutUpgrade) {
                zone.canvas.src = dataUrl;
                this.scheduleZoneRelayout(key);
              } else {
                this.editor.changeViewZones((accessor) => {
                  accessor.removeZone(zone.viewZoneId);
                });
                const pendingTimer = this.zoneRelayoutTimers.get(key);
                if (pendingTimer) {
                  clearTimeout(pendingTimer);
                  this.zoneRelayoutTimers.delete(key);
                }
                this.zones.delete(key);
                this.createZone(key, ann, dataUrl, semanticType, width, height);
              }
            } else {
              this.createZone(key, ann, dataUrl, semanticType, width, height);
            }
            continue;
          }
          if (!this.zones.has(key)) {
            const seriesDims = this.getSeriesDimensions();
            this.upsertUnavailableZone(key, ann, 'unknown', seriesDims.width, seriesDims.height, 'span not capturable');
          }
          continue;
        }
        
        const semanticType = spanInfo.type || 'unknown';
        lastSemanticType = semanticType;
        
        // Render visualization based on type, domain, and explicit visualizer kind
        let dataUrl = null;
        const seriesDims = this.getSeriesDimensions();
        let vizWidth = seriesDims.width;
        let vizHeight = seriesDims.height;
        const sweepInfo = resolveAnnotationSweep(ann);
        const sweepMax = sweepInfo.value;
        const explicitKind = String(ann.kind || '').toLowerCase();
        let renderPath = 'thumbnail';
        let shaderWgsl = '';
        let visualizerMeta = null;
        
        if (ann.domain === 'thumb') {
          const thumbDims = this.getThumbnailDimensions(isExpanded);
          const visualizer: any = await requestWorkerQuery("compile_visualizer", {
            source,
            spanStart: ann.spanStart,
            spanEnd: ann.spanEnd,
            kind: 'thumbnail',
            domain: 'thumb',
            sweepMax: 0
          });
          if (generation !== this.refreshGeneration) {
            return;
          }
          if (!visualizer?.ok || !(visualizer.drawWgsl || visualizer.wgsl)) {
            const reason = Array.isArray(visualizer?.diagnostics) && visualizer.diagnostics.length > 0
              ? String(visualizer.diagnostics[0]?.message || 'visualizer compile failed')
              : 'visualizer compile failed';
            if (!this.zones.has(key)) {
              this.upsertUnavailableZone(key, ann, semanticType, seriesDims.width, seriesDims.height, reason);
            }
            continue;
          }
          shaderWgsl = visualizer.drawWgsl || visualizer.wgsl;
          lastVariantWgsl = shaderWgsl;
          // Use 2D thumbnail for thumb mode
          renderPath = 'thumbnail';
          dataUrl = await this.compiledVisualizerRenderer.renderShader({ drawWgsl: shaderWgsl, reduceWgsl: visualizer.reduceWgsl || "" }, {
            width: thumbDims.width,
            height: thumbDims.height,
            clearColor: [0.5, 0.5, 0.5, 1.0]
          });
          vizWidth = thumbDims.width;
          vizHeight = thumbDims.height;
        } else if (explicitKind === 'thumbnail') {
          const thumbDims = this.getThumbnailDimensions(isExpanded);
          const visualizer: any = await requestWorkerQuery("compile_visualizer", {
            source,
            spanStart: ann.spanStart,
            spanEnd: ann.spanEnd,
            kind: 'thumbnail',
            domain: 'thumb',
            sweepMax: 0
          });
          if (generation !== this.refreshGeneration) {
            return;
          }
          if (!visualizer?.ok || !(visualizer.drawWgsl || visualizer.wgsl)) {
            const reason = Array.isArray(visualizer?.diagnostics) && visualizer.diagnostics.length > 0
              ? String(visualizer.diagnostics[0]?.message || 'visualizer compile failed')
              : 'visualizer compile failed';
            if (!this.zones.has(key)) {
              this.upsertUnavailableZone(key, ann, semanticType, seriesDims.width, seriesDims.height, reason);
            }
            continue;
          }
          shaderWgsl = visualizer.drawWgsl || visualizer.wgsl;
          lastVariantWgsl = shaderWgsl;
          visualizerMeta = visualizer.metadata || null;
          visualizerMeta = visualizer.metadata || null;
          renderPath = 'thumbnail';
          dataUrl = await this.compiledVisualizerRenderer.renderShader({ drawWgsl: shaderWgsl, reduceWgsl: visualizer.reduceWgsl || "" }, {
            width: thumbDims.width,
            height: thumbDims.height,
            clearColor: [0.5, 0.5, 0.5, 1.0]
          });
          vizWidth = thumbDims.width;
          vizHeight = thumbDims.height;
        } else if (semanticType === 'color' || explicitKind === 'swatch') {
          const visualizer: any = await requestWorkerQuery("compile_visualizer", {
            source,
            spanStart: ann.spanStart,
            spanEnd: ann.spanEnd,
            kind: 'swatch',
            domain: 'time',
            sweepMax
          });
          if (generation !== this.refreshGeneration) {
            return;
          }
          if (!visualizer?.ok || !(visualizer.drawWgsl || visualizer.wgsl)) {
            const reason = Array.isArray(visualizer?.diagnostics) && visualizer.diagnostics.length > 0
              ? String(visualizer.diagnostics[0]?.message || 'visualizer compile failed')
              : 'visualizer compile failed';
            if (!this.zones.has(key)) {
              this.upsertUnavailableZone(key, ann, semanticType, seriesDims.width, seriesDims.height, reason);
            }
            continue;
          }
          shaderWgsl = visualizer.drawWgsl || visualizer.wgsl;
          lastVariantWgsl = shaderWgsl;
          visualizerMeta = visualizer.metadata || null;
          renderPath = 'swatch';
          dataUrl = await this.compiledVisualizerRenderer.renderShader({ drawWgsl: shaderWgsl, reduceWgsl: visualizer.reduceWgsl || "" }, {
            width: vizWidth,
            height: vizHeight,
            clearColor: [0.0, 0.0, 0.0, 1.0]
          });
        } else if (
          semanticType === 'scalar' ||
          semanticType === 'f32' ||
          semanticType === 'vec2' ||
          semanticType === 'vec2<f32>' ||
          explicitKind === 'timeseries' ||
          explicitKind === 'chart'
        ) {
          const visualizer: any = await requestWorkerQuery("compile_visualizer", {
            source,
            spanStart: ann.spanStart,
            spanEnd: ann.spanEnd,
            kind: 'sparkline',
            domain: ann.domain,
            sweepMax
          });
          if (generation !== this.refreshGeneration) {
            return;
          }
          if (!visualizer?.ok || !(visualizer.drawWgsl || visualizer.wgsl)) {
            const reason = Array.isArray(visualizer?.diagnostics) && visualizer.diagnostics.length > 0
              ? String(visualizer.diagnostics[0]?.message || 'visualizer compile failed')
              : 'visualizer compile failed';
            if (!this.zones.has(key)) {
              this.upsertUnavailableZone(key, ann, semanticType, seriesDims.width, seriesDims.height, reason);
            }
            continue;
          }
          shaderWgsl = visualizer.drawWgsl || visualizer.wgsl;
          lastVariantWgsl = shaderWgsl;
          visualizerMeta = visualizer.metadata || null;
          renderPath = 'sparkline';
          dataUrl = await this.compiledVisualizerRenderer.renderShader({ drawWgsl: shaderWgsl, reduceWgsl: visualizer.reduceWgsl || "" }, {
            width: vizWidth,
            height: vizHeight,
            clearColor: [0.04, 0.08, 0.12, 1.0]
          });
        } else {
          const thumbDims = this.getThumbnailDimensions(isExpanded);
          const visualizer: any = await requestWorkerQuery("compile_visualizer", {
            source,
            spanStart: ann.spanStart,
            spanEnd: ann.spanEnd,
            kind: 'thumbnail',
            domain: 'thumb',
            sweepMax: 0
          });
          if (generation !== this.refreshGeneration) {
            return;
          }
          if (!visualizer?.ok || !(visualizer.drawWgsl || visualizer.wgsl)) {
            const reason = Array.isArray(visualizer?.diagnostics) && visualizer.diagnostics.length > 0
              ? String(visualizer.diagnostics[0]?.message || 'visualizer compile failed')
              : 'visualizer compile failed';
            if (!this.zones.has(key)) {
              this.upsertUnavailableZone(key, ann, semanticType, seriesDims.width, seriesDims.height, reason);
            }
            continue;
          }
          shaderWgsl = visualizer.drawWgsl || visualizer.wgsl;
          lastVariantWgsl = shaderWgsl;
          visualizerMeta = visualizer.metadata || null;
          // Fallback to 2D thumbnail for shapes/layers
          renderPath = 'thumbnail';
          dataUrl = await this.compiledVisualizerRenderer.renderShader({ drawWgsl: shaderWgsl, reduceWgsl: visualizer.reduceWgsl || "" }, {
            width: thumbDims.width,
            height: thumbDims.height,
            clearColor: [0.5, 0.5, 0.5, 1.0]
          });
          vizWidth = thumbDims.width;
          vizHeight = thumbDims.height;
        }

        const shaderExplain = buildShaderExplainText(ann, semanticType, renderPath, sweepInfo, shaderWgsl, visualizerMeta);
        lastShaderExplain = shaderExplain;
        lastVisualizerMeta = visualizerMeta;
        
        if (!dataUrl) {
          this.upsertUnavailableZone(key, ann, semanticType, vizWidth, vizHeight, 'renderer returned no image');
          continue;
        }
        
        // Create or update zone
        if (this.zones.has(key)) {
          // Update existing zone's image, or replace placeholder with a rendered zone
          const zone = this.zones.get(key);
          zone.shaderWgsl = shaderWgsl;
          zone.shaderExplain = shaderExplain;
          zone.sweepInfo = sweepInfo;
          zone.visualizerMeta = visualizerMeta;
          const needsLayoutUpgrade = !zone.domNode?.querySelector('.viz-right-actions');
          if (zone.canvas && !needsLayoutUpgrade) {
            zone.canvas.src = dataUrl;
            this.updateFloatingWindowImage(key, dataUrl, ann.title || ann.name);
            this.scheduleZoneRelayout(key);
          } else {
            this.editor.changeViewZones((accessor) => {
              accessor.removeZone(zone.viewZoneId);
            });
            const pendingTimer = this.zoneRelayoutTimers.get(key);
            if (pendingTimer) {
              clearTimeout(pendingTimer);
              this.zoneRelayoutTimers.delete(key);
            }
            this.zones.delete(key);
            this.createZone(key, ann, dataUrl, semanticType, vizWidth, vizHeight, '', shaderWgsl, sweepInfo, shaderExplain, visualizerMeta);
          }
        } else {
          // Create new zone
          this.createZone(key, ann, dataUrl, semanticType, vizWidth, vizHeight, '', shaderWgsl, sweepInfo, shaderExplain, visualizerMeta);
        }
      } catch (err) {
        console.warn(`Failed to render viz for ${ann.name}:`, err);
        if (!this.zones.has(key)) {
          this.upsertUnavailableZone(
            key,
            ann,
            lastSemanticType,
            300,
            72,
            String(err?.message || err || 'render exception'),
            lastVariantWgsl,
            lastShaderExplain
          );
        }
      }
    }
  }

  buildParamFallbackViz(annotation) {
    const line = String(annotation?.lineText || '');
    const match = line.match(/^\s*param\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([^=\s]+)/);
    if (!match || !renderer || !Array.isArray(renderer.paramDefs)) {
      return null;
    }

    const paramName = match[1];
    const declaredType = normalizeTypeName(match[2]);
    const def = renderer.paramDefs.find((entry) => String(entry.name) === paramName) || {
      name: paramName,
      type: declaredType,
      min: null,
      max: null,
      default: 0
    };

    const seriesDims = this.getSeriesDimensions();
    const value = renderer.normalizeParamValue(def, renderer.paramValues.get(paramName));
    if (isColorLikeParamType(def.type)) {
      return {
        dataUrl: this.renderParamColorFallback(value, seriesDims.width, seriesDims.height),
        semanticType: 'color',
        width: seriesDims.width,
        height: seriesDims.height
      };
    }

    return {
      dataUrl: this.renderParamScalarFallback(value, def, seriesDims.width, seriesDims.height),
      semanticType: normalizeTypeName(def.type) || 'scalar',
      width: seriesDims.width,
      height: seriesDims.height
    };
  }

  renderParamScalarFallback(value, def, width = 300, height = 48) {
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      return null;
    }

    ctx.fillStyle = 'rgba(10, 20, 30, 1)';
    ctx.fillRect(0, 0, width, height);

    const t = normalizeTypeName(def?.type);
    const raw = Number(value);
    const numeric = Number.isFinite(raw) ? raw : 0;
    const min = Number.isFinite(def?.min) ? Number(def.min) : (t === 'bool' ? 0 : 0);
    const max = Number.isFinite(def?.max) ? Number(def.max) : (t === 'bool' ? 1 : 1);
    const span = Math.abs(max - min) > 1e-6 ? (max - min) : 1;
    const normalized = Math.max(0, Math.min(1, (numeric - min) / span));
    const y = Math.round((1 - normalized) * (height - 1));

    ctx.strokeStyle = 'rgba(45, 212, 191, 1)';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();

    ctx.fillStyle = 'rgba(154, 200, 222, 0.85)';
    ctx.font = '10px "IBM Plex Mono", monospace';
    ctx.fillText(`${def?.name || 'param'}=${Number.isFinite(numeric) ? numeric.toFixed(3) : String(value)}`, 6, 14);

    return canvas.toDataURL();
  }

  renderParamColorFallback(value, width = 300, height = 48) {
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      return null;
    }

    const rgba = Array.isArray(value) ? value : [1, 1, 1, 1];
    const r = Math.max(0, Math.min(255, Math.round((Number(rgba[0]) || 0) * 255)));
    const g = Math.max(0, Math.min(255, Math.round((Number(rgba[1]) || 0) * 255)));
    const b = Math.max(0, Math.min(255, Math.round((Number(rgba[2]) || 0) * 255)));
    const a = Math.max(0, Math.min(1, Number(rgba[3]) || 1));

    ctx.fillStyle = 'rgba(10, 20, 30, 1)';
    ctx.fillRect(0, 0, width, height);
    ctx.fillStyle = `rgba(${r}, ${g}, ${b}, ${a})`;
    ctx.fillRect(0, 0, width, height);

    return canvas.toDataURL();
  }

  upsertUnavailableZone(key, annotation, semanticType, width, height, reason, shaderWgsl = '', shaderExplain = '') {
    const message = String(reason || 'preview unavailable');
    if (!this.zones.has(key)) {
      this.createZone(key, annotation, null, semanticType, width, height, message, shaderWgsl, null, shaderExplain);
      return;
    }

    const zone = this.zones.get(key);
    if (!zone?.domNode) {
      return;
    }

    if (shaderWgsl) {
      zone.shaderWgsl = shaderWgsl;
    }
    if (shaderExplain) {
      zone.shaderExplain = shaderExplain;
    }

    const placeholder = zone.domNode.querySelector('.viz-placeholder');
    if (placeholder) {
      placeholder.textContent = message;
      placeholder.title = message;
    }

    const hasShaderButton = Boolean(zone.domNode.querySelector('.viz-right-actions .viz-action-btn'));
    if (shaderWgsl && !hasShaderButton) {
      this.editor.changeViewZones((accessor) => {
        accessor.removeZone(zone.viewZoneId);
      });
      this.zones.delete(key);
      this.createZone(key, annotation, null, semanticType, width, height, message, shaderWgsl, zone.sweepInfo || null, shaderExplain || zone.shaderExplain || '');
    }
  }

  getZoneIndentColumns(annotation) {
    const model = this.editor?.getModel?.();
    if (!model) {
      return 0;
    }

    const source = String(model.getValue?.() || "");
    const lines = source.split('\n');
    const lineNumber = Number(annotation?.directiveLineNumber || annotation?.lineNumber || 0);
    if (!Number.isFinite(lineNumber) || lineNumber < 1 || lineNumber > lines.length) {
      return 0;
    }

    const line = String(lines[lineNumber - 1] || "");
    const leadingWhitespace = line.match(/^[\t ]*/)?.[0] || "";
    if (!leadingWhitespace) {
      return 0;
    }

    const tabSize = 4;

    let columns = 0;
    for (const ch of leadingWhitespace) {
      columns += ch === '\t' ? tabSize : 1;
    }
    return columns;
  }

  estimateZoneHeight(annotation, semanticType, contentHeight) {
    const baseHeight = Math.max(48, Math.ceil(Number(contentHeight) || 0));
    const isChartLike = semanticType === 'f32'
      || semanticType === 'scalar'
      || semanticType === 'vec2'
      || semanticType === 'vec2<f32>'
      || semanticType === 'color';
    const axisFooter = isChartLike ? 20 : 0;
    const chrome = 24; // zone vertical padding + border + breathing room
    const metaReserve = 18; // label + metadata line
    const controlsReserve = Array.isArray(annotation?.controls) && annotation.controls.length > 0 ? 8 : 0;
    return baseHeight + axisFooter + chrome + metaReserve + controlsReserve;
  }

  isAnnotationVisible(annotation) {
    const lineNumber = Number(annotation?.lineNumber || 0);
    if (!Number.isFinite(lineNumber) || lineNumber < 1) {
      return false;
    }
    const ranges = this.editor?.getVisibleRanges?.() || [];
    for (const range of ranges) {
      if (lineNumber >= range.startLineNumber && lineNumber <= range.endLineNumber + 2) {
        return true;
      }
    }
    return false;
  }

  measureLiveZoneHeight(domNode) {
    if (!domNode) {
      return 0;
    }

    const computed = window.getComputedStyle(domNode);
    const paddingTop = parseFloat(computed.paddingTop || '0') || 0;
    const paddingBottom = parseFloat(computed.paddingBottom || '0') || 0;
    const borderTop = parseFloat(computed.borderTopWidth || '0') || 0;
    const borderBottom = parseFloat(computed.borderBottomWidth || '0') || 0;

    let visualHeight = 0;
    const visualNode = domNode.querySelector('.viz-placeholder, .viz-canvas');
    if (visualNode instanceof HTMLElement) {
      const wrapper = visualNode.parentElement;
      if (wrapper && wrapper !== domNode) {
        visualHeight = Math.max(visualHeight, wrapper.scrollHeight || wrapper.offsetHeight || 0);
      }
      visualHeight = Math.max(visualHeight, visualNode.scrollHeight || visualNode.offsetHeight || 0);
    }

    const infoNode = domNode.querySelector('.viz-info');
    const infoHeight = infoNode instanceof HTMLElement
      ? Math.max(infoNode.scrollHeight || 0, infoNode.offsetHeight || 0)
      : 0;

    const rowHeight = Math.max(visualHeight, infoHeight, 24);
    return Math.ceil(rowHeight + paddingTop + paddingBottom + borderTop + borderBottom);
  }

  measureZoneHeight(domNode, fallbackHeight = 140) {
    if (!domNode) {
      return Math.max(96, Math.ceil(Number(fallbackHeight) || 140));
    }

    const liveMeasured = this.measureLiveZoneHeight(domNode);
    if (liveMeasured > 0) {
      return Math.max(96, liveMeasured + 8);
    }
    return Math.max(96, Math.ceil(Number(fallbackHeight) || 140));
  }

  relayoutZoneToContent(key) {
    const zone = this.zones.get(key);
    if (!zone?.domNode || !zone?.viewZone || !zone?.viewZoneId) {
      return;
    }

    const estimated = this.estimateZoneHeight(zone.annotation, zone.semanticType, zone.canvas?.height || 96);
    const isVisible = this.isAnnotationVisible(zone.annotation);
    const measured = isVisible ? this.measureZoneHeight(zone.domNode, estimated) : 0;
    const nextHeight = measured > 0 ? Math.max(96, measured) : Math.max(96, estimated);
    const currentHeight = Number(zone.viewZone.heightInPx) || 0;
    if (Math.abs(nextHeight - currentHeight) < 2) {
      return;
    }

    zone.viewZone.heightInPx = nextHeight;
    this.editor.changeViewZones((accessor) => {
      accessor.layoutZone(zone.viewZoneId);
    });
  }

  scheduleZoneRelayout(key) {
    const existing = this.zoneRelayoutTimers.get(key);
    if (existing) {
      clearTimeout(existing);
    }

    const timerId = setTimeout(() => {
      this.zoneRelayoutTimers.delete(key);
      this.relayoutZoneToContent(key);
      requestAnimationFrame(() => {
        this.relayoutZoneToContent(key);
      });
    }, 45);

    this.zoneRelayoutTimers.set(key, timerId);
  }

  createZone(key, annotation, dataUrl, semanticType, width, height, statusText = "", shaderWgsl = "", sweepInfo = null, shaderExplain = "", visualizerMeta = null) {
    const domNode = document.createElement('div');
    domNode.className = 'viz-zone';
    domNode.contentEditable = 'false';
    domNode.setAttribute('role', 'group');
    domNode.setAttribute('aria-label', `Visualizer ${annotation.title || annotation.name}`);
    const indentColumns = Math.min(this.getZoneIndentColumns(annotation), 60);
    if (indentColumns > 0) {
      domNode.style.marginLeft = `${indentColumns}ch`;
      domNode.style.width = `calc(100% - ${indentColumns}ch)`;
    }
    
    // Create canvas image
    let canvas = null;
    if (dataUrl) {
      canvas = document.createElement('img');
      const isScalar = semanticType === 'f32' || semanticType === 'scalar' || semanticType === 'vec2' || semanticType === 'vec2<f32>';
      const isColor = semanticType === 'color';
      canvas.className = isScalar || isColor ? 'viz-canvas viz-canvas-chart' : 'viz-canvas';
      canvas.src = dataUrl;
      canvas.width = width;
      canvas.height = height;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      canvas.title = 'Click to expand inline. Double-click to pop out.';
      canvas.addEventListener('mousedown', (event) => {
        event.preventDefault();
        event.stopPropagation();
      });
      canvas.addEventListener('click', (event) => {
        event.preventDefault();
        event.stopPropagation();
        if (this.expandedZones.has(key)) {
          this.expandedZones.delete(key);
        } else {
          this.expandedZones.add(key);
        }
        this.refresh();
      });
      canvas.addEventListener('dblclick', (event) => {
        event.preventDefault();
        event.stopPropagation();
        this.openFloatingWindow(key, annotation, semanticType, canvas.src, width, height);
      });
      canvas.addEventListener('load', () => {
        this.scheduleZoneRelayout(key);
      });
      
      // For sparklines and color swatches, add axis overlay with optional time cursor
      if (isScalar || isColor) {
        const showTimeCursor = annotation.domain === 'time';
        const resolvedSweep = Number(sweepInfo?.value);
        const overlaySweep = Number.isFinite(resolvedSweep) && resolvedSweep > 0
          ? resolvedSweep
          : resolveAnnotationSweepMax(annotation);
        const wrapper = addAxisOverlay(domNode, canvas, annotation.domain, overlaySweep, showTimeCursor, visualizerMeta, renderer);
        domNode.appendChild(wrapper);
      } else {
        domNode.appendChild(canvas);
      }
    } else {
      const placeholder = document.createElement('div');
      placeholder.className = 'viz-placeholder';
      placeholder.style.width = `${width}px`;
      placeholder.style.height = `${height}px`;
      const message = statusText || 'visualizer pending';
      placeholder.textContent = message;
      placeholder.title = message;
      domNode.appendChild(placeholder);
    }
    
    const info = document.createElement('div');
    info.className = 'viz-info';

    const label = document.createElement('span');
    label.className = 'viz-label';
    label.textContent = annotation.title || annotation.name;
    info.appendChild(label);

    const meta = document.createElement('span');
    meta.className = 'viz-meta';
    const sweepMax = Number(sweepInfo?.value);
    const effectiveSweepMax = Number.isFinite(visualizerMeta?.sweepMax) && visualizerMeta?.sweepMax > 0
      ? Number(visualizerMeta.sweepMax)
      : Number.isFinite(sweepMax) && sweepMax > 0
      ? sweepMax
      : resolveAnnotationSweepMax(annotation);
    const displayDomain = visualizerMeta?.domain || annotation.domain;
    const domainLabel = displayDomain === 'time'
      ? `t=0..${effectiveSweepMax.toFixed(2).replace(/\.00$/, '')}s`
      : displayDomain === 'x'
        ? `x=0..${effectiveSweepMax.toFixed(2).replace(/\.00$/, '')}`
        : displayDomain === 'thumb' ? 'thumbnail' : '';
    const sampleLabel = displayDomain === 'time' || displayDomain === 'x'
      ? `samples=${width}`
      : '';
    const previewLabel = visualizerMeta?.previewLabel || '';
    const previewDetail = visualizerMeta?.previewDetail || '';
    const kindLabel = visualizerMeta?.kind
      ? `kind=${visualizerMeta.kind}`
      : annotation.kind ? `kind=${annotation.kind}` : `kind=${semanticType}`;
    const axisLabel = visualizerMeta?.yAxisLabel ? `y=${visualizerMeta.yAxisLabel}` : '';
    const controlsLabel = Array.isArray(annotation.controls) && annotation.controls.length > 0
      ? `controls=${annotation.controls.join(', ')}`
      : '';
    const sizeHint = displayDomain === 'thumb'
      ? (this.expandedZones.has(key) ? 'click=shrink, dblclick=popout' : 'click=expand, dblclick=popout')
      : '';
    meta.textContent = previewLabel
      ? [previewLabel, previewDetail, controlsLabel, sizeHint].filter(Boolean).join(' • ')
      : [kindLabel, domainLabel, sampleLabel, axisLabel, controlsLabel, sizeHint].filter(Boolean).join(' • ');
    info.appendChild(meta);

    const legendItems = [];
    if ((visualizerMeta?.kind || '') === 'thumbnail' && semanticType === 'space') {
      legendItems.push('before (left)', 'after (right)', 'warm = distortion');
    }
    if ((visualizerMeta?.kind || '') === 'thumbnail' && semanticType === 'shape') {
      legendItems.push('cyan = inside', 'white = edge band', 'hue = edge direction');
    }
    if (legendItems.length > 0) {
      const legend = document.createElement('div');
      legend.className = 'viz-legend';
      for (const item of legendItems) {
        const chip = document.createElement('span');
        chip.className = 'viz-legend-chip';
        chip.textContent = item;
        legend.appendChild(chip);
      }
      info.appendChild(legend);
    }

    domNode.appendChild(info);

    const rightActions = document.createElement('div');
    rightActions.className = 'viz-right-actions';
    rightActions.addEventListener('mousedown', (event) => {
      event.preventDefault();
      event.stopPropagation();
    });

    if (shaderWgsl) {
      const shaderBtn = document.createElement('button');
      shaderBtn.className = 'viz-action-btn';
      shaderBtn.type = 'button';
      shaderBtn.textContent = 'shader';
      shaderBtn.title = 'View generated WGSL for this visualizer';
      shaderBtn.addEventListener('mousedown', (event) => {
        event.preventDefault();
        event.stopPropagation();
      });
      shaderBtn.addEventListener('click', () => {
        const zone = this.zones.get(key);
        const currentShader = String(zone?.shaderWgsl || shaderWgsl || '');
        const explain = String(zone?.shaderExplain || shaderExplain || '');
        if (!currentShader) {
          return;
        }
        this.showShaderDialog(annotation.title || annotation.name, currentShader, explain);
      });
      rightActions.appendChild(shaderBtn);
    }

    const closeBtn = document.createElement('button');
    closeBtn.className = 'viz-close';
    closeBtn.title = 'Remove visualizer annotation';
    closeBtn.textContent = '×';
    closeBtn.addEventListener('mousedown', (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    closeBtn.addEventListener('click', () => this.removeAnnotation(annotation));
    rightActions.appendChild(closeBtn);

    domNode.appendChild(rightActions);
    
    const fallbackZoneHeight = this.estimateZoneHeight(annotation, semanticType, height);
    const zoneHeightPx = this.measureZoneHeight(domNode, fallbackZoneHeight);

    let viewZoneId;
    const viewZone = {
      afterLineNumber: annotation.lineNumber,
      heightInPx: zoneHeightPx,
      domNode,
      suppressMouseDown: true
    };
    this.editor.changeViewZones((accessor) => {
      viewZoneId = accessor.addZone(viewZone);
    });
    
    this.zones.set(key, {
      viewZoneId,
      viewZone,
      annotation,
      semanticType,
      dataUrl,
      canvas,
      domNode,
      shaderWgsl,
      sweepInfo,
      shaderExplain,
      visualizerMeta
    });

    this.scheduleZoneRelayout(key);
  }

  ensureShaderDialog() {
    if (this.shaderDialog) {
      return this.shaderDialog;
    }

    const backdrop = document.createElement('div');
    backdrop.className = 'viz-shader-backdrop';

    const panel = document.createElement('div');
    panel.className = 'viz-shader-panel';

    const header = document.createElement('div');
    header.className = 'viz-shader-header';

    const title = document.createElement('div');
    title.className = 'viz-shader-title';
    title.textContent = 'Generated WGSL';

    const controls = document.createElement('div');
    controls.className = 'viz-shader-controls';

    const copyBtn = document.createElement('button');
    copyBtn.className = 'viz-action-btn';
    copyBtn.type = 'button';
    copyBtn.textContent = 'copy';

    const closeBtn = document.createElement('button');
    closeBtn.className = 'viz-action-btn';
    closeBtn.type = 'button';
    closeBtn.textContent = 'close';

    controls.appendChild(copyBtn);
    controls.appendChild(closeBtn);
    header.appendChild(title);
    header.appendChild(controls);

    const pre = document.createElement('pre');
    pre.className = 'viz-shader-pre';

    const explain = document.createElement('pre');
    explain.className = 'viz-shader-explain';

    panel.appendChild(header);
    panel.appendChild(explain);
    panel.appendChild(pre);
    backdrop.appendChild(panel);

    const close = () => {
      backdrop.style.display = 'none';
    };

    backdrop.addEventListener('click', (event) => {
      if (event.target === backdrop) {
        close();
      }
    });
    closeBtn.addEventListener('click', close);

    copyBtn.addEventListener('click', async () => {
      const text = pre.textContent || '';
      if (!text) {
        return;
      }
      try {
        await navigator.clipboard.writeText(text);
        copyBtn.textContent = 'copied';
      } catch {
        copyBtn.textContent = 'copy failed';
      }
      setTimeout(() => {
        copyBtn.textContent = 'copy';
      }, 1200);
    });

    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && backdrop.style.display !== 'none') {
        close();
      }
    });

    document.body.appendChild(backdrop);
    this.shaderDialog = { backdrop, title, pre, explain };
    return this.shaderDialog;
  }

  showShaderDialog(name, wgsl, explainText = '') {
    const dialog = this.ensureShaderDialog();
    dialog.title.textContent = `Generated WGSL: ${name || 'visualizer'}`;
    dialog.explain.textContent = String(explainText || 'No provenance available.');
    dialog.pre.textContent = String(wgsl || '');
    dialog.backdrop.style.display = 'flex';
  }

  closeFloatingWindow(key) {
    const entry = this.floatingWindows.get(key);
    if (!entry) {
      return;
    }
    entry.root.remove();
    this.floatingWindows.delete(key);
  }

  updateFloatingWindowImage(key, dataUrl, titleText = '') {
    const entry = this.floatingWindows.get(key);
    if (!entry) {
      return;
    }
    if (entry.img && dataUrl) {
      entry.img.src = dataUrl;
    }
    if (entry.title && titleText) {
      entry.title.textContent = titleText;
    }
  }

  openFloatingWindow(key, annotation, semanticType, dataUrl, width, height) {
    if (!dataUrl) {
      return;
    }

    const existing = this.floatingWindows.get(key);
    if (existing) {
      existing.root.style.display = 'block';
      existing.root.style.zIndex = '10010';
      existing.img.src = dataUrl;
      return;
    }

    const root = document.createElement('div');
    root.className = 'viz-float-window';
    const cascade = this.nextFloatCascade % 9;
    this.nextFloatCascade += 1;
    root.style.left = `${48 + cascade * 20}px`;
    root.style.top = `${120 + cascade * 16}px`;

    const header = document.createElement('div');
    header.className = 'viz-float-header';

    const title = document.createElement('div');
    title.className = 'viz-float-title';
    title.textContent = annotation.title || annotation.name || 'visualizer';

    const meta = document.createElement('div');
    meta.className = 'viz-float-meta';
    meta.textContent = `${semanticType || 'unknown'} • ${annotation.domain || 'thumb'}`;

    const controls = document.createElement('div');
    controls.className = 'viz-float-controls';

    const closeBtn = document.createElement('button');
    closeBtn.className = 'viz-action-btn';
    closeBtn.type = 'button';
    closeBtn.textContent = 'close';
    closeBtn.title = 'Close floating chart window';
    closeBtn.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      this.closeFloatingWindow(key);
    });

    controls.appendChild(closeBtn);
    header.appendChild(title);
    header.appendChild(meta);
    header.appendChild(controls);

    const body = document.createElement('div');
    body.className = 'viz-float-body';

    const img = document.createElement('img');
    img.className = 'viz-float-image';
    img.src = dataUrl;
    img.alt = `${annotation.title || annotation.name || 'visualizer'} preview`;
    img.draggable = false;
    img.style.width = '100%';
    img.style.height = '100%';
    img.style.objectFit = 'contain';

    body.appendChild(img);
    root.appendChild(header);
    root.appendChild(body);

    const initialWidth = Math.max(320, Number(width) || 320);
    const initialHeight = Math.max(220, Number(height) || 220);
    root.style.width = `${Math.min(980, initialWidth + 160)}px`;
    root.style.height = `${Math.min(700, initialHeight + 120)}px`;

    let dragging = false;
    let dragStartX = 0;
    let dragStartY = 0;
    let baseLeft = 0;
    let baseTop = 0;

    const onMove = (event) => {
      if (!dragging) {
        return;
      }
      const dx = event.clientX - dragStartX;
      const dy = event.clientY - dragStartY;
      root.style.left = `${Math.max(8, baseLeft + dx)}px`;
      root.style.top = `${Math.max(8, baseTop + dy)}px`;
    };

    const onUp = () => {
      dragging = false;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      root.classList.remove('dragging');
    };

    header.addEventListener('pointerdown', (event) => {
      if (event.target instanceof HTMLElement && event.target.closest('button')) {
        return;
      }
      dragging = true;
      dragStartX = event.clientX;
      dragStartY = event.clientY;
      baseLeft = Number.parseFloat(root.style.left) || 0;
      baseTop = Number.parseFloat(root.style.top) || 0;
      root.classList.add('dragging');
      window.addEventListener('pointermove', onMove);
      window.addEventListener('pointerup', onUp);
    });

    document.body.appendChild(root);
    this.floatingWindows.set(key, { root, img, title });
  }

  removeAnnotation(annotation) {
    const model = this.editor.getModel();
    if (!model) return;

    if (annotation.directivePlacement === 'leading' || annotation.directivePlacement === 'below') {
      const directiveLine = annotation.directiveLineNumber;
      const lineContent = model.getLineContent(directiveLine);
      model.pushEditOperations(
        [],
        [{
          range: {
            startLineNumber: directiveLine,
            startColumn: 1,
            endLineNumber: directiveLine,
            endColumn: lineContent.length + 1
          },
          text: ''
        }],
        () => null
      );
      return;
    }

    const lineContent = model.getLineContent(annotation.lineNumber);
    const newLine = removeVizDirectiveFromLine(lineContent);
    if (newLine === null) {
      return;
    }

    model.pushEditOperations(
      [],
      [{
        range: {
          startLineNumber: annotation.lineNumber,
          startColumn: 1,
          endLineNumber: annotation.lineNumber,
          endColumn: lineContent.length + 1
        },
        text: newLine
      }],
      () => null
    );
  }

  scheduleRefresh() {
    if (this.updateTimer) {
      clearTimeout(this.updateTimer);
    }
    this.updateTimer = setTimeout(() => {
      void this.refresh();
    }, 250);
  }

  dispose() {
    // Remove all zones
    this.editor.changeViewZones((accessor) => {
      for (const zone of this.zones.values()) {
        accessor.removeZone(zone.viewZoneId);
      }
    });
    this.zones.clear();
    for (const timerId of this.zoneRelayoutTimers.values()) {
      clearTimeout(timerId);
    }
    this.zoneRelayoutTimers.clear();
    this.scrollListener?.dispose?.();
    this.layoutListener?.dispose?.();
    for (const key of this.floatingWindows.keys()) {
      this.closeFloatingWindow(key);
    }
  }
}
