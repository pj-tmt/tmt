//! Office pairing: the pairing protocol and records, the HTTP transport to the
//! paired Office service, deployment targets and the OS credential vault.
//! It reaches core only through `tmt-adapters`' public modules.

pub mod office_deployment;
mod office_http;
pub mod office_pairing;

#[cfg(test)]
mod test_support;
