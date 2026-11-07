use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Fixed namespace for every deterministic Eddy identifier. Changing it would
/// break the mapping between standalone data and server data.
pub const NAMESPACE: Uuid = Uuid::from_u128(0x5f0c_8e1a_93b2_4d6e_a7c1_2e9d_4b60_1f37);

fn v5(input: &str) -> String {
    Uuid::new_v5(&NAMESPACE, input.as_bytes()).to_string()
}

/// Upstream account identity, e.g. a FreshRSS user on a normalized API base URL.
pub fn account_id(kind: &str, base_url: &str, username: &str) -> String {
    v5(&format!("account\u{1f}{kind}\u{1f}{base_url}\u{1f}{username}"))
}

pub fn feed_source_id(account_id: &str, upstream_feed_id: &str) -> String {
    v5(&format!("source\u{1f}{account_id}\u{1f}{upstream_feed_id}"))
}

pub fn feed_item_id(account_id: &str, upstream_item_id: &str) -> String {
    v5(&format!("item\u{1f}{account_id}\u{1f}{upstream_item_id}"))
}

/// The built-in source that holds pages saved by the user.
pub fn saved_source_id() -> String {
    v5("source\u{1f}local\u{1f}saved")
}

/// Time-ordered random identifier for locally created entities.
pub fn new_local_id() -> String {
    Uuid::now_v7().to_string()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_ids_are_stable_and_distinct() {
        let account = account_id("greader", "https://rss.example.com/api/greader.php", "me");
        assert_eq!(account, account_id("greader", "https://rss.example.com/api/greader.php", "me"));
        assert_ne!(account, account_id("greader", "https://rss.example.com/api/greader.php", "you"));
        let item = feed_item_id(&account, "000000000000abcd");
        assert_eq!(item, feed_item_id(&account, "000000000000abcd"));
        assert_ne!(item, feed_source_id(&account, "000000000000abcd"));
    }
}
