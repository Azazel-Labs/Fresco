type DocsIndexEntry = {
  title: string;
  id: string;
  level: number;
};

type DocsOverlay = {
  backdrop: HTMLDivElement;
  frame: HTMLIFrameElement;
  searchInput: HTMLInputElement;
  goBtn: HTMLButtonElement;
  statusEl: HTMLSpanElement;
  docsLoaded: boolean;
  index: DocsIndexEntry[];
  activeAnchor: string;
  bestMatchId: string;
  matchCount: number;
  close: () => void;
};

export type DocsOverlayController = {
  show: () => void;
  toggle: () => void;
};

export function createDocsOverlayController(
  docsToggleEl: HTMLElement | null,
  docsPageUrl: string
): DocsOverlayController {
  let docsOverlay: DocsOverlay | null = null;

  function slugifyHeading(text: string): string {
    const base = String(text || "")
      .toLowerCase()
      .trim()
      .replace(/[^a-z0-9\s-]/g, "")
      .replace(/\s+/g, "-")
      .replace(/-+/g, "-");
    return base || "section";
  }

  function normalizeSearchText(text: string): string {
    return String(text || "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, " ")
      .trim();
  }

  function buildHeadingIndexFromHtml(html: string): DocsIndexEntry[] {
    const parser = new DOMParser();
    const doc = parser.parseFromString(String(html || ""), "text/html");
    const headings = Array.from(doc.querySelectorAll("h1, h2, h3"));
    const seen = new Map<string, number>();
    const out: DocsIndexEntry[] = [];

    for (const heading of headings) {
      const title = String(heading.textContent || "").trim();
      if (!title) {
        continue;
      }
      const level = Number(heading.tagName.slice(1)) || 2;
      const base = slugifyHeading(title);
      const count = (seen.get(base) || 0) + 1;
      seen.set(base, count);
      const id = count === 1 ? base : `${base}-${count}`;
      out.push({ title, id, level });
    }

    // Index schema metadata line so users can find the exact docs schema quickly.
    const schemaCode = doc.querySelector("p code");
    const schemaText = String(schemaCode?.textContent || "").trim();
    if (schemaText) {
      out.push({
        title: `Schema: ${schemaText}`,
        id: out[0]?.id || "fresco-language-api-reference",
        level: 2,
      });
    }

    return out;
  }

  function applyHeadingIdsToFrameDoc(frame: HTMLIFrameElement): void {
    let doc: Document | null = null;
    try {
      doc = frame.contentDocument;
    } catch {
      doc = null;
    }
    if (!doc) {
      return;
    }

    const headings = Array.from(doc.querySelectorAll("h1, h2, h3"));
    const seen = new Map<string, number>();
    for (const heading of headings) {
      const title = String(heading.textContent || "").trim();
      if (!title) {
        continue;
      }
      const base = slugifyHeading(title);
      const count = (seen.get(base) || 0) + 1;
      seen.set(base, count);
      const id = count === 1 ? base : `${base}-${count}`;
      heading.id = id;
    }
  }

  function rankMatch(title: string, query: string): number {
    const tRaw = String(title || "");
    const qRaw = String(query || "").trim();
    if (!qRaw) {
      return 0;
    }

    const t = normalizeSearchText(tRaw);
    const q = normalizeSearchText(qRaw);
    if (!q) {
      return 0;
    }

    const terms = q.split(/\s+/).filter(Boolean);
    if (terms.length === 0) {
      return 0;
    }
    if (!terms.every((term) => t.includes(term))) {
      return -1;
    }

    if (t === q) {
      return 1000;
    }
    if (t.startsWith(q)) {
      return 700;
    }

    let score = 300;
    for (const term of terms) {
      const idx = t.indexOf(term);
      score += Math.max(0, 60 - idx);
    }
    return score;
  }

  function navigateFrameToAnchor(overlay: DocsOverlay, anchorId: string): void {
    overlay.activeAnchor = String(anchorId || "").trim();
    if (!overlay.activeAnchor || !overlay.docsLoaded) {
      return;
    }

    try {
      const win = overlay.frame.contentWindow;
      const doc = overlay.frame.contentDocument;
      const el = doc?.getElementById(overlay.activeAnchor);
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "start" });
      }
      if (win) {
        win.location.hash = overlay.activeAnchor;
      }
    } catch {
      overlay.frame.src = `${docsPageUrl}#${encodeURIComponent(overlay.activeAnchor)}`;
    }
  }

  function renderSearchResults(overlay: DocsOverlay): void {
    const query = String(overlay.searchInput.value || "").trim().toLowerCase();
    const ranked = overlay.index
      .map((entry) => ({ entry, score: rankMatch(entry.title, query) }))
      .filter(({ score }) => (query ? score >= 0 : true))
      .sort(
        (a, b) =>
          b.score - a.score
          || a.entry.title.localeCompare(b.entry.title, undefined, { sensitivity: "base" })
      )
      .slice(0, 40)
      .map(({ entry }) => entry);

    overlay.bestMatchId = ranked[0]?.id || "";
    overlay.matchCount = ranked.length;
    overlay.goBtn.disabled = !overlay.bestMatchId;

    const total = query ? ranked.length : overlay.index.length;
    if (!overlay.index.length) {
      overlay.statusEl.textContent = "Docs index unavailable";
    } else if (query) {
      const topTitle = ranked[0]?.title ? ` - top: ${ranked[0].title}` : "";
      overlay.statusEl.textContent = `${total} match${total === 1 ? "" : "es"}${topTitle}`;
    } else {
      overlay.statusEl.textContent = `${total} sections`;
    }
  }

  function jumpToBestMatch(overlay: DocsOverlay): void {
    if (!overlay.bestMatchId) {
      return;
    }
    navigateFrameToAnchor(overlay, overlay.bestMatchId);
  }

  async function ensureDocsIndex(overlay: DocsOverlay): Promise<void> {
    if (overlay.index.length > 0) {
      return;
    }

    try {
      const response = await fetch(docsPageUrl, { credentials: "same-origin" });
      if (!response.ok) {
        throw new Error(`docs fetch failed: ${response.status}`);
      }
      const html = await response.text();
      overlay.index = buildHeadingIndexFromHtml(html);
      renderSearchResults(overlay);
    } catch {
      overlay.index = [];
      renderSearchResults(overlay);
    }
  }

  function ensureDocsOverlay(): DocsOverlay {
    if (docsOverlay) {
      return docsOverlay;
    }

    const backdrop = document.createElement("div");
    backdrop.className = "docs-backdrop";
    backdrop.setAttribute("role", "dialog");
    backdrop.setAttribute("aria-modal", "true");

    const panel = document.createElement("div");
    panel.className = "docs-panel";

    const header = document.createElement("div");
    header.className = "docs-header";

    const title = document.createElement("div");
    title.className = "docs-title";
    title.textContent = "Fresco API Reference";

    const controls = document.createElement("div");
    controls.className = "docs-controls";

    const openBtn = document.createElement("button");
    openBtn.className = "docs-action-btn";
    openBtn.type = "button";
    openBtn.textContent = "open tab";

    const closeBtn = document.createElement("button");
    closeBtn.className = "docs-action-btn";
    closeBtn.type = "button";
    closeBtn.textContent = "close";

    const searchWrap = document.createElement("div");
    searchWrap.className = "docs-search";

    const searchInput = document.createElement("input");
    searchInput.type = "search";
    searchInput.className = "docs-search-input";
    searchInput.placeholder = "Search docs sections...";
    searchInput.setAttribute("aria-label", "Search docs sections");

    const statusEl = document.createElement("span");
    statusEl.className = "docs-search-status";
    statusEl.textContent = "Loading index...";

    const goBtn = document.createElement("button");
    goBtn.type = "button";
    goBtn.className = "docs-action-btn docs-go-btn";
    goBtn.textContent = "Go";
    goBtn.disabled = true;

    const frame = document.createElement("iframe");
    frame.className = "docs-frame";
    frame.loading = "lazy";
    frame.referrerPolicy = "no-referrer";

    const close = () => {
      backdrop.style.display = "none";
      docsToggleEl?.setAttribute("aria-expanded", "false");
    };

    docsOverlay = {
      backdrop,
      frame,
      searchInput,
      goBtn,
      statusEl,
      docsLoaded: false,
      index: [],
      activeAnchor: "",
      bestMatchId: "",
      matchCount: 0,
      close,
    };

    openBtn.addEventListener("click", () => {
      window.open(docsPageUrl, "_blank", "noopener,noreferrer");
    });
    closeBtn.addEventListener("click", close);

    searchInput.addEventListener("input", () => {
      if (docsOverlay) {
        renderSearchResults(docsOverlay);
      }
    });

    searchInput.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") {
        return;
      }
      event.preventDefault();
      if (docsOverlay) {
        jumpToBestMatch(docsOverlay);
      }
    });

    goBtn.addEventListener("click", () => {
      if (docsOverlay) {
        jumpToBestMatch(docsOverlay);
      }
    });

    backdrop.addEventListener("click", (event) => {
      if (event.target === backdrop) {
        close();
      }
    });

    frame.addEventListener("load", () => {
      if (!docsOverlay) {
        return;
      }
      docsOverlay.docsLoaded = true;
      applyHeadingIdsToFrameDoc(frame);
      if (docsOverlay.activeAnchor) {
        navigateFrameToAnchor(docsOverlay, docsOverlay.activeAnchor);
      }
    });

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && backdrop.style.display !== "none") {
        close();
      }
    });

    controls.appendChild(openBtn);
    controls.appendChild(closeBtn);
    header.appendChild(title);
    header.appendChild(controls);

    searchWrap.appendChild(searchInput);
    searchWrap.appendChild(statusEl);
    searchWrap.appendChild(goBtn);

    panel.appendChild(header);
    panel.appendChild(searchWrap);
    panel.appendChild(frame);

    backdrop.appendChild(panel);
    document.body.appendChild(backdrop);

    void ensureDocsIndex(docsOverlay);
    renderSearchResults(docsOverlay);
    return docsOverlay;
  }

  function show(): void {
    const overlay = ensureDocsOverlay();
    if (!overlay.frame.src) {
      overlay.frame.src = docsPageUrl;
    }
    overlay.docsLoaded = false;
    overlay.backdrop.style.display = "flex";
    docsToggleEl?.setAttribute("aria-expanded", "true");
    overlay.searchInput.focus();
    void ensureDocsIndex(overlay);
    renderSearchResults(overlay);
  }

  function toggle(): void {
    const overlay = ensureDocsOverlay();
    if (overlay.backdrop.style.display === "flex") {
      overlay.close();
      return;
    }
    show();
  }

  return { show, toggle };
}
