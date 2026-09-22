//! API layer: provider abstraction plus the Wikimedia Commons implementation.

pub mod provider;
pub mod rate_limit;
pub mod wikimedia;

pub use provider::AssetProvider;
pub use wikimedia::WikimediaClient;
