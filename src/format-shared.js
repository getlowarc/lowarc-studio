// A single small opt-in utility: compact number formatting ("1.4K", "10K", "1M") for anywhere in
// the IDE — or a plugin's own sandboxed iframe — that has numeric text to keep short. Not a hand-
// rolled formatter: Intl.NumberFormat's own compact notation already does exactly this, verified
// live (1400 -> "1.4K", 10000 -> "10K", 1000000 -> "1M", 999 -> "999", uppercase K/M/B/T for
// en-US), so this file is just a thin, memoized wrapper rather than reimplementing the convention.
//
// Shared the same dual way dropdown-shared.js already is: a real file under src/ a host page can
// `<script src="format-shared.js">` directly, and also vendored into plugin_assets.rs
// (SHARED_FORMAT_JS) so any plugin's iframe can opt in via `<script src="__lowarc-format.js">`.
const lowarcCompactNumberFormatter = new Intl.NumberFormat(undefined, { notation: "compact", compactDisplay: "short" });

function lowarcFormatCompact(n) {
  return lowarcCompactNumberFormatter.format(n);
}
