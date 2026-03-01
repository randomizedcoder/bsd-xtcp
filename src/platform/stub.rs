use super::CollectError;
use super::CollectionResult;

/// Stub collector for platforms that don't support pcblist_n.
pub fn collect() -> Result<CollectionResult, CollectError> {
    Err(CollectError::UnsupportedPlatform)
}
