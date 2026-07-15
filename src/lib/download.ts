// Trigger a browser file download for in-memory content — the create-Blob → object-URL → anchor.click
// → revoke dance, shared so it can't drift (was inlined in AnalyticsTab CSV export + TerminalPane log).
export function downloadBlob(filename: string, content: BlobPart, mime: string): void {
  const url = URL.createObjectURL(new Blob([content], { type: mime }));
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}
