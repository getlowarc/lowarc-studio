// The one place a version is written down.
//
// The number itself lives in Cargo.toml and reaches everything else through CARGO_PKG_VERSION:
// tauri.conf.json has no `version` key, so the bundler, the updater manifest and
// window.__TAURI__.app.getVersion() all take Cargo's. Changing Cargo.toml is the whole release
// edit. What is added here is the name, which semver has no field for.
//
// Numbering, for whoever bumps it next:
//   major  structure and architecture changed. Reset minor and patch. Pinned at 0 until 1.0, so
//          for now architecture lands in minor with everything else.
//   minor  features added or removed. Reset patch.
//   patch  fixes, and anything too small to be a feature.

/// Name of each major version. Majors get names; minors and patches do not.
const NAMES: &[(u32, &str)] = &[(0, "Daedalus")];

pub const NUMBER: &str = env!("CARGO_PKG_VERSION");

/// The name for the major this build is on, or an empty string for a major nobody has named yet.
pub fn name() -> &'static str {
    let major: u32 = NUMBER.split('.').next().unwrap_or("").parse().unwrap_or(u32::MAX);
    NAMES.iter().find(|(m, _)| *m == major).map(|(_, n)| *n).unwrap_or("")
}

/// "0.58.6 Daedalus". What the About box, the logo tooltip and `lowarc --version` all show, so
/// there is never a build whose three answers disagree.
pub fn label() -> String {
    match name() {
        "" => NUMBER.to_string(),
        n => format!("{NUMBER} {n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_carries_the_name_of_the_current_major() {
        // Guards the parse in name(): a Cargo.toml version this can't read silently drops the name
        // from every surface at once, and nothing else would notice.
        assert!(!name().is_empty(), "no name for major in {NUMBER}; add one to NAMES");
        assert_eq!(label(), format!("{NUMBER} {}", name()));
    }
}
