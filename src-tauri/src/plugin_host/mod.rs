// Plugins are invoked per call, not spawned and kept alive — see protocol.rs for the wire
// protocol and why. Nothing here needs to scan/start anything at launch any more; installs.rs
// already does the "what's on disk" scan for the Plugins manage page, and protocol::invoke spawns
// fresh on demand, so this module is just the re-export point.

pub mod protocol;
