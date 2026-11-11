/**
 * Folders that only hold cache: a program rebuilds them, and they are full of files that are locked,
 * half-written or cloud placeholders, so they fail in backups for no benefit. Matched by name, at any
 * depth (like a folder rule without a path).
 */
export const CACHE_FOLDERS = [".dropbox.cache", ".cache", "__pycache__", "Cache", "Code Cache", "GPUCache"];

/** The cache folder a path is in (or is), as named in `CACHE_FOLDERS`, or null. */
export function cacheFolderIn(path: string): string | null {
  const parts = path.toLowerCase().split("\\").filter(Boolean);
  return CACHE_FOLDERS.find((f) => parts.includes(f.toLowerCase())) ?? null;
}
