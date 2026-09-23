import { createHash } from "node:crypto";

export function digest(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function inputDigest(dependencies, settings) {
  const hash = createHash("sha256");
  for (const [name, content] of [...dependencies].sort(([a], [b]) => a.localeCompare(b))) {
    // Length framing avoids ambiguous concatenation; LF avoids checkout-only drift.
    const bytes = Buffer.isBuffer(content) ? content : Buffer.from(content.replaceAll("\r\n", "\n"));
    hash.update(JSON.stringify([name, bytes.length]));
    hash.update(bytes);
  }
  hash.update(JSON.stringify(settings));
  return hash.digest("hex");
}

export function isCurrent(record, inputHash, bytes) {
  return record?.inputSha256 === inputHash && bytes != null && record.outputSha256 === digest(bytes);
}

export function embedPreviews(markdown, samples) {
  const byId = new Map(samples.map(sample => [sample.id, sample]));
  const seen = new Set();
  // Replace existing generated previews as well as inserting missing ones.
  const pattern = /(<!-- readme:sample ([^\n]+) -->\n[\s\S]*?<!-- readme:end -->)(?:\n\n<!-- readme:preview -->\n[\s\S]*?<!-- readme:preview-end -->)?/g;
  const result = markdown.replace(pattern, (whole, block, id) => {
    const sample = byId.get(id);
    if (!sample) throw new Error(`Unknown README sample: ${id}`);
    if (seen.has(id)) throw new Error(`Duplicate README sample: ${id}`);
    seen.add(id);
    if (sample.preview === false) return block;
    return `${block}\n\n<!-- readme:preview -->\n![${sample.alt}](docs/readme/media/${sample.id}.webp)\n<!-- readme:preview-end -->`;
  });
  for (const sample of samples) {
    if (!seen.has(sample.id)) throw new Error(`Sample is not embedded in README: ${sample.id}`);
  }
  return result;
}
