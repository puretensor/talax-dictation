// SQLite CURRENT_TIMESTAMP is UTC text "YYYY-MM-DD HH:MM:SS".
// A date-time with a space is not the ECMAScript interchange format, and
// Date.parse falls back to a local-time reading on engines that accept it.
const SQLITE_UTC_TIMESTAMP =
  /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2})(\.\d+)?$/;

export function parseSessionTimestamp(ts: string): Date | null {
  const sqlite = SQLITE_UTC_TIMESTAMP.exec(ts);
  const parsed = sqlite
    ? new Date(`${sqlite[1]}T${sqlite[2]}${sqlite[3] ?? ""}Z`)
    : new Date(ts);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

export function formatSessionTimestamp(ts: string): string {
  const parsed = parseSessionTimestamp(ts);
  return parsed ? parsed.toLocaleString() : ts;
}
