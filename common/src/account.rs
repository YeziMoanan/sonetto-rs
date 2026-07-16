/// Parse the account identifier shape observed in CN 3.8 SDK requests.
///
/// The client-provided value is a positive decimal user ID. Channel prefixes
/// are not accepted because the current APK and request evidence does not
/// establish a prefix contract.
pub fn parse_user_id(account_id: &str) -> Option<i64> {
    if account_id.is_empty() || !account_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    account_id.parse().ok().filter(|user_id| *user_id > 0)
}
