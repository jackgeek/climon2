//! Test-only fixture, never shipped (#97): the client library of the
//! [second-release fixture](../../../GLOSSARY.md#testing), an independently
//! packaged release. It compiles `climon2-client`'s library
//! source unchanged, so it exports only `climon2_client_get_interface` and
//! implements native revision 1 like any release, but its release identity
//! is this package's version, not the workspace's. It is built without the
//! `test-hooks` faults: a fault injected into the process reaches only the
//! hooked release under test, which exposes any call that crossed releases.

#[path = "../../climon2-client/src"]
mod client {
    pub mod library;
}
